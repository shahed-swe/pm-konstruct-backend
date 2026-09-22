//! Calendar event shaping.
//!
//! The month arithmetic and the date fallback live here because they are the
//! parts that can be wrong in ways SQL will not catch: an off-by-one at a
//! month boundary silently drops a day's work off the calendar.

use crate::error::{DomainError, DomainResult};
use crate::ids::{JobId, UserId};

/// What a calendar bar represents.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum EventKind {
    Job,
    /// A `STAGE_CLAIM` call-forward item.
    Claim,
    /// A `TASK` call-forward item.
    Task,
}

impl EventKind {
    #[must_use]
    pub const fn as_str(self) -> &'static str {
        match self {
            Self::Job => "job",
            Self::Claim => "claim",
            Self::Task => "task",
        }
    }

    /// Parses the `type` query parameter. `all` is `None` -- every kind.
    pub fn parse_filter(raw: Option<&str>) -> DomainResult<Option<Self>> {
        match raw.map(str::trim) {
            None | Some("") | Some("all") => Ok(None),
            Some("job") => Ok(Some(Self::Job)),
            Some("claim") => Ok(Some(Self::Claim)),
            Some("task") => Ok(Some(Self::Task)),
            Some(_) => Err(DomainError::invalid(
                "type",
                "must be one of job, claim, task, all",
            )),
        }
    }

    /// True when this kind should be included under the given filter.
    #[must_use]
    pub fn included_by(self, filter: Option<Self>) -> bool {
        filter.is_none_or(|f| f == self)
    }
}

/// The inclusive day range a calendar request covers.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct MonthWindow {
    pub first: chrono::NaiveDate,
    pub last: chrono::NaiveDate,
}

impl MonthWindow {
    /// Parses a `YYYY-MM` month into its first and last day.
    ///
    /// The last day is found by stepping back one day from the first of the
    /// following month, so February and the leap years look after themselves.
    pub fn parse(month: &str) -> DomainResult<Self> {
        let bad = || DomainError::invalid("month", "must be in YYYY-MM form");
        let (year, mon) = month.trim().split_once('-').ok_or_else(bad)?;
        let year: i32 = year.parse().map_err(|_| bad())?;
        let mon: u32 = mon.parse().map_err(|_| bad())?;
        if !(1..=12).contains(&mon) {
            return Err(DomainError::invalid("month", "must be between 01 and 12"));
        }
        let first = chrono::NaiveDate::from_ymd_opt(year, mon, 1).ok_or_else(bad)?;
        let next_first = if mon == 12 {
            chrono::NaiveDate::from_ymd_opt(year + 1, 1, 1)
        } else {
            chrono::NaiveDate::from_ymd_opt(year, mon + 1, 1)
        }
        .ok_or_else(bad)?;
        Ok(Self {
            first,
            last: next_first.pred_opt().ok_or_else(bad)?,
        })
    }

    /// The month `today` falls in.
    #[must_use]
    pub fn containing(today: chrono::NaiveDate) -> Self {
        use chrono::Datelike;
        // from_ymd_opt with day 1 of an existing date's month cannot fail, but
        // the fallback keeps this total rather than panicking on a bad clock.
        let first =
            chrono::NaiveDate::from_ymd_opt(today.year(), today.month(), 1).unwrap_or(today);
        let next_first = if today.month() == 12 {
            chrono::NaiveDate::from_ymd_opt(today.year() + 1, 1, 1)
        } else {
            chrono::NaiveDate::from_ymd_opt(today.year(), today.month() + 1, 1)
        };
        let last = next_first.and_then(|d| d.pred_opt()).unwrap_or(today);
        Self { first, last }
    }

    /// Everything, for the `allDates` gantt view.
    ///
    /// Bounded rather than unbounded so the same range predicate serves both
    /// paths and the query planner still sees a range.
    #[must_use]
    pub fn all_time() -> Self {
        Self {
            first: chrono::NaiveDate::from_ymd_opt(1900, 1, 1).unwrap_or_default(),
            last: chrono::NaiveDate::from_ymd_opt(2999, 12, 31).unwrap_or_default(),
        }
    }
}

/// The span a call-forward item actually occupies on the calendar.
///
/// Actual dates win over estimates, and either end falls back to the other end
/// when only one date is set -- an item with just an estimated start still
/// renders, as a single-day bar, rather than disappearing.
///
/// Returns `None` when all four dates are absent: there is nowhere to draw it.
#[must_use]
pub fn effective_range(
    actual_start: Option<chrono::NaiveDate>,
    est_start: Option<chrono::NaiveDate>,
    actual_finish: Option<chrono::NaiveDate>,
    est_finish: Option<chrono::NaiveDate>,
) -> Option<(chrono::NaiveDate, chrono::NaiveDate)> {
    let start = actual_start
        .or(est_start)
        .or(actual_finish)
        .or(est_finish)?;
    let finish = actual_finish
        .or(est_finish)
        .or(actual_start)
        .or(est_start)?;
    Some((start, finish))
}

/// A bar on the calendar.
#[derive(Debug, Clone)]
pub struct CalendarEvent {
    /// `job-12`, `claim-7`, `task-9`. Unique across kinds, which share a
    /// numeric id space between `jobs` and `call_forward`.
    pub id: String,
    pub kind: EventKind,
    pub title: String,
    pub start: chrono::NaiveDate,
    pub end: chrono::NaiveDate,
    pub est_start: Option<chrono::NaiveDate>,
    pub est_finish: Option<chrono::NaiveDate>,
    pub actual_start: Option<chrono::NaiveDate>,
    pub actual_finish: Option<chrono::NaiveDate>,
    pub job_id: JobId,
    pub job_name: String,
    pub job_number: String,
    pub job_address: Option<String>,
    pub supervisor_id: Option<UserId>,
    pub supervisor_name: Option<String>,
    pub status: String,
    pub supplier_trade: Option<String>,
    /// Where the UI navigates when the bar is clicked.
    pub url: String,
}

/// The filter dropdowns the calendar offers.
#[derive(Debug, Clone)]
pub struct CalendarFilterOptions {
    pub jobs: Vec<CalendarFilterJob>,
    pub supervisors: Vec<CalendarFilterSupervisor>,
}

#[derive(Debug, Clone)]
pub struct CalendarFilterJob {
    pub id: JobId,
    pub name: String,
    pub job_number: String,
    pub address: Option<String>,
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub struct CalendarFilterSupervisor {
    pub id: UserId,
    pub name: String,
}

#[cfg(test)]
mod tests {
    use super::*;
    use chrono::NaiveDate;

    fn d(y: i32, m: u32, day: u32) -> NaiveDate {
        NaiveDate::from_ymd_opt(y, m, day).unwrap()
    }

    #[test]
    fn a_month_resolves_to_its_first_and_last_day() {
        let w = MonthWindow::parse("2026-03").unwrap();
        assert_eq!(w.first, d(2026, 3, 1));
        assert_eq!(w.last, d(2026, 3, 31));
    }

    #[test]
    fn thirty_day_months_end_on_the_thirtieth() {
        assert_eq!(MonthWindow::parse("2026-04").unwrap().last, d(2026, 4, 30));
        assert_eq!(MonthWindow::parse("2026-11").unwrap().last, d(2026, 11, 30));
    }

    #[test]
    fn february_is_correct_in_both_common_and_leap_years() {
        assert_eq!(MonthWindow::parse("2026-02").unwrap().last, d(2026, 2, 28));
        assert_eq!(MonthWindow::parse("2028-02").unwrap().last, d(2028, 2, 29));
        // 1900 was not a leap year; 2000 was.
        assert_eq!(MonthWindow::parse("1900-02").unwrap().last, d(1900, 2, 28));
        assert_eq!(MonthWindow::parse("2000-02").unwrap().last, d(2000, 2, 29));
    }

    #[test]
    fn december_rolls_into_the_next_year_without_losing_a_day() {
        let w = MonthWindow::parse("2026-12").unwrap();
        assert_eq!(w.first, d(2026, 12, 1));
        assert_eq!(w.last, d(2026, 12, 31));
    }

    #[test]
    fn a_malformed_month_is_a_field_level_error() {
        for bad in ["2026", "2026-13", "2026-00", "not-a-month", "", "2026-1x"] {
            let e = MonthWindow::parse(bad).unwrap_err();
            assert!(
                matches!(e, DomainError::Invalid { field, .. } if field == "month"),
                "{bad} gave {e:?}"
            );
        }
    }

    #[test]
    fn a_single_digit_month_is_accepted_as_well_as_a_padded_one() {
        // The UI sends "2026-03"; tolerating "2026-3" costs nothing.
        assert_eq!(
            MonthWindow::parse("2026-3").unwrap(),
            MonthWindow::parse("2026-03").unwrap()
        );
    }

    #[test]
    fn the_containing_month_matches_parsing_it() {
        assert_eq!(
            MonthWindow::containing(d(2026, 2, 17)),
            MonthWindow::parse("2026-02").unwrap()
        );
        assert_eq!(
            MonthWindow::containing(d(2026, 12, 31)),
            MonthWindow::parse("2026-12").unwrap()
        );
    }

    #[test]
    fn actual_dates_win_over_estimates() {
        let r = effective_range(
            Some(d(2026, 3, 2)),
            Some(d(2026, 3, 1)),
            Some(d(2026, 3, 9)),
            Some(d(2026, 3, 8)),
        );
        assert_eq!(r, Some((d(2026, 3, 2), d(2026, 3, 9))));
    }

    #[test]
    fn an_item_with_only_a_start_renders_as_a_single_day() {
        // Otherwise a task with an estimated start and no finish would vanish.
        let r = effective_range(None, Some(d(2026, 3, 1)), None, None);
        assert_eq!(r, Some((d(2026, 3, 1), d(2026, 3, 1))));
    }

    #[test]
    fn an_item_with_only_a_finish_renders_as_a_single_day() {
        let r = effective_range(None, None, None, Some(d(2026, 3, 8)));
        assert_eq!(r, Some((d(2026, 3, 8), d(2026, 3, 8))));
    }

    #[test]
    fn an_item_with_no_dates_at_all_is_not_drawn() {
        assert_eq!(effective_range(None, None, None, None), None);
    }

    #[test]
    fn a_finish_before_its_start_is_reported_as_given() {
        // The calendar draws what the data says; silently swapping the ends
        // would hide a data-entry mistake the user needs to see.
        let r = effective_range(Some(d(2026, 3, 9)), None, Some(d(2026, 3, 2)), None);
        assert_eq!(r, Some((d(2026, 3, 9), d(2026, 3, 2))));
    }

    #[test]
    fn the_type_filter_accepts_the_four_documented_values() {
        assert_eq!(EventKind::parse_filter(None).unwrap(), None);
        assert_eq!(EventKind::parse_filter(Some("all")).unwrap(), None);
        assert_eq!(EventKind::parse_filter(Some("")).unwrap(), None);
        assert_eq!(
            EventKind::parse_filter(Some("job")).unwrap(),
            Some(EventKind::Job)
        );
        assert_eq!(
            EventKind::parse_filter(Some(" task ")).unwrap(),
            Some(EventKind::Task)
        );
        assert!(EventKind::parse_filter(Some("JOB")).is_err());
        assert!(EventKind::parse_filter(Some("everything")).is_err());
    }

    #[test]
    fn no_filter_includes_every_kind() {
        for k in [EventKind::Job, EventKind::Claim, EventKind::Task] {
            assert!(k.included_by(None));
        }
        assert!(EventKind::Job.included_by(Some(EventKind::Job)));
        assert!(!EventKind::Job.included_by(Some(EventKind::Task)));
    }
}
