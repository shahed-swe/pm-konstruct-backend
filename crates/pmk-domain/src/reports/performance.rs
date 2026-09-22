//! Supervisor performance scoring.
//!
//! This report grades people, so every number in it is worth pinning down. The
//! weights and the fallbacks below are the legacy ones; the tests exist so a
//! change to someone's score is a deliberate act rather than a side effect.
//!
//! The `i64 -> f64` casts here are allowed deliberately: every value crossing
//! one is a percentage in 0..=100 or a count of jobs, tasks or days on one
//! company's book. Reaching 2^53 of any of them is not a scenario this
//! software has, and the alternative -- fixed-point arithmetic for a
//! percentage -- would obscure the formula the report is meant to document.
#![allow(clippy::cast_precision_loss, clippy::cast_possible_truncation)]

use chrono::Datelike;

/// How much each component contributes to the composite score.
///
/// They sum to 1.0. On-time completion dominates because it is the outcome the
/// client sees; being on track is weighted least because it double-counts
/// delay, which already suppresses the completion rate.
const W_COMPLETION: f64 = 0.35;
const W_START: f64 = 0.25;
const W_DIARY: f64 = 0.25;
const W_ON_TRACK: f64 = 0.15;

/// Stands in for a rate that has no data behind it.
///
/// A supervisor with no finished tasks is scored as average on that component
/// rather than as zero -- otherwise someone newly assigned would rank below
/// someone performing badly, purely for having done nothing yet.
const NO_DATA_SCORE: f64 = 50.0;

/// The weighted composite, 0–100.
///
/// `None` for a rate means no data, which scores [`NO_DATA_SCORE`]. A
/// supervisor with no tasks at all is fully on track, not zero: there is
/// nothing to be late on.
#[must_use]
pub fn performance_score(
    on_time_completion: Option<i64>,
    on_time_start: Option<i64>,
    diary_compliance: Option<i64>,
    total_tasks: i64,
    delayed_tasks: i64,
) -> i64 {
    let comp = on_time_completion.map_or(NO_DATA_SCORE, |v| v as f64);
    let start = on_time_start.map_or(NO_DATA_SCORE, |v| v as f64);
    let diary = diary_compliance.map_or(NO_DATA_SCORE, |v| v as f64);
    let on_track = if total_tasks > 0 {
        percentage(total_tasks - delayed_tasks, total_tasks) as f64
    } else {
        100.0
    };

    round_half_up(W_COMPLETION * comp + W_START * start + W_DIARY * diary + W_ON_TRACK * on_track)
}

/// `part` as a whole-number percentage of `whole`.
///
/// Zero when there is nothing to divide by, which is the caller's cue that the
/// figure is meaningless rather than genuinely zero.
#[must_use]
pub fn percentage(part: i64, whole: i64) -> i64 {
    if whole <= 0 {
        return 0;
    }
    round_half_up(part as f64 * 100.0 / whole as f64)
}

/// Diary compliance, capped at 100.
///
/// More than one entry can be filed on a working day, so the raw ratio can
/// exceed 100% -- which would read as a supervisor being 140% compliant.
#[must_use]
pub fn diary_compliance(actual: i64, expected: i64) -> Option<i64> {
    if expected <= 0 {
        return None;
    }
    Some(percentage(actual, expected).min(100))
}

/// Working days from `start` to `end`, both inclusive, Monday to Friday.
///
/// Public holidays are not excluded: the legacy did not know about them, and
/// guessing a calendar for one Australian state would make the number wrong in
/// a new way rather than a known one.
#[must_use]
pub fn working_days(start: chrono::NaiveDate, end: chrono::NaiveDate) -> i64 {
    if start > end {
        return 0;
    }
    // Counted arithmetically rather than by stepping day by day: a job running
    // for three years would otherwise be 1,000 loop iterations per supervisor
    // per request.
    let total_days = (end - start).num_days() + 1;
    let whole_weeks = total_days / 7;
    let mut count = whole_weeks * 5;

    // Walk only the remainder, which is at most six days.
    let remainder_start = start + chrono::Duration::days(whole_weeks * 7);
    let mut day = remainder_start;
    while day <= end {
        if !matches!(day.weekday(), chrono::Weekday::Sat | chrono::Weekday::Sun) {
            count += 1;
        }
        day += chrono::Duration::days(1);
    }
    count
}

/// The mean of `total` over `count`, in whole days.
///
/// `None` when there is nothing to average, which the report prints as a dash
/// rather than as zero -- "no delays" and "delays averaging zero days" are
/// different statements.
#[must_use]
pub fn mean_days(total: i64, count: i64) -> Option<i64> {
    (count > 0).then(|| round_half_up(total as f64 / count as f64))
}

/// The mean of a list of measurements, to one decimal place.
///
/// Used for the average temperature, which is the only float the reports
/// average. `None` for an empty list.
#[must_use]
pub fn mean_1dp(values: &[f64]) -> Option<f64> {
    if values.is_empty() {
        return None;
    }
    let sum: f64 = values.iter().sum();
    Some(crate::reports::round_1dp(sum / values.len() as f64))
}

/// JavaScript's `Math.round`: half goes toward positive infinity.
fn round_half_up(v: f64) -> i64 {
    (v + 0.5).floor() as i64
}

#[cfg(test)]
mod tests {
    use super::*;
    use chrono::NaiveDate;

    fn d(y: i32, m: u32, day: u32) -> NaiveDate {
        NaiveDate::from_ymd_opt(y, m, day).unwrap()
    }

    #[test]
    fn a_perfect_supervisor_scores_a_hundred() {
        assert_eq!(
            performance_score(Some(100), Some(100), Some(100), 10, 0),
            100
        );
    }

    #[test]
    fn the_worst_case_scores_zero() {
        // Every rate at zero, and every task delayed.
        assert_eq!(performance_score(Some(0), Some(0), Some(0), 10, 10), 0);
    }

    #[test]
    fn missing_rates_score_as_average_not_as_zero() {
        // Someone newly assigned must not rank below someone doing badly.
        let newcomer = performance_score(None, None, None, 0, 0);
        let poor = performance_score(Some(0), Some(0), Some(0), 10, 10);
        assert!(newcomer > poor, "{newcomer} should beat {poor}");
        // 0.35*50 + 0.25*50 + 0.25*50 + 0.15*100 = 57.5 -> 58
        assert_eq!(newcomer, 58);
    }

    #[test]
    fn a_supervisor_with_no_tasks_is_fully_on_track() {
        // Nothing to be late on, so the on-track component is 100.
        assert_eq!(performance_score(Some(0), Some(0), Some(0), 0, 0), 15);
    }

    #[test]
    fn the_weights_sum_to_one() {
        let sum = W_COMPLETION + W_START + W_DIARY + W_ON_TRACK;
        assert!((sum - 1.0).abs() < f64::EPSILON, "{sum}");
    }

    #[test]
    fn each_component_moves_the_score_by_its_weight() {
        let base = performance_score(Some(0), Some(0), Some(0), 10, 10);
        assert_eq!(
            performance_score(Some(100), Some(0), Some(0), 10, 10) - base,
            35
        );
        assert_eq!(
            performance_score(Some(0), Some(100), Some(0), 10, 10) - base,
            25
        );
        assert_eq!(
            performance_score(Some(0), Some(0), Some(100), 10, 10) - base,
            25
        );
        assert_eq!(
            performance_score(Some(0), Some(0), Some(0), 10, 0) - base,
            15
        );
    }

    #[test]
    fn a_mean_in_days_rounds_half_up() {
        assert_eq!(mean_days(10, 4), Some(3)); // 2.5 -> 3
        assert_eq!(mean_days(9, 4), Some(2)); // 2.25 -> 2
        assert_eq!(mean_days(0, 3), Some(0));
    }

    #[test]
    fn a_mean_of_nothing_is_absent_not_zero() {
        // "No delays" and "delays averaging zero days" are different claims.
        assert_eq!(mean_days(0, 0), None);
        assert_eq!(mean_1dp(&[]), None);
    }

    #[allow(clippy::float_cmp)]
    #[test]
    fn a_float_mean_is_printed_to_one_decimal_place() {
        assert_eq!(mean_1dp(&[14.5, 18.0, 26.0, 24.0, 19.5]), Some(20.4));
        assert_eq!(mean_1dp(&[1.0]), Some(1.0));
    }

    #[test]
    fn percentages_round_half_up() {
        assert_eq!(percentage(1, 3), 33);
        assert_eq!(percentage(2, 3), 67);
        assert_eq!(percentage(1, 8), 13);
        assert_eq!(percentage(1, 2), 50);
        assert_eq!(percentage(0, 10), 0);
        assert_eq!(percentage(10, 10), 100);
    }

    #[test]
    fn a_percentage_of_nothing_is_zero_rather_than_a_division_error() {
        assert_eq!(percentage(5, 0), 0);
        assert_eq!(percentage(0, 0), 0);
    }

    #[test]
    fn diary_compliance_cannot_exceed_a_hundred() {
        // Two entries on one working day would otherwise read as 200%.
        assert_eq!(diary_compliance(20, 10), Some(100));
        assert_eq!(diary_compliance(5, 10), Some(50));
    }

    #[test]
    fn diary_compliance_is_absent_when_no_days_were_expected() {
        assert_eq!(diary_compliance(0, 0), None);
        assert_eq!(diary_compliance(3, 0), None);
    }

    #[test]
    fn a_full_week_holds_five_working_days() {
        // Monday 2 March 2026 through Sunday 8 March.
        assert_eq!(working_days(d(2026, 3, 2), d(2026, 3, 8)), 5);
    }

    #[test]
    fn both_ends_are_counted() {
        // Monday to Monday is two working days plus a weekend.
        assert_eq!(working_days(d(2026, 3, 2), d(2026, 3, 9)), 6);
        // A single weekday is one day.
        assert_eq!(working_days(d(2026, 3, 4), d(2026, 3, 4)), 1);
    }

    #[test]
    fn a_weekend_holds_none() {
        // Saturday 7 March to Sunday 8 March 2026.
        assert_eq!(working_days(d(2026, 3, 7), d(2026, 3, 8)), 0);
        assert_eq!(working_days(d(2026, 3, 7), d(2026, 3, 7)), 0);
    }

    #[test]
    fn an_inverted_range_holds_none() {
        assert_eq!(working_days(d(2026, 3, 10), d(2026, 3, 1)), 0);
    }

    #[test]
    fn the_arithmetic_shortcut_agrees_with_counting_day_by_day() {
        // The implementation counts whole weeks rather than looping, so this
        // checks it against the obvious version over every start weekday and a
        // range of lengths.
        for start_offset in 0..7 {
            let start = d(2026, 3, 1) + chrono::Duration::days(start_offset);
            for len in 0..40 {
                let end = start + chrono::Duration::days(len);
                let mut expected = 0;
                let mut day = start;
                while day <= end {
                    if !matches!(day.weekday(), chrono::Weekday::Sat | chrono::Weekday::Sun) {
                        expected += 1;
                    }
                    day += chrono::Duration::days(1);
                }
                assert_eq!(working_days(start, end), expected, "{start} .. {end}");
            }
        }
    }

    #[test]
    fn a_long_range_is_still_correct() {
        // A year from Thursday 1 Jan 2026 to Thursday 31 Dec 2026: 261.
        assert_eq!(working_days(d(2026, 1, 1), d(2026, 12, 31)), 261);
    }
}
