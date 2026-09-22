//! Roles, permissions and effective-permission resolution — domain-rules R5.
//!
//! Ported exactly from `services/permissionsService.ts`, including the
//! `access-rules:managed` sentinel, whose semantics are subtle:
//!
//!   * sentinel **absent**  -> stored grants are ignored entirely; role
//!                             defaults apply
//!   * sentinel **present** -> stored grants apply verbatim, minus the sentinel
//!
//! So a user granted `jobs:read` with no sentinel does not get it *from that
//! row* — they get it from the defaults, coincidentally. Revoking it changes
//! nothing until the sentinel exists. That is confusing but it is the live
//! behaviour, and v1 reproduces it. Replacing it with an explicit
//! `users.permissions_managed` boolean is a post-launch item.
//!
//! Acceptance: `docs/contract/rbac-matrix.csv`, all 1,710 rows.

use std::collections::BTreeSet;
use std::fmt;

#[derive(Debug, Clone, Copy, PartialEq, Eq, PartialOrd, Ord, Hash,
         serde::Serialize, serde::Deserialize)]
#[serde(rename_all = "UPPERCASE")]
pub enum Role {
    Manager,
    Supervisor,
    Office,
}

impl Role {
    /// Parses the stored `users.role` text. Constrained in the database by
    /// `users_role_check` (migration 0003), so `None` means corrupt data.
    #[must_use]
    pub fn parse(s: &str) -> Option<Self> {
        match s {
            "MANAGER" => Some(Self::Manager),
            "SUPERVISOR" => Some(Self::Supervisor),
            "OFFICE" => Some(Self::Office),
            _ => None,
        }
    }

    #[must_use]
    pub const fn as_str(self) -> &'static str {
        match self {
            Self::Manager => "MANAGER",
            Self::Supervisor => "SUPERVISOR",
            Self::Office => "OFFICE",
        }
    }

    /// Managers bypass permission checks entirely and never consult the
    /// permission table.
    #[must_use]
    pub const fn bypasses_permission_checks(self) -> bool {
        matches!(self, Self::Manager)
    }

    /// `MANAGER` and `OFFICE` see every job in their company; `SUPERVISOR` sees
    /// only assignments plus jobs where they are the primary supervisor
    /// (domain-rules R3).
    #[must_use]
    pub const fn sees_all_company_jobs(self) -> bool {
        matches!(self, Self::Manager | Self::Office)
    }
}

impl fmt::Display for Role {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        f.write_str(self.as_str())
    }
}

/// A `resource:action` grant, e.g. `site-diary:write`.
#[derive(Debug, Clone, PartialEq, Eq, PartialOrd, Ord, Hash,
         serde::Serialize, serde::Deserialize)]
pub struct Permission {
    pub resource: String,
    pub action: String,
}

impl Permission {
    pub fn new(resource: impl Into<String>, action: impl Into<String>) -> Self {
        Self { resource: resource.into(), action: action.into() }
    }

    /// Parses `"resource:action"`. Rejects empty halves and extra colons.
    #[must_use]
    pub fn parse(s: &str) -> Option<Self> {
        let (r, a) = s.split_once(':')?;
        if r.is_empty() || a.is_empty() || a.contains(':') {
            return None;
        }
        Some(Self::new(r, a))
    }

    #[must_use]
    pub fn key(&self) -> String {
        format!("{}:{}", self.resource, self.action)
    }
}

impl fmt::Display for Permission {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        write!(f, "{}:{}", self.resource, self.action)
    }
}

/// The sentinel row that switches a user from role defaults to stored grants.
pub const MANAGED_MARKER_RESOURCE: &str = "access-rules";
pub const MANAGED_MARKER_ACTION: &str = "managed";

/// Defaults for every non-supervisor, non-manager role (i.e. `OFFICE`).
pub const NON_MANAGER_VIEW_DEFAULTS: &[(&str, &str)] = &[
    ("jobs", "read"),
    ("site-diary", "read"),
    ("call-forward", "read"),
    ("reports", "read"),
    ("trade-scheduler", "read"),
];

/// Supervisor defaults: the view set plus writes on the three operational
/// modules.
pub const SUPERVISOR_EXTRA_DEFAULTS: &[(&str, &str)] = &[
    ("site-diary", "write"),
    ("call-forward", "write"),
    ("trade-scheduler", "write"),
];

fn defaults_for(role: Role) -> BTreeSet<Permission> {
    let mut set: BTreeSet<Permission> = NON_MANAGER_VIEW_DEFAULTS
        .iter()
        .map(|(r, a)| Permission::new(*r, *a))
        .collect();
    if role == Role::Supervisor {
        set.extend(
            SUPERVISOR_EXTRA_DEFAULTS
                .iter()
                .map(|(r, a)| Permission::new(*r, *a)),
        );
    }
    set
}

fn is_managed_marker(p: &Permission) -> bool {
    p.resource == MANAGED_MARKER_RESOURCE && p.action == MANAGED_MARKER_ACTION
}

/// Resolves the permissions actually in force for a user.
///
/// `stored` is the raw contents of `user_permissions` for this user.
#[must_use]
pub fn effective_permissions(role: Role, stored: &[Permission]) -> BTreeSet<Permission> {
    let managed = stored.iter().any(is_managed_marker);
    if !managed {
        return defaults_for(role);
    }
    stored
        .iter()
        .filter(|p| !is_managed_marker(p))
        .cloned()
        .collect()
}

/// Whether a user may perform `resource:action`.
///
/// Managers short-circuit to `true` without consulting `stored`, matching
/// `requireManagerOrPermission` and `requireManagerSupervisorOrPermission`
/// (which, despite the name, are identical in effect).
#[must_use]
pub fn has_permission(role: Role, stored: &[Permission], resource: &str, action: &str) -> bool {
    if role.bypasses_permission_checks() {
        return true;
    }
    effective_permissions(role, stored)
        .iter()
        .any(|p| p.resource == resource && p.action == action)
}

/// Marks a permission list as managed, for the manager-facing permissions UI.
#[must_use]
pub fn mark_managed(mut permissions: Vec<Permission>) -> Vec<Permission> {
    if !permissions.iter().any(is_managed_marker) {
        permissions.push(Permission::new(MANAGED_MARKER_RESOURCE, MANAGED_MARKER_ACTION));
    }
    permissions
}

#[cfg(test)]
mod tests {
    use super::*;

    fn p(s: &str) -> Permission {
        Permission::parse(s).expect("test permission literal")
    }
    fn marker() -> Permission {
        Permission::new(MANAGED_MARKER_RESOURCE, MANAGED_MARKER_ACTION)
    }

    // -- managers -----------------------------------------------------------
    #[test]
    fn manager_bypasses_everything_including_unknown_resources() {
        assert!(has_permission(Role::Manager, &[], "jobs", "write"));
        assert!(has_permission(Role::Manager, &[], "anything", "at-all"));
    }

    // -- sentinel absent: defaults apply ------------------------------------
    #[test]
    fn supervisor_without_sentinel_gets_supervisor_defaults() {
        let eff = effective_permissions(Role::Supervisor, &[]);
        assert_eq!(eff.len(), 8, "5 view + 3 write");
        for k in ["jobs:read", "site-diary:read", "call-forward:read", "reports:read",
                  "trade-scheduler:read", "site-diary:write", "call-forward:write",
                  "trade-scheduler:write"] {
            assert!(eff.contains(&p(k)), "missing {k}");
        }
        assert!(!eff.contains(&p("jobs:write")), "supervisors do not write jobs by default");
    }

    #[test]
    fn office_without_sentinel_gets_read_only_defaults() {
        let eff = effective_permissions(Role::Office, &[]);
        assert_eq!(eff.len(), 5);
        assert!(eff.contains(&p("jobs:read")));
        assert!(!eff.contains(&p("site-diary:write")));
    }

    #[test]
    fn stored_grants_are_ignored_without_the_sentinel() {
        // The counter-intuitive half of R5: this grant does nothing.
        let eff = effective_permissions(Role::Office, &[p("jobs:write")]);
        assert!(!eff.contains(&p("jobs:write")));
        assert_eq!(eff, effective_permissions(Role::Office, &[]));
    }

    #[test]
    fn revoking_a_default_without_the_sentinel_has_no_effect() {
        // Storing nothing but the sentinel is the only way to remove access.
        let eff = effective_permissions(Role::Supervisor, &[]);
        assert!(eff.contains(&p("jobs:read")));
    }

    // -- sentinel present: stored grants apply ------------------------------
    #[test]
    fn sentinel_with_no_grants_removes_all_access() {
        let eff = effective_permissions(Role::Supervisor, &[marker()]);
        assert!(eff.is_empty(), "managed with zero grants means nothing allowed");
        assert!(!has_permission(Role::Supervisor, &[marker()], "jobs", "read"));
    }

    #[test]
    fn sentinel_applies_grants_verbatim_and_hides_itself() {
        let stored = vec![marker(), p("jobs:read"), p("jobs:write")];
        let eff = effective_permissions(Role::Supervisor, &stored);
        assert_eq!(eff.len(), 2);
        assert!(eff.contains(&p("jobs:read")));
        assert!(eff.contains(&p("jobs:write")));
        assert!(!eff.iter().any(is_managed_marker), "marker must not leak out");
        // Defaults no longer apply once managed.
        assert!(!eff.contains(&p("site-diary:read")));
    }

    #[test]
    fn sentinel_can_grant_more_than_the_defaults() {
        let stored = vec![marker(), p("jobs:write")];
        assert!(has_permission(Role::Office, &stored, "jobs", "write"));
    }

    // -- role helpers -------------------------------------------------------
    #[test]
    fn job_visibility_flags_match_r3() {
        assert!(Role::Manager.sees_all_company_jobs());
        assert!(Role::Office.sees_all_company_jobs());
        assert!(!Role::Supervisor.sees_all_company_jobs());
    }

    #[test]
    fn role_round_trips_and_rejects_unknown() {
        for s in ["MANAGER", "SUPERVISOR", "OFFICE"] {
            assert_eq!(Role::parse(s).map(Role::as_str), Some(s));
        }
        assert_eq!(Role::parse("ADMIN"), None);
        assert_eq!(Role::parse("manager"), None, "stored values are uppercase");
    }

    // -- permission parsing -------------------------------------------------
    #[test]
    fn permission_parsing_rejects_malformed_keys() {
        assert_eq!(p("jobs:read").key(), "jobs:read");
        assert_eq!(Permission::parse("jobs"), None);
        assert_eq!(Permission::parse(":read"), None);
        assert_eq!(Permission::parse("jobs:"), None);
        assert_eq!(Permission::parse("a:b:c"), None);
    }

    #[test]
    fn mark_managed_is_idempotent() {
        let once = mark_managed(vec![p("jobs:read")]);
        let twice = mark_managed(once.clone());
        assert_eq!(once, twice);
        assert_eq!(twice.iter().filter(|x| is_managed_marker(x)).count(), 1);
    }
}
