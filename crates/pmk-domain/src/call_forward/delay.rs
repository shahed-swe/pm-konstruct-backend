//! Call-forward delay engine — domain-rules R1.
//!
//! A line-for-line port of `computeDelayInfo()` from the legacy
//! `services/callForwardService.ts`. The precedence order is load-bearing and
//! two orderings are easy to get wrong:
//!
//!   * `on_hold` beats `delayed` — an on-hold item months overdue is `on_hold`.
//!   * `delayed` beats the stored status — an item the user marked `in_progress`
//!     past its `est_finish` reports `delayed`.
//!
//! `today` is supplied by the caller (from the `Clock` port, in the company's
//! configured timezone) and never read from the system clock. The legacy code
//! compared a *local* midnight against *UTC*-parsed dates, which only agrees
//! with itself on a UTC server; measured over all 1,673 production rows,
//! Australia/Melbourne (+10 and +11) changes nothing and UTC−5 changes 13 rows.
//! Pinning the timezone in config is the deliberate divergence.
//!
//! Acceptance: `docs/contract/delay-vectors.json`, all 1,673 vectors.

use chrono::NaiveDate;

/// The stored `call_forward.status` column.
#[derive(Debug, Clone, Copy, PartialEq, Eq, serde::Serialize, serde::Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum CfStatus {
    NotStarted,
    InProgress,
    Completed,
    OnHold,
}

impl CfStatus {
    /// Parses the stored text. Unrecognised values fall through to the
    /// date-based inference in rules 7–8, matching the legacy behaviour: its
    /// `if` chain simply ran out of matches.
    #[must_use]
    pub fn parse(s: &str) -> Option<Self> {
        match s {
            "not_started" => Some(Self::NotStarted),
            "in_progress" => Some(Self::InProgress),
            "completed" => Some(Self::Completed),
            "on_hold" => Some(Self::OnHold),
            _ => None,
        }
    }

    #[must_use]
    pub const fn as_str(self) -> &'static str {
        match self {
            Self::NotStarted => "not_started",
            Self::InProgress => "in_progress",
            Self::Completed => "completed",
            Self::OnHold => "on_hold",
        }
    }
}

/// The *computed* status returned to clients. Distinct from [`CfStatus`]:
/// `Delayed` is never stored, only derived.
#[derive(Debug, Clone, Copy, PartialEq, Eq, serde::Serialize, serde::Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum DelayStatus {
    NotStarted,
    InProgress,
    Completed,
    OnHold,
    Delayed,
}

impl DelayStatus {
    #[must_use]
    pub const fn as_str(self) -> &'static str {
        match self {
            Self::NotStarted => "not_started",
            Self::InProgress => "in_progress",
            Self::Completed => "completed",
            Self::OnHold => "on_hold",
            Self::Delayed => "delayed",
        }
    }
}

/// Just the fields the engine reads. Keeping this narrow is what lets the rule
/// be unit-tested without constructing a full aggregate or touching a database.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct DelayInput {
    pub status: Option<CfStatus>,
    pub est_start: Option<NaiveDate>,
    pub est_finish: Option<NaiveDate>,
    pub actual_start: Option<NaiveDate>,
    pub actual_finish: Option<NaiveDate>,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq, serde::Serialize)]
pub struct DelayInfo {
    pub delay_status: DelayStatus,
    /// Whole days past `est_finish`. Only ever `Some` when
    /// `delay_status == Delayed`.
    pub delay_days: Option<i64>,
}

impl DelayInfo {
    const fn plain(status: DelayStatus) -> Self {
        Self { delay_status: status, delay_days: None }
    }
}

/// Computes the delay status for one item as of `today`.
///
/// See the module docs for the precedence rules. Numbered comments map to the
/// table in domain-rules.md R1.
#[must_use]
pub fn compute_delay_info(item: &DelayInput, today: NaiveDate) -> DelayInfo {
    // 1. On hold wins outright, even when overdue.
    if item.status == Some(CfStatus::OnHold) {
        return DelayInfo::plain(DelayStatus::OnHold);
    }

    // 2. Complete by stored status OR by a recorded actual finish.
    if item.status == Some(CfStatus::Completed) || item.actual_finish.is_some() {
        return DelayInfo::plain(DelayStatus::Completed);
    }

    // 3. Past the estimated finish with no actual finish: overdue. This
    //    deliberately overrides whatever the user set the status to.
    if let Some(est_finish) = item.est_finish {
        if today > est_finish {
            // Legacy: Math.round((today - estFinish) / 86_400_000). Both sides
            // are midnight-aligned, so the division is exact and the rounding
            // never actually rounds -- but the day count must match exactly.
            let delay_days = (today - est_finish).num_days();
            return DelayInfo { delay_status: DelayStatus::Delayed, delay_days: Some(delay_days) };
        }
    }

    // 4. An actual start is authoritative: older rows can still carry the
    //    default `not_started` after someone recorded when work began.
    if item.actual_start.is_some() {
        return DelayInfo::plain(DelayStatus::InProgress);
    }

    // 5-6. Respect the stored status while not yet overdue.
    match item.status {
        Some(CfStatus::InProgress) => return DelayInfo::plain(DelayStatus::InProgress),
        Some(CfStatus::NotStarted) => return DelayInfo::plain(DelayStatus::NotStarted),
        _ => {}
    }

    // 7. Fall back to date-based inference.
    if let Some(est_start) = item.est_start {
        if today >= est_start {
            return DelayInfo::plain(DelayStatus::InProgress);
        }
    }

    // 8. Nothing else applies.
    DelayInfo::plain(DelayStatus::NotStarted)
}

#[cfg(test)]
mod tests {
    use super::*;

    fn d(s: &str) -> NaiveDate {
        s.parse().expect("test date literal")
    }

    const TODAY: &str = "2026-09-21";

    fn input(status: Option<CfStatus>) -> DelayInput {
        DelayInput {
            status,
            est_start: None,
            est_finish: None,
            actual_start: None,
            actual_finish: None,
        }
    }

    fn run(i: &DelayInput) -> DelayInfo {
        compute_delay_info(i, d(TODAY))
    }

    // -- rule 1: on_hold beats everything -----------------------------------
    #[test]
    fn on_hold_wins_even_when_massively_overdue() {
        let i = DelayInput { est_finish: Some(d("2026-05-08")), ..input(Some(CfStatus::OnHold)) };
        assert_eq!(run(&i), DelayInfo::plain(DelayStatus::OnHold));
    }

    // -- rule 2: completed, both routes -------------------------------------
    #[test]
    fn completed_by_stored_status() {
        assert_eq!(
            run(&input(Some(CfStatus::Completed))),
            DelayInfo::plain(DelayStatus::Completed)
        );
    }

    #[test]
    fn completed_by_actual_finish_despite_not_started_status() {
        let i = DelayInput {
            actual_finish: Some(d("2026-09-01")),
            ..input(Some(CfStatus::NotStarted))
        };
        assert_eq!(run(&i), DelayInfo::plain(DelayStatus::Completed));
    }

    #[test]
    fn completed_beats_overdue() {
        let i = DelayInput {
            est_finish: Some(d("2026-01-01")),
            actual_finish: Some(d("2026-09-01")),
            ..input(Some(CfStatus::NotStarted))
        };
        assert_eq!(run(&i), DelayInfo::plain(DelayStatus::Completed));
    }

    // -- rule 3: delayed, and its boundaries --------------------------------
    #[test]
    fn overdue_by_one_day() {
        let i = DelayInput {
            est_finish: Some(d("2026-09-20")),
            ..input(Some(CfStatus::NotStarted))
        };
        assert_eq!(
            run(&i),
            DelayInfo { delay_status: DelayStatus::Delayed, delay_days: Some(1) }
        );
    }

    #[test]
    fn est_finish_today_is_not_yet_delayed() {
        // Strictly `today > est_finish`, so today itself is not overdue.
        let i = DelayInput {
            est_finish: Some(d(TODAY)),
            ..input(Some(CfStatus::NotStarted))
        };
        assert_eq!(run(&i), DelayInfo::plain(DelayStatus::NotStarted));
    }

    #[test]
    fn overdue_overrides_manual_in_progress() {
        let i = DelayInput {
            est_finish: Some(d("2026-09-16")),
            ..input(Some(CfStatus::InProgress))
        };
        assert_eq!(
            run(&i),
            DelayInfo { delay_status: DelayStatus::Delayed, delay_days: Some(5) }
        );
    }

    #[test]
    fn overdue_overrides_an_actual_start() {
        let i = DelayInput {
            est_finish: Some(d("2026-09-18")),
            actual_start: Some(d("2026-09-01")),
            ..input(Some(CfStatus::NotStarted))
        };
        assert_eq!(
            run(&i),
            DelayInfo { delay_status: DelayStatus::Delayed, delay_days: Some(3) }
        );
    }

    #[test]
    fn matches_the_production_delay_day_range() {
        // The production export spans 3..=136 days overdue.
        for days in [3_i64, 50, 136] {
            let i = DelayInput {
                est_finish: Some(d(TODAY) - chrono::Duration::days(days)),
                ..input(Some(CfStatus::NotStarted))
            };
            assert_eq!(run(&i).delay_days, Some(days));
        }
    }

    // -- rule 4 --------------------------------------------------------------
    #[test]
    fn actual_start_implies_in_progress() {
        let i = DelayInput {
            actual_start: Some(d("2026-09-01")),
            ..input(Some(CfStatus::NotStarted))
        };
        assert_eq!(run(&i), DelayInfo::plain(DelayStatus::InProgress));
    }

    // -- rules 5-6 -----------------------------------------------------------
    #[test]
    fn stored_status_respected_when_not_overdue() {
        assert_eq!(
            run(&input(Some(CfStatus::InProgress))),
            DelayInfo::plain(DelayStatus::InProgress)
        );
        assert_eq!(
            run(&input(Some(CfStatus::NotStarted))),
            DelayInfo::plain(DelayStatus::NotStarted)
        );
    }

    // -- rule 7: date inference for unrecognised statuses --------------------
    #[test]
    fn unknown_status_with_est_start_reached_infers_in_progress() {
        let i = DelayInput { est_start: Some(d("2026-09-01")), ..input(None) };
        assert_eq!(run(&i), DelayInfo::plain(DelayStatus::InProgress));
    }

    #[test]
    fn unknown_status_with_est_start_today_infers_in_progress() {
        // `today >= est_start`, inclusive.
        let i = DelayInput { est_start: Some(d(TODAY)), ..input(None) };
        assert_eq!(run(&i), DelayInfo::plain(DelayStatus::InProgress));
    }

    #[test]
    fn unknown_status_with_future_est_start_is_not_started() {
        let i = DelayInput { est_start: Some(d("2026-12-01")), ..input(None) };
        assert_eq!(run(&i), DelayInfo::plain(DelayStatus::NotStarted));
    }

    // -- rule 8 --------------------------------------------------------------
    #[test]
    fn all_dates_null_and_unknown_status_is_not_started() {
        assert_eq!(run(&input(None)), DelayInfo::plain(DelayStatus::NotStarted));
    }

    // -- invariant -----------------------------------------------------------
    #[test]
    fn delay_days_is_set_only_for_delayed() {
        let cases = [
            input(Some(CfStatus::OnHold)),
            input(Some(CfStatus::Completed)),
            input(Some(CfStatus::InProgress)),
            input(Some(CfStatus::NotStarted)),
            input(None),
        ];
        for c in cases {
            let r = run(&c);
            assert_eq!(
                r.delay_days.is_some(),
                r.delay_status == DelayStatus::Delayed,
                "delay_days must be Some only for Delayed, got {r:?}"
            );
        }
    }

    #[test]
    fn status_round_trips_through_parse() {
        for s in ["not_started", "in_progress", "completed", "on_hold"] {
            assert_eq!(CfStatus::parse(s).map(CfStatus::as_str), Some(s));
        }
        assert_eq!(CfStatus::parse("delayed"), None, "delayed is computed, never stored");
        assert_eq!(CfStatus::parse(""), None);
    }
}
