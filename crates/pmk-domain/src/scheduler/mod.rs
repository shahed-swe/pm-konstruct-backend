//! Trade scheduler — domain-rules R12.
//!
//! Three invariants, all enforced in the database as well as here:
//!
//! * one worker is allocated to at most one place per day
//!   (`uq_scheduler_company_worker_date`)
//! * an allocation targets a job **or** a maintenance job, never both and
//!   never neither (`scheduler_allocation_target_check`)
//! * absence periods for one worker may not overlap
//!   (`enforce_scheduler_worker_absence_no_overlap`, a trigger with inclusive
//!   date ranges and an advisory lock)

use crate::ids::{JobId, SchedulerAbsenceId, SchedulerAllocationId, SchedulerWorkerId};
use crate::tenant::CompanyId;
use crate::{DomainError, DomainResult};

/// Why a worker is unavailable. Constrained by
/// `scheduler_worker_absence_type_check`.
#[derive(Debug, Clone, Copy, PartialEq, Eq, serde::Serialize, serde::Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum AbsenceType {
    Sick,
    Leave,
    TradeSchool,
}

impl AbsenceType {
    #[must_use]
    pub fn parse(s: &str) -> Option<Self> {
        match s {
            "sick" => Some(Self::Sick),
            "leave" => Some(Self::Leave),
            "trade_school" => Some(Self::TradeSchool),
            _ => None,
        }
    }

    #[must_use]
    pub const fn as_str(self) -> &'static str {
        match self {
            Self::Sick => "sick",
            Self::Leave => "leave",
            Self::TradeSchool => "trade_school",
        }
    }

    pub const ALL: &'static str = "sick, leave, trade_school";
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub struct Worker {
    pub id: SchedulerWorkerId,
    pub company_id: CompanyId,
    pub name: String,
    /// Free text in production ("Carpenter", "Apprentice"), not an enum.
    pub trade: Option<String>,
    pub color: String,
    pub active: bool,
    pub on_leave: bool,
    pub leave_from: Option<chrono::NaiveDate>,
    pub leave_to: Option<chrono::NaiveDate>,
}

#[derive(Debug, Clone, Default)]
pub struct WorkerInput {
    pub name: String,
    pub trade: Option<String>,
    pub color: Option<String>,
    pub active: Option<bool>,
}

impl WorkerInput {
    pub fn validate(&self) -> DomainResult<()> {
        if self.name.trim().is_empty() {
            return Err(DomainError::invalid("name", "is required"));
        }
        if self.name.len() > 200 {
            return Err(DomainError::invalid(
                "name",
                "must be 200 characters or fewer",
            ));
        }
        if let Some(c) = &self.color {
            if !is_hex_colour(c) {
                return Err(DomainError::invalid(
                    "color",
                    "must be a hex colour such as #3B82F6",
                ));
            }
        }
        Ok(())
    }

    #[must_use]
    pub fn normalised(mut self) -> Self {
        self.name = self.name.trim().to_string();
        self.trade = self
            .trade
            .map(|t| t.trim().to_string())
            .filter(|t| !t.is_empty());
        self
    }
}

/// The scheduler board renders each worker in their own colour, so a bad value
/// breaks the UI rather than being merely cosmetic.
fn is_hex_colour(s: &str) -> bool {
    let s = s.trim();
    (s.len() == 7 || s.len() == 4)
        && s.starts_with('#')
        && s[1..].chars().all(|c| c.is_ascii_hexdigit())
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub struct Absence {
    pub id: SchedulerAbsenceId,
    pub worker_id: SchedulerWorkerId,
    pub absence_type: AbsenceType,
    pub start_date: chrono::NaiveDate,
    pub end_date: chrono::NaiveDate,
}

#[derive(Debug, Clone)]
pub struct AbsenceInput {
    pub worker_id: i32,
    pub absence_type: AbsenceType,
    pub start_date: chrono::NaiveDate,
    pub end_date: chrono::NaiveDate,
}

impl AbsenceInput {
    /// Validates before the database sees it.
    ///
    /// This matters more than usual here. `scheduler_worker_absence_dates_check`
    /// exists, but Postgres fires `BEFORE ROW` triggers *before* CHECK
    /// constraints, so an inverted range reaches `daterange(start, end, '[]')`
    /// inside the overlap trigger and raises a raw `22000 data_exception` --
    /// a 500 rather than a 400. Catching it here is what keeps that from
    /// happening (domain-rules R12).
    pub fn validate(&self) -> DomainResult<()> {
        if self.worker_id <= 0 {
            return Err(DomainError::invalid("workerId", "is required"));
        }
        if self.start_date > self.end_date {
            return Err(DomainError::invalid(
                "endDate",
                "must be on or after the start date",
            ));
        }
        // A decade-long absence is a typo, and the board renders day by day.
        if (self.end_date - self.start_date).num_days() > 366 {
            return Err(DomainError::invalid(
                "endDate",
                "an absence cannot exceed one year",
            ));
        }
        Ok(())
    }

    /// Whether this period overlaps another. Bounds are **inclusive** on both
    /// ends, matching the trigger's `'[]'` ranges: an absence ending 5 May and
    /// one starting 5 May do overlap.
    #[must_use]
    pub fn overlaps(&self, other_start: chrono::NaiveDate, other_end: chrono::NaiveDate) -> bool {
        self.start_date <= other_end && other_start <= self.end_date
    }
}

/// What an allocation points at. Exactly one, never both, never neither.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum AllocationTarget {
    Job(JobId),
    MaintenanceJob(i32),
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub struct Allocation {
    pub id: SchedulerAllocationId,
    pub worker_id: SchedulerWorkerId,
    pub target: AllocationTarget,
    pub assigned_date: chrono::NaiveDate,
    pub note: Option<String>,
}

#[derive(Debug, Clone)]
pub struct AllocationInput {
    pub worker_id: i32,
    pub job_id: Option<i32>,
    pub maintenance_job_id: Option<i32>,
    pub assigned_date: chrono::NaiveDate,
    pub note: Option<String>,
}

impl AllocationInput {
    /// Mirrors `scheduler_allocation_target_check`, which the legacy schema
    /// already enforced.
    pub fn target(&self) -> DomainResult<AllocationTarget> {
        match (self.job_id, self.maintenance_job_id) {
            (Some(j), None) => Ok(AllocationTarget::Job(JobId(j))),
            (None, Some(m)) => Ok(AllocationTarget::MaintenanceJob(m)),
            (Some(_), Some(_)) => Err(DomainError::invalid(
                "jobId",
                "an allocation targets a job or a maintenance job, not both",
            )),
            (None, None) => Err(DomainError::invalid(
                "jobId",
                "an allocation must target a job or a maintenance job",
            )),
        }
    }

    pub fn validate(&self) -> DomainResult<()> {
        if self.worker_id <= 0 {
            return Err(DomainError::invalid("workerId", "is required"));
        }
        self.target()?;
        Ok(())
    }
}

/// A day the board must show as unavailable.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct AbsenceConflict {
    pub absence_id: SchedulerAbsenceId,
    pub absence_type: AbsenceType,
    pub start_date: chrono::NaiveDate,
    pub end_date: chrono::NaiveDate,
}

/// Finds an absence covering `date`, if any.
///
/// Allocating into one returns 409 with the conflicting absence, so the UI can
/// say *why* rather than just refusing.
#[must_use]
pub fn absence_covering(
    absences: &[Absence],
    worker: SchedulerWorkerId,
    date: chrono::NaiveDate,
) -> Option<AbsenceConflict> {
    absences
        .iter()
        .find(|a| a.worker_id == worker && a.start_date <= date && date <= a.end_date)
        .map(|a| AbsenceConflict {
            absence_id: a.id,
            absence_type: a.absence_type,
            start_date: a.start_date,
            end_date: a.end_date,
        })
}

#[cfg(test)]
mod tests {
    use super::*;

    fn d(s: &str) -> chrono::NaiveDate {
        s.parse().expect("test date")
    }

    fn absence(start: &str, end: &str) -> AbsenceInput {
        AbsenceInput {
            worker_id: 1,
            absence_type: AbsenceType::Leave,
            start_date: d(start),
            end_date: d(end),
        }
    }

    #[test]
    fn absence_types_round_trip() {
        for s in ["sick", "leave", "trade_school"] {
            assert_eq!(AbsenceType::parse(s).map(AbsenceType::as_str), Some(s));
        }
        assert_eq!(AbsenceType::parse("holiday"), None);
    }

    #[test]
    fn an_inverted_range_is_caught_before_the_database_sees_it() {
        // Without this the overlap trigger raises 22000 and the API 500s.
        match absence("2026-06-10", "2026-06-01").validate() {
            Err(DomainError::Invalid { field, .. }) => assert_eq!(field, "endDate"),
            other => panic!("expected rejection, got {other:?}"),
        }
    }

    #[test]
    fn a_single_day_absence_is_valid() {
        assert!(absence("2026-05-05", "2026-05-05").validate().is_ok());
    }

    #[test]
    fn an_absurdly_long_absence_is_rejected() {
        assert!(absence("2026-01-01", "2030-01-01").validate().is_err());
    }

    #[test]
    fn overlap_bounds_are_inclusive_like_the_trigger() {
        let a = absence("2026-05-01", "2026-05-10");
        // Touching on the end date counts as overlapping.
        assert!(a.overlaps(d("2026-05-10"), d("2026-05-12")));
        assert!(a.overlaps(d("2026-04-25"), d("2026-05-01")));
        // Fully inside, and fully containing.
        assert!(a.overlaps(d("2026-05-03"), d("2026-05-04")));
        assert!(a.overlaps(d("2026-04-01"), d("2026-06-01")));
        // The day either side does not.
        assert!(!a.overlaps(d("2026-05-11"), d("2026-05-12")));
        assert!(!a.overlaps(d("2026-04-28"), d("2026-04-30")));
    }

    #[test]
    fn an_allocation_needs_exactly_one_target() {
        let base = AllocationInput {
            worker_id: 1,
            job_id: None,
            maintenance_job_id: None,
            assigned_date: d("2026-05-01"),
            note: None,
        };
        assert!(base.target().is_err(), "neither");

        let both = AllocationInput {
            job_id: Some(1),
            maintenance_job_id: Some(2),
            ..base.clone()
        };
        assert!(both.target().is_err(), "both");

        let job = AllocationInput {
            job_id: Some(7),
            ..base.clone()
        };
        assert_eq!(job.target().unwrap(), AllocationTarget::Job(JobId(7)));

        let maint = AllocationInput {
            maintenance_job_id: Some(3),
            ..base
        };
        assert_eq!(maint.target().unwrap(), AllocationTarget::MaintenanceJob(3));
    }

    #[test]
    fn worker_colour_must_be_hex() {
        let mut w = WorkerInput {
            name: "Chippy".into(),
            ..Default::default()
        };
        assert!(w.validate().is_ok(), "colour is optional");
        for good in ["#3B82F6", "#fff", "#ABCDEF"] {
            w.color = Some(good.into());
            assert!(w.validate().is_ok(), "{good}");
        }
        for bad in ["3B82F6", "#GGGGGG", "blue", "#12345", ""] {
            w.color = Some(bad.into());
            assert!(w.validate().is_err(), "{bad} should be rejected");
        }
    }

    #[test]
    fn worker_name_is_required_and_trimmed() {
        let w = WorkerInput {
            name: "  ".into(),
            ..Default::default()
        };
        assert!(w.validate().is_err());
        let w = WorkerInput {
            name: "  Chippy  ".into(),
            trade: Some("  ".into()),
            ..Default::default()
        }
        .normalised();
        assert_eq!(w.name, "Chippy");
        assert_eq!(w.trade, None, "a blank trade is absent, not empty string");
    }

    #[test]
    fn absence_covering_finds_the_conflicting_period() {
        let list = vec![Absence {
            id: SchedulerAbsenceId(9),
            worker_id: SchedulerWorkerId(1),
            absence_type: AbsenceType::Sick,
            start_date: d("2026-05-01"),
            end_date: d("2026-05-10"),
        }];
        let hit = absence_covering(&list, SchedulerWorkerId(1), d("2026-05-05"));
        assert_eq!(hit.map(|c| c.absence_id), Some(SchedulerAbsenceId(9)));
        // Inclusive on both ends.
        assert!(absence_covering(&list, SchedulerWorkerId(1), d("2026-05-01")).is_some());
        assert!(absence_covering(&list, SchedulerWorkerId(1), d("2026-05-10")).is_some());
        assert!(absence_covering(&list, SchedulerWorkerId(1), d("2026-05-11")).is_none());
        // Another worker's absence is not this worker's problem.
        assert!(absence_covering(&list, SchedulerWorkerId(2), d("2026-05-05")).is_none());
    }
}
