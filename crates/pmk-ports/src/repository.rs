//! Repository ports.
//!
//! Every tenant-scoped method takes a `TenantScope`. It cannot be constructed
//! without a verified principal, so omitting tenant scoping is a compile error
//! rather than a cross-tenant leak -- the structural fix for Analysis 4.3.

use async_trait::async_trait;
use pmk_domain::ids::UserId;
use pmk_domain::tenant::{CompanyId, TenantScope};
use pmk_domain::{access::Permission, User};

use crate::PortResult;

/// A stored credential, kept separate from [`User`] so a hash is never carried
/// around inside a value that gets serialised into a response.
#[derive(Debug, Clone)]
pub struct StoredCredential {
    pub user: User,
    pub password_hash: Option<String>,
}

#[async_trait]
pub trait UserRepository: Send + Sync {
    /// Looks a user up by email for login. Deliberately **not** tenant-scoped:
    /// at login time no tenant is known yet. Emails are globally unique.
    async fn find_credential_by_email(&self, email: &str) -> PortResult<Option<StoredCredential>>;

    /// Re-loads a user during token verification. Not tenant-scoped for the
    /// same reason: the scope is derived *from* this result.
    async fn find_by_id_unscoped(&self, id: UserId) -> PortResult<Option<User>>;

    async fn replace_password_hash(&self, id: UserId, hash: &str) -> PortResult<()>;

    async fn permissions_for(&self, scope: TenantScope, id: UserId) -> PortResult<Vec<Permission>>;

    async fn list(&self, scope: TenantScope) -> PortResult<Vec<User>>;
}

/// Refresh-token family, for rotation with reuse detection.
#[derive(Debug, Clone)]
pub struct RefreshTokenRecord {
    pub id: i64,
    pub user_id: UserId,
    pub family_id: uuid::Uuid,
    pub expires_at: chrono::DateTime<chrono::Utc>,
    pub revoked_at: Option<chrono::DateTime<chrono::Utc>>,
    pub used_at: Option<chrono::DateTime<chrono::Utc>>,
}

#[async_trait]
pub trait RefreshTokenRepository: Send + Sync {
    async fn insert(
        &self,
        user_id: UserId,
        token_hash: &str,
        family_id: uuid::Uuid,
        expires_at: chrono::DateTime<chrono::Utc>,
    ) -> PortResult<()>;

    async fn find_by_hash(&self, token_hash: &str) -> PortResult<Option<RefreshTokenRecord>>;

    async fn mark_used(&self, id: i64) -> PortResult<()>;

    /// Revokes every token in a family. Called when a used token is presented
    /// again, which means it leaked.
    async fn revoke_family(&self, family_id: uuid::Uuid) -> PortResult<u64>;

    async fn revoke_all_for_user(&self, user_id: UserId) -> PortResult<u64>;

    async fn delete_expired(&self) -> PortResult<u64>;
}

/// Billing entitlement, read on nearly every request (domain-rules R6).
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct Entitlement {
    pub can_access_application: bool,
    pub can_configure_account: bool,
    pub onboarding_complete: bool,
}

impl Entitlement {
    /// Used until Phase 15 wires Stripe. Every company is entitled, which
    /// matches the legacy default `billing_onboarding_completed = true`.
    #[must_use]
    pub const fn permissive() -> Self {
        Self {
            can_access_application: true,
            can_configure_account: true,
            onboarding_complete: true,
        }
    }
}

#[async_trait]
pub trait BillingRepository: Send + Sync {
    async fn entitlement(&self, company: CompanyId) -> PortResult<Entitlement>;
}
