use std::sync::Arc;

use pmk_domain::access::{effective_permissions, Permission, Role};
use pmk_domain::identity::accounts::{
    assert_not_self, assert_permissions_editable, mark_managed, recovery_code_from, Seats,
    UserInput, RECOVERY_CODE_LEN, RECOVERY_CODE_TTL_MINUTES,
};
use pmk_domain::ids::UserId;
use pmk_domain::{DomainError, User};
use pmk_ports::repository::{BillingRepository, UserRepository};
use pmk_ports::Clock;

use crate::identity::password;
use crate::identity::SessionUser;
use crate::{AppError, AppResult};

/// A code a manager reads out to someone who is locked out.
#[derive(Debug, Clone)]
pub struct IssuedRecoveryCode {
    /// Shown once. Only its hash is stored.
    pub code: String,
    pub expires_at: chrono::DateTime<chrono::Utc>,
    pub user_id: UserId,
    pub user_name: String,
}

pub struct UsersService {
    users: Arc<dyn UserRepository>,
    billing: Arc<dyn BillingRepository>,
    clock: Arc<dyn Clock>,
}

impl std::fmt::Debug for UsersService {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        f.debug_struct("UsersService").finish_non_exhaustive()
    }
}

impl UsersService {
    #[must_use]
    pub fn new(
        users: Arc<dyn UserRepository>,
        billing: Arc<dyn BillingRepository>,
        clock: Arc<dyn Clock>,
    ) -> Self {
        Self {
            users,
            billing,
            clock,
        }
    }

    pub async fn list(&self, s: &SessionUser) -> AppResult<Vec<User>> {
        Ok(self.users.list(s.principal.scope()).await?)
    }

    pub async fn get(&self, s: &SessionUser, id: UserId) -> AppResult<User> {
        self.users
            .find(s.principal.scope(), id)
            .await?
            .ok_or_else(|| AppError::Domain(DomainError::not_found("User")))
    }

    /// Seats, as they stand right now.
    async fn seats(&self, s: &SessionUser) -> AppResult<Seats> {
        let entitlement = self.billing.entitlement(s.principal.company_id()).await?;
        Ok(Seats {
            limit: entitlement.seat_limit,
            in_use: self.users.active_count(s.principal.scope()).await?,
        })
    }

    pub async fn create(&self, s: &SessionUser, input: &UserInput) -> AppResult<User> {
        input.validate(true).map_err(AppError::Domain)?;

        // Only an active user consumes a seat, so an inactive one can always
        // be created -- which is how a company at its limit still records
        // someone joining next month.
        if input.active {
            let seats = self.seats(s).await?;
            if !seats.has_room() {
                return Err(AppError::Domain(DomainError::Conflict(
                    seats.full_message(false),
                )));
            }
        }

        let hash =
            password::hash(input.password.as_deref().ok_or_else(|| {
                AppError::Domain(DomainError::invalid("password", "is required"))
            })?)?;
        Ok(self.users.create(s.principal.scope(), input, &hash).await?)
    }

    pub async fn update(&self, s: &SessionUser, id: UserId, input: &UserInput) -> AppResult<User> {
        input.validate(false).map_err(AppError::Domain)?;
        let existing = self.get(s, id).await?;

        // A manager who demotes or deactivates themselves can lock the company
        // out of its own administration.
        if id == s.user.id {
            if !input.active {
                return Err(AppError::Domain(DomainError::invalid(
                    "active",
                    "You cannot deactivate your own account",
                )));
            }
            if input.role != existing.role {
                return Err(AppError::Domain(DomainError::invalid(
                    "role",
                    "You cannot change your own role",
                )));
            }
        }

        // Only a transition into active needs a seat; someone already active
        // keeps the one they hold.
        if input.active && !existing.active {
            let seats = self.seats(s).await?;
            if !seats.has_room() {
                return Err(AppError::Domain(DomainError::Conflict(
                    seats.full_message(true),
                )));
            }
        }

        // An empty password means "unchanged", which the domain already
        // accepts; it must not reach the hasher.
        let hash = match input.password.as_deref() {
            Some(p) if !p.is_empty() => Some(password::hash(p)?),
            _ => None,
        };

        self.users
            .update(s.principal.scope(), id, input, hash.as_deref())
            .await?
            .ok_or_else(|| AppError::Domain(DomainError::not_found("User")))
    }

    pub async fn delete(&self, s: &SessionUser, id: UserId) -> AppResult<()> {
        assert_not_self(s.user.id, id, "You cannot delete your own account")
            .map_err(AppError::Domain)?;
        self.get(s, id).await?;
        if self.users.delete(s.principal.scope(), id).await? {
            Ok(())
        } else {
            Err(AppError::Domain(DomainError::not_found("User")))
        }
    }

    /// The effective permissions the admin UI shows for a user.
    ///
    /// Empty for a manager: they hold everything, so an exhaustive list would
    /// imply the set is editable when it is not.
    pub async fn permissions(&self, s: &SessionUser, id: UserId) -> AppResult<Vec<Permission>> {
        let user = self.get(s, id).await?;
        if user.role == Role::Manager {
            return Ok(Vec::new());
        }
        let stored = self.users.permissions_for(s.principal.scope(), id).await?;
        Ok(effective_permissions(user.role, &stored)
            .into_iter()
            .collect())
    }

    /// Replaces a user's permissions.
    ///
    /// The sentinel is added here rather than trusted from the client: without
    /// it the stored rows are inert and the user silently keeps their role
    /// defaults, so saving an empty list would look like a revocation while
    /// changing nothing (R5).
    pub async fn set_permissions(
        &self,
        s: &SessionUser,
        id: UserId,
        permissions: Vec<Permission>,
    ) -> AppResult<()> {
        let user = self.get(s, id).await?;
        assert_permissions_editable(user.role).map_err(AppError::Domain)?;
        Ok(self
            .users
            .set_permissions(s.principal.scope(), id, &mark_managed(permissions))
            .await?)
    }

    /// Issues a one-time code a manager can read out to a locked-out user.
    pub async fn issue_recovery_code(
        &self,
        s: &SessionUser,
        id: UserId,
    ) -> AppResult<IssuedRecoveryCode> {
        let user = self.get(s, id).await?;
        // An inactive account has no business being recovered into: the code
        // would work and the login would then be refused.
        if !user.active {
            return Err(AppError::Domain(DomainError::invalid(
                "id",
                "Activate this user before issuing a recovery code",
            )));
        }

        let mut bytes = [0u8; RECOVERY_CODE_LEN];
        getrandom_bytes(&mut bytes);
        let code = recovery_code_from(&bytes);
        let expires_at =
            self.clock.now_utc() + chrono::Duration::minutes(RECOVERY_CODE_TTL_MINUTES);

        self.users
            .issue_recovery_code(
                s.principal.scope(),
                id,
                &hash_recovery_code(&code),
                expires_at,
            )
            .await?;

        Ok(IssuedRecoveryCode {
            code,
            expires_at,
            user_id: user.id,
            user_name: user.name,
        })
    }
}

/// Hashes a recovery code for storage.
///
/// Prefixed so a manager-issued code is distinguishable from a self-service
/// reset token in the same table, which is how the reset endpoint knows which
/// rules to apply. SHA-256 rather than Argon2 because the input is 49 bits of
/// server-chosen entropy with a fifteen-minute life, not a human password.
fn hash_recovery_code(code: &str) -> String {
    use sha2::{Digest, Sha256};
    let digest = Sha256::digest(code.as_bytes());
    format!("manager:{}", hex::encode(digest))
}

/// Fills `out` with cryptographically secure random bytes.
fn getrandom_bytes(out: &mut [u8]) {
    use rand::RngCore;
    rand::rngs::OsRng.fill_bytes(out);
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn a_recovery_code_hash_is_prefixed_and_deterministic() {
        let a = hash_recovery_code("ABC23");
        assert!(a.starts_with("manager:"), "{a}");
        assert_eq!(a, hash_recovery_code("ABC23"));
        assert_ne!(a, hash_recovery_code("ABC24"));
        // 8 for the prefix plus 64 hex characters.
        assert_eq!(a.len(), "manager:".len() + 64);
    }

    #[test]
    fn the_code_itself_never_appears_in_its_hash() {
        let code = "ZZZZZZZZZZ";
        assert!(!hash_recovery_code(code).contains(code));
    }

    #[test]
    fn random_bytes_are_not_all_the_same() {
        // A smoke test that the source is wired up at all.
        let mut a = [0u8; 16];
        let mut b = [0u8; 16];
        getrandom_bytes(&mut a);
        getrandom_bytes(&mut b);
        assert_ne!(a, b);
    }
}
