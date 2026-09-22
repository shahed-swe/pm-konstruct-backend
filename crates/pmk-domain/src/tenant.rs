//! Tenant identity and the unforgeable scope token.
//!
//! Legacy tenancy was opt-in: `companyId` was threaded by hand per service, and
//! several services carried no reference to it at all, relying on the caller
//! remembering to call `assertJobAccess()` first (Analysis 4.3). Worse,
//! `getAuthorizedJobIds()` returned `null` meaning "all jobs" when the company
//! was absent, and `assertJobAccess()` treated `null` as "allow".
//!
//! Here, a [`TenantScope`] cannot be constructed from thin air: the only
//! constructor takes an [`AuthenticatedPrincipal`], which only `pmk-api` can
//! mint after verifying a token. Every repository method demands one, so an
//! unscoped query is a compile error rather than a cross-tenant leak.
//!
//! Postgres row-level security (migration 0004) is the second layer: with the
//! `app.company_id` GUC unset, every policy evaluates false and queries return
//! zero rows -- fail closed, not open. Verified by `tools/phase3/test_rls.sh`.

use crate::ids::UserId;
use std::fmt;

#[derive(
    Debug, Clone, Copy, PartialEq, Eq, PartialOrd, Ord, Hash, serde::Serialize, serde::Deserialize,
)]
#[serde(transparent)]
pub struct CompanyId(pub i32);

impl CompanyId {
    #[must_use]
    pub const fn new(v: i32) -> Self {
        Self(v)
    }
    #[must_use]
    pub const fn get(self) -> i32 {
        self.0
    }
}

impl fmt::Display for CompanyId {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        write!(f, "{}", self.0)
    }
}

/// A caller whose token has been verified. Produced only by the API layer's
/// auth extractor; the private field stops anything else constructing one.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct AuthenticatedPrincipal {
    user_id: UserId,
    company_id: CompanyId,
    role: crate::access::Role,
    _seal: Sealed,
}

/// Prevents construction outside this crate's `new` (and therefore outside a
/// verified-token code path), while keeping the struct's fields readable.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
struct Sealed;

impl AuthenticatedPrincipal {
    /// Mint a principal. Call this **only** after verifying a token and loading
    /// the user from the database.
    #[must_use]
    pub fn new(user_id: UserId, company_id: CompanyId, role: crate::access::Role) -> Self {
        Self {
            user_id,
            company_id,
            role,
            _seal: Sealed,
        }
    }

    #[must_use]
    pub const fn user_id(&self) -> UserId {
        self.user_id
    }
    #[must_use]
    pub const fn company_id(&self) -> CompanyId {
        self.company_id
    }
    #[must_use]
    pub const fn role(&self) -> crate::access::Role {
        self.role
    }

    /// The tenant scope for this principal. There is no other way to get one.
    #[must_use]
    pub const fn scope(&self) -> TenantScope {
        TenantScope(self.company_id)
    }
}

/// Proof that a query is tenant-scoped.
///
/// Deliberately **not** `Default`, and not constructible from a bare `i32`
/// outside this module: that is the whole point.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct TenantScope(CompanyId);

impl TenantScope {
    #[must_use]
    pub const fn company_id(self) -> CompanyId {
        self.0
    }

    /// Escape hatch for the worker and CLI, which are legitimately
    /// cross-tenant (billing webhook delivery, media GC). Named to be
    /// conspicuous in review and in a grep; never call it from `pmk-api`.
    #[must_use]
    pub const fn for_system_task_bypassing_tenant_checks(company_id: CompanyId) -> Self {
        Self(company_id)
    }
}

impl fmt::Display for TenantScope {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        write!(f, "company={}", self.0)
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::access::Role;

    #[test]
    fn scope_comes_from_the_principal() {
        let p = AuthenticatedPrincipal::new(UserId(7), CompanyId(3), Role::Manager);
        assert_eq!(p.scope().company_id(), CompanyId(3));
        assert_eq!(p.user_id(), UserId(7));
    }

    #[test]
    fn scopes_for_different_tenants_are_not_equal() {
        let a = AuthenticatedPrincipal::new(UserId(1), CompanyId(1), Role::Office);
        let b = AuthenticatedPrincipal::new(UserId(2), CompanyId(2), Role::Office);
        assert_ne!(a.scope(), b.scope());
    }
}
