//! The authentication use cases: login, refresh, logout, session resolution.

use std::sync::Arc;

use chrono::Utc;
use pmk_domain::access::{effective_permissions, Permission};
use pmk_domain::tenant::AuthenticatedPrincipal;
use pmk_domain::User;
use pmk_ports::repository::{
    BillingRepository, Entitlement, RefreshTokenRepository, UserRepository,
};

use super::password;
use super::token::{hash_refresh, AccessClaims, RefreshToken, TokenCodec};
use crate::{AppError, AppResult};
use pmk_domain::identity::recovery::{
    may_reset_by_email, validate_reset, ResetCandidates, INVALID_TOKEN_MESSAGE,
    RESET_LINK_TTL_MINUTES,
};
use pmk_domain::ids::UserId;
use pmk_domain::DomainError;
use pmk_ports::repository::{BootstrapRepository, CompanyRegistration};
use pmk_ports::Clock;

/// Everything the HTTP layer needs about the caller, resolved once per request.
#[derive(Debug, Clone)]
pub struct SessionUser {
    pub user: User,
    pub principal: AuthenticatedPrincipal,
    pub permissions: Vec<Permission>,
    pub entitlement: Entitlement,
}

impl SessionUser {
    /// Effective permissions after the `access-rules:managed` sentinel is
    /// applied (domain-rules R5).
    #[must_use]
    pub fn effective(&self) -> Vec<Permission> {
        effective_permissions(self.user.role, &self.permissions)
            .into_iter()
            .collect()
    }

    #[must_use]
    pub fn can(&self, resource: &str, action: &str) -> bool {
        pmk_domain::access::has_permission(self.user.role, &self.permissions, resource, action)
    }
}

#[derive(Debug, Clone)]
pub struct LoginOutcome {
    pub access_token: String,
    pub access_claims: AccessClaims,
    pub refresh_token: String,
    pub user: User,
}

/// What the session-less paths need: first-run setup, registration and
/// password recovery.
///
/// Grouped because they travel together and none of them is used by the
/// ordinary login path.
pub struct BootstrapDeps {
    pub repository: Arc<dyn BootstrapRepository>,
    pub clock: Arc<dyn Clock>,
    // `setup_secret` below is why this type has a hand-written Debug: it must
    // never reach a log line.
    /// The shared secret first-run setup requires. Empty disables setup
    /// entirely, which is what a deployment past its first run wants.
    pub setup_secret: String,
}

impl std::fmt::Debug for BootstrapDeps {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        // The secret is deliberately absent, and reported only as present or
        // not: printing it would put it in whatever captured the log.
        f.debug_struct("BootstrapDeps")
            .field("setup_enabled", &!self.setup_secret.is_empty())
            .finish_non_exhaustive()
    }
}

pub struct AuthService {
    users: Arc<dyn UserRepository>,
    refresh: Arc<dyn RefreshTokenRepository>,
    billing: Arc<dyn BillingRepository>,
    codec: TokenCodec,
    refresh_ttl: chrono::Duration,
    bootstrap: BootstrapDeps,
}

impl std::fmt::Debug for AuthService {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        f.debug_struct("AuthService").finish_non_exhaustive()
    }
}

impl AuthService {
    pub fn new(
        users: Arc<dyn UserRepository>,
        refresh: Arc<dyn RefreshTokenRepository>,
        billing: Arc<dyn BillingRepository>,
        codec: TokenCodec,
        refresh_ttl: std::time::Duration,
        bootstrap: BootstrapDeps,
    ) -> AppResult<Self> {
        Ok(Self {
            users,
            refresh,
            billing,
            codec,
            refresh_ttl: chrono::Duration::from_std(refresh_ttl).map_err(AppError::internal)?,
            bootstrap,
        })
    }

    // ── first run and registration ──────────────────────────────────────────

    /// Does this installation still need its first account?
    pub async fn needs_setup(&self) -> AppResult<bool> {
        Ok(!self.bootstrap.repository.any_users_exist().await?)
    }

    /// Creates the first company and its manager.
    ///
    /// Guarded by a shared secret held by whoever deploys the server, because
    /// the endpoint is necessarily unauthenticated: before it runs there is
    /// nobody to authenticate as.
    pub async fn complete_setup(
        &self,
        setup_key: &str,
        input: &CompanyRegistration,
    ) -> AppResult<LoginOutcome> {
        if self.bootstrap.setup_secret.is_empty() {
            return Err(AppError::FeatureUnavailable(
                "Setup is disabled. SETUP_SECRET is not configured on the server.".into(),
            ));
        }
        // Constant-time: the secret is long-lived and the endpoint is public,
        // so a timing difference would leak it a character at a time.
        if !constant_time_eq(setup_key.as_bytes(), self.bootstrap.setup_secret.as_bytes()) {
            return Err(AppError::Domain(DomainError::Forbidden(
                "Invalid setup key",
            )));
        }
        self.register(input, true).await
    }

    /// Registers a company through the public sign-up form.
    pub async fn register_company(&self, input: &CompanyRegistration) -> AppResult<LoginOutcome> {
        self.register(input, false).await
    }

    async fn register(
        &self,
        input: &CompanyRegistration,
        bootstrap: bool,
    ) -> AppResult<LoginOutcome> {
        if input.company_name.trim().is_empty() {
            return Err(AppError::Domain(DomainError::invalid(
                "companyName",
                "a company name is required",
            )));
        }
        input.manager.validate(true).map_err(AppError::Domain)?;

        let password = input
            .manager
            .password
            .as_deref()
            .ok_or_else(|| AppError::Domain(DomainError::invalid("password", "is required")))?;
        let hash = password::hash(password)?;

        let registered = match self
            .bootstrap
            .repository
            .register_company(input, &hash, bootstrap)
            .await
        {
            Ok(r) => r,
            // The database signals this with its own SQLSTATE; it is a
            // refusal to act, not an upstream failure, so it is a 403.
            Err(pmk_ports::PortError::Rejected { detail, .. }) => {
                return Err(AppError::Domain(DomainError::Forbidden(
                    "Setup has already been completed",
                )))
                .inspect_err(|_| tracing::info!(%detail, "setup refused"));
            }
            Err(e) => return Err(e.into()),
        };
        self.issue_session(registered.manager).await
    }

    // ── password recovery ───────────────────────────────────────────────────

    /// Issues an emailed reset link, if this account is eligible for one.
    ///
    /// Returns the token and the user only when a link should actually be
    /// sent. Every other case -- unknown address, inactive account, a company
    /// with colleagues who could issue a code instead -- returns `None`, and
    /// the caller responds identically either way. A response that varied
    /// would tell an unauthenticated caller which addresses are real.
    pub async fn begin_password_reset(&self, email: &str) -> AppResult<Option<(UserId, String)>> {
        if email.trim().is_empty() {
            return Err(AppError::Domain(DomainError::invalid(
                "email",
                "Email is required",
            )));
        }

        let Some((user, active_users)) = self.bootstrap.repository.recovery_context(email).await?
        else {
            return Ok(None);
        };
        if !may_reset_by_email(active_users) {
            return Ok(None);
        }

        let token = random_hex(32);
        let expires_at =
            self.bootstrap.clock.now_utc() + chrono::Duration::minutes(RESET_LINK_TTL_MINUTES);
        self.bootstrap
            .repository
            .store_reset_token(user, &token, expires_at)
            .await?;
        Ok(Some((user, token)))
    }

    /// Sets a new password from an emailed link or a manager-issued code.
    pub async fn reset_password(&self, submitted: &str, password: &str) -> AppResult<()> {
        validate_reset(submitted, password).map_err(AppError::Domain)?;

        let candidates = ResetCandidates::of(submitted);
        let hashed = if candidates.normalised_code.is_empty() {
            String::new()
        } else {
            hash_recovery_code(&candidates.normalised_code)
        };

        let invalid = || AppError::Domain(DomainError::invalid("token", INVALID_TOKEN_MESSAGE));

        let Some(found) = self
            .bootstrap
            .repository
            .find_reset_token(&candidates.raw, &hashed)
            .await?
        else {
            return Err(invalid());
        };

        let hash = password::hash(password)?;
        // The claim is conditional, so a token spent between the lookup and
        // here fails rather than silently resetting twice.
        if self
            .bootstrap
            .repository
            .claim_reset_and_set_password(found.id, found.user_id, &hash)
            .await?
        {
            // Every session ends: whoever triggered the reset may be the
            // person being locked out, or the person locking them out.
            self.refresh.revoke_all_for_user(found.user_id).await?;
            Ok(())
        } else {
            Err(invalid())
        }
    }

    /// Authenticates by email and password.
    ///
    /// Every failure path returns the same `InvalidCredentials` error: an
    /// unknown email, an inactive account and a wrong password must be
    /// indistinguishable, or the endpoint becomes a user-enumeration oracle.
    pub async fn login(&self, email: &str, password_input: &str) -> AppResult<LoginOutcome> {
        let found = self.users.find_credential_by_email(email).await?;

        let Some(cred) = found else {
            // Hash anyway so a missing account is not detectably faster than a
            // wrong password.
            let _ = password::hash(password_input);
            return Err(AppError::InvalidCredentials);
        };

        if !cred.user.active {
            return Err(AppError::InvalidCredentials);
        }
        let Some(stored) = cred.password_hash.as_deref() else {
            return Err(AppError::InvalidCredentials);
        };

        let verdict = password::verify(password_input, stored);
        if !verdict.matched {
            return Err(AppError::InvalidCredentials);
        }

        // Transparent bcrypt -> Argon2id upgrade. A failure here must not fail
        // the login: the user authenticated correctly.
        if verdict.needs_rehash {
            match password::hash(password_input) {
                Ok(new_hash) => {
                    if let Err(e) = self
                        .users
                        .replace_password_hash(cred.user.id, &new_hash)
                        .await
                    {
                        tracing::warn!(user_id = %cred.user.id, error = %e,
                            "password rehash failed; login still succeeded");
                    } else {
                        tracing::info!(user_id = %cred.user.id,
                            "upgraded bcrypt password hash to argon2id");
                    }
                }
                Err(e) => tracing::warn!(error = %e, "could not compute argon2 hash"),
            }
        }

        self.issue_session(cred.user).await
    }

    /// Rotates a refresh token.
    ///
    /// Presenting a token that was already used means it leaked, so the entire
    /// family is revoked rather than just rejecting the request.
    pub async fn refresh(&self, presented: &str) -> AppResult<LoginOutcome> {
        let hash = hash_refresh(presented);
        let Some(record) = self.refresh.find_by_hash(&hash).await? else {
            return Err(AppError::InvalidToken);
        };

        if record.revoked_at.is_some() {
            return Err(AppError::InvalidToken);
        }
        if record.used_at.is_some() {
            let revoked = self.refresh.revoke_family(record.family_id).await?;
            tracing::warn!(
                user_id = %record.user_id, family = %record.family_id, revoked,
                "refresh token reuse detected; revoked the whole family"
            );
            return Err(AppError::InvalidToken);
        }
        if record.expires_at <= Utc::now() {
            return Err(AppError::InvalidToken);
        }

        let Some(user) = self.users.find_by_id_unscoped(record.user_id).await? else {
            return Err(AppError::InvalidToken);
        };
        if !user.active {
            self.refresh.revoke_all_for_user(user.id).await?;
            return Err(AppError::InvalidToken);
        }

        self.refresh.mark_used(record.id).await?;
        self.issue_in_family(user, record.family_id).await
    }

    pub async fn logout(&self, presented: &str) -> AppResult<()> {
        let hash = hash_refresh(presented);
        if let Some(record) = self.refresh.find_by_hash(&hash).await? {
            self.refresh.revoke_family(record.family_id).await?;
        }
        // Unknown tokens succeed silently: logout must be idempotent and must
        // not reveal whether a token was valid.
        Ok(())
    }

    /// Resolves a bearer token into a full session in **one** database
    /// round-trip for the user plus one for permissions and one for billing.
    ///
    /// The legacy stack re-verified the token and re-queried the user in every
    /// stacked guard, costing four to six round-trips per request
    /// (Analysis 4.5).
    pub async fn resolve_session(&self, bearer: &str) -> AppResult<SessionUser> {
        let claims = self.codec.verify_access(bearer)?;
        let user_id = claims.user_id()?;

        let Some(user) = self.users.find_by_id_unscoped(user_id).await? else {
            return Err(AppError::InvalidToken);
        };
        if !user.active {
            return Err(AppError::InvalidToken);
        }

        // A password change invalidates every outstanding access token.
        let current_ms = user.password_changed_at.map(|t| t.timestamp_millis());
        if current_ms != claims.password_changed_at_ms {
            return Err(AppError::InvalidToken);
        }

        let principal = user.principal();
        let permissions = self
            .users
            .permissions_for(principal.scope(), user.id)
            .await?;
        let entitlement = self.billing.entitlement(user.company_id).await?;

        Ok(SessionUser {
            user,
            principal,
            permissions,
            entitlement,
        })
    }

    async fn issue_session(&self, user: User) -> AppResult<LoginOutcome> {
        self.issue_in_family(user, uuid::Uuid::new_v4()).await
    }

    async fn issue_in_family(&self, user: User, family_id: uuid::Uuid) -> AppResult<LoginOutcome> {
        let now = Utc::now();
        let (access_token, access_claims) = self.codec.issue_access(&user, now)?;
        let refresh = RefreshToken::generate();
        self.refresh
            .insert(user.id, &refresh.hash, family_id, now + self.refresh_ttl)
            .await?;
        Ok(LoginOutcome {
            access_token,
            access_claims,
            refresh_token: refresh.value,
            user,
        })
    }
}

/// Compares two byte strings without an early return.
///
/// The setup secret is long-lived and its endpoint is public, so a comparison
/// that stopped at the first difference would leak it a character at a time.
fn constant_time_eq(a: &[u8], b: &[u8]) -> bool {
    if a.len() != b.len() {
        return false;
    }
    a.iter().zip(b).fold(0u8, |acc, (x, y)| acc | (x ^ y)) == 0
}

/// A random hex string of `bytes` bytes.
fn random_hex(bytes: usize) -> String {
    use rand::RngCore;
    let mut buf = vec![0u8; bytes];
    rand::rngs::OsRng.fill_bytes(&mut buf);
    hex::encode(buf)
}

/// Hashes a normalised manager code the way `UsersService` stores it.
fn hash_recovery_code(code: &str) -> String {
    use sha2::{Digest, Sha256};
    format!("manager:{}", hex::encode(Sha256::digest(code.as_bytes())))
}

#[cfg(test)]
mod bootstrap_tests {
    use super::*;

    #[test]
    fn the_secret_comparison_is_length_and_content_sensitive() {
        assert!(constant_time_eq(b"secret", b"secret"));
        assert!(!constant_time_eq(b"secret", b"secrez"));
        assert!(!constant_time_eq(b"secret", b"secre"));
        assert!(!constant_time_eq(b"", b"x"));
        assert!(constant_time_eq(b"", b""));
    }

    #[test]
    fn a_random_token_is_hex_of_the_right_length() {
        let t = random_hex(32);
        assert_eq!(t.len(), 64);
        assert!(t.chars().all(|c| c.is_ascii_hexdigit()));
        assert_ne!(t, random_hex(32));
    }

    #[test]
    fn the_code_hash_matches_what_the_issuer_stores() {
        // UsersService::issue_recovery_code writes exactly this shape; if the
        // two drifted, no manager-issued code would ever be accepted.
        let h = hash_recovery_code("ABC23");
        assert!(h.starts_with("manager:"));
        assert_eq!(h.len(), "manager:".len() + 64);
    }
}
