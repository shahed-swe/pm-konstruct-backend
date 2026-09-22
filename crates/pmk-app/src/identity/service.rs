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

pub struct AuthService {
    users: Arc<dyn UserRepository>,
    refresh: Arc<dyn RefreshTokenRepository>,
    billing: Arc<dyn BillingRepository>,
    codec: TokenCodec,
    refresh_ttl: chrono::Duration,
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
    ) -> AppResult<Self> {
        Ok(Self {
            users,
            refresh,
            billing,
            codec,
            refresh_ttl: chrono::Duration::from_std(refresh_ttl).map_err(AppError::internal)?,
        })
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
