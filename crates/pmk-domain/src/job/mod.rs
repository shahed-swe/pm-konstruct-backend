//! Job visibility and assignment invariants — domain-rules R3, R4.

use crate::access::Role;
use crate::ids::{JobId, UserId};
use crate::{DomainError, DomainResult};
use std::collections::BTreeSet;

#[derive(Debug, Clone, Copy, PartialEq, Eq, serde::Serialize, serde::Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum JobStatus {
    Active,
    Completed,
    Archived,
    OnHold,
}

impl JobStatus {
    /// Constrained in the database by `jobs_status_check` (migration 0003).
    #[must_use]
    pub fn parse(s: &str) -> Option<Self> {
        match s {
            "active" => Some(Self::Active),
            "completed" => Some(Self::Completed),
            "archived" => Some(Self::Archived),
            "on_hold" => Some(Self::OnHold),
            _ => None,
        }
    }
    #[must_use]
    pub const fn as_str(self) -> &'static str {
        match self {
            Self::Active => "active",
            Self::Completed => "completed",
            Self::Archived => "archived",
            Self::OnHold => "on_hold",
        }
    }
}

/// How a caller may see jobs. `All` means every job in the tenant; tenancy
/// itself is still enforced by `TenantScope` and RLS, never by this type.
#[derive(Debug, Clone, PartialEq, Eq)]
pub enum JobVisibility {
    /// MANAGER and OFFICE: all company jobs.
    AllInCompany,
    /// SUPERVISOR: assignments UNION jobs where they are primary supervisor.
    Restricted(BTreeSet<JobId>),
}

impl JobVisibility {
    /// R3. Note this takes the *already-resolved* id sets rather than querying:
    /// the rule stays pure and testable, and the repository owns the SQL.
    #[must_use]
    pub fn resolve(role: Role, assigned: &[JobId], primary_supervisor_of: &[JobId]) -> Self {
        if role.sees_all_company_jobs() {
            return Self::AllInCompany;
        }
        let mut ids: BTreeSet<JobId> = assigned.iter().copied().collect();
        ids.extend(primary_supervisor_of.iter().copied());
        Self::Restricted(ids)
    }

    #[must_use]
    pub fn allows(&self, job: JobId) -> bool {
        match self {
            Self::AllInCompany => true,
            Self::Restricted(ids) => ids.contains(&job),
        }
    }

    /// Unlike the legacy `assertJobAccess`, there is no "unknown means allow"
    /// path: an empty restricted set denies everything.
    pub fn assert_allows(&self, job: JobId) -> DomainResult<()> {
        if self.allows(job) {
            Ok(())
        } else {
            // 404 rather than 403 so job existence does not leak.
            Err(DomainError::not_found("Job"))
        }
    }
}

/// R4. Which user `jobs.supervisor_id` must mirror, given a job's assignments
/// ordered by `assigned_at ASC`.
///
/// The primary assignment wins; otherwise the earliest assignment; otherwise
/// none. The legacy field was misleadingly called `id` while holding a user id
/// — here the type says what it is.
#[must_use]
pub fn primary_supervisor(assignments_by_assigned_at: &[(UserId, bool)]) -> Option<UserId> {
    assignments_by_assigned_at
        .iter()
        .find(|(_, is_primary)| *is_primary)
        .or_else(|| assignments_by_assigned_at.first())
        .map(|(user, _)| *user)
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn managers_and_office_see_all_company_jobs() {
        for role in [Role::Manager, Role::Office] {
            assert_eq!(
                JobVisibility::resolve(role, &[], &[]),
                JobVisibility::AllInCompany
            );
            assert!(JobVisibility::resolve(role, &[], &[]).allows(JobId(999)));
        }
    }

    #[test]
    fn supervisor_sees_the_union_of_assignments_and_primary_jobs() {
        let v = JobVisibility::resolve(
            Role::Supervisor,
            &[JobId(1), JobId(2)],
            &[JobId(2), JobId(3)],
        );
        assert!(v.allows(JobId(1)));
        assert!(v.allows(JobId(2)));
        assert!(
            v.allows(JobId(3)),
            "primary-supervisor jobs count even without an assignment"
        );
        assert!(!v.allows(JobId(4)));
    }

    #[test]
    fn supervisor_with_nothing_sees_nothing() {
        let v = JobVisibility::resolve(Role::Supervisor, &[], &[]);
        assert!(!v.allows(JobId(1)));
        // The legacy null-means-everything fallback is unrepresentable here.
        assert_eq!(
            v.assert_allows(JobId(1)),
            Err(DomainError::not_found("Job"))
        );
    }

    #[test]
    fn primary_assignment_wins_over_earlier_one() {
        let a = [(UserId(5), false), (UserId(6), true), (UserId(7), false)];
        assert_eq!(primary_supervisor(&a), Some(UserId(6)));
    }

    #[test]
    fn falls_back_to_the_earliest_assignment() {
        let a = [(UserId(5), false), (UserId(6), false)];
        assert_eq!(primary_supervisor(&a), Some(UserId(5)));
    }

    #[test]
    fn no_assignments_means_null_supervisor() {
        assert_eq!(primary_supervisor(&[]), None);
    }

    #[test]
    fn job_status_round_trips() {
        for s in ["active", "completed", "archived", "on_hold"] {
            assert_eq!(JobStatus::parse(s).map(JobStatus::as_str), Some(s));
        }
        assert_eq!(JobStatus::parse("pending"), None);
    }
}
