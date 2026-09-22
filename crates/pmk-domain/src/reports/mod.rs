//! Report classifications.
//!
//! Every threshold here is a number the reports print, so a change is visible
//! to the client. They live in the domain with tests rather than inline in SQL
//! so a shift cannot pass unnoticed.

use crate::error::{DomainError, DomainResult};

/// How serious an overdue item is, on the reports.
///
/// Note these are **not** the dashboard's buckets. The dashboard groups by
/// elapsed time (`< 1 week`, `1–2 weeks`, `2–4 weeks`, `> 4 weeks`); the
/// reports grade by severity at 3, 7 and 14 days. The two have always
/// disagreed, and unifying them would change numbers the client reads.
#[derive(Debug, Clone, Copy, PartialEq, Eq, PartialOrd, Ord)]
pub enum Severity {
    Low,
    Medium,
    High,
    Critical,
}

impl Severity {
    #[must_use]
    pub const fn as_str(self) -> &'static str {
        match self {
            Self::Low => "low",
            Self::Medium => "medium",
            Self::High => "high",
            Self::Critical => "critical",
        }
    }

    /// Strictly greater than each threshold: exactly 14 days late is `High`,
    /// not `Critical`.
    #[must_use]
    pub const fn of(delay_days: i64) -> Self {
        if delay_days > 14 {
            Self::Critical
        } else if delay_days > 7 {
            Self::High
        } else if delay_days > 3 {
            Self::Medium
        } else {
            Self::Low
        }
    }
}

/// A job's headline state on the progress report.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum JobHealth {
    OnTrack,
    AtRisk,
    Delayed,
}

impl JobHealth {
    #[must_use]
    pub const fn as_str(self) -> &'static str {
        match self {
            Self::OnTrack => "on_track",
            Self::AtRisk => "at_risk",
            Self::Delayed => "delayed",
        }
    }
}

/// A job is `Delayed` if anything on it is overdue, and `OnTrack` otherwise.
///
/// `AtRisk` exists in the type because the legacy report declares it, but the
/// legacy code can never produce it: its ternary reduces to
/// `delayed > 0 ? "delayed" : "on_track"`, with both remaining branches
/// returning the same value. Reproduced exactly -- inventing a threshold for
/// `AtRisk` would change a status the client already reads, and there is no
/// evidence of what it was meant to be.
#[must_use]
pub const fn job_health(delayed_count: i64) -> JobHealth {
    if delayed_count > 0 {
        JobHealth::Delayed
    } else {
        JobHealth::OnTrack
    }
}

/// Completed items as a whole-number percentage of all non-header items.
///
/// A job with no items is 0%, not 100%: nothing has been done, and showing a
/// finished bar for an empty programme would be worse than showing an empty
/// one.
#[must_use]
pub fn completion_pct(completed: i64, total: i64) -> i64 {
    if total <= 0 {
        return 0;
    }
    // Rounds half away from zero, matching the legacy `Math.round`.
    (completed * 200 + total) / (total * 2)
}

/// An inclusive date range for a report filter.
#[derive(Debug, Clone, Copy, Default, PartialEq, Eq)]
pub struct ReportRange {
    pub from: Option<chrono::NaiveDate>,
    pub to: Option<chrono::NaiveDate>,
}

impl ReportRange {
    /// Rejects an inverted range rather than silently returning nothing.
    pub fn validate(&self) -> DomainResult<()> {
        if let (Some(from), Some(to)) = (self.from, self.to) {
            if from > to {
                return Err(DomainError::invalid(
                    "dateTo",
                    "must be on or after the from date",
                ));
            }
        }
        Ok(())
    }

    #[must_use]
    pub fn contains(&self, date: chrono::NaiveDate) -> bool {
        self.from.is_none_or(|f| date >= f) && self.to.is_none_or(|t| date <= t)
    }
}

pub mod performance;

// ── weather impact ──────────────────────────────────────────────────────────

/// Rainfall above this, in millimetres, stops work on its own.
const HEAVY_RAINFALL_MM: f64 = 10.0;

/// Weather words that mean a day was not workable.
const ADVERSE_CONDITIONS: [&str; 5] = ["rain", "storm", "thunder", "shower", "drizzle"];

/// Words in the issues field that attribute a problem to the weather.
const WEATHER_ISSUE_WORDS: [&str; 4] = ["weather", "rain", "storm", "flood"];

/// Why a day counted as weather-affected, or `None` if it did not.
///
/// The three tests are checked in this order, and only the first match is
/// reported: heavy rainfall is a measurement, an adverse condition is an
/// observation, and a weather-related issue is someone's note. The most
/// objective evidence wins.
#[must_use]
pub fn weather_impact(
    condition: Option<&str>,
    rainfall_mm: Option<f64>,
    issues: Option<&str>,
) -> Option<String> {
    let rainfall = rainfall_mm.unwrap_or(0.0);
    if rainfall > HEAVY_RAINFALL_MM {
        return Some(format!("Heavy rainfall: {rainfall}mm"));
    }

    let condition_text = condition.unwrap_or("").to_lowercase();
    if ADVERSE_CONDITIONS
        .iter()
        .any(|w| condition_text.contains(w))
    {
        // Echoed as written, not lowercased: the report shows what was
        // recorded.
        return Some(format!("Adverse weather: {}", condition.unwrap_or("")));
    }

    let issues_text = issues.unwrap_or("").to_lowercase();
    if WEATHER_ISSUE_WORDS.iter().any(|w| issues_text.contains(w)) {
        return Some("Weather-related issues reported".to_string());
    }

    None
}

/// Did it rain at all?
///
/// A looser test than [`weather_impact`], and deliberately so: the rainy-day
/// count answers "how often did it rain", which any measurable rainfall
/// satisfies, while the impact count answers "how often did weather stop
/// work".
#[must_use]
pub fn was_rainy(condition: Option<&str>, rainfall_mm: Option<f64>) -> bool {
    condition.unwrap_or("").to_lowercase().contains("rain") || rainfall_mm.unwrap_or(0.0) > 0.0
}

/// Rounds to one decimal place, as the weather totals are printed.
///
/// Half rounds *up* (toward positive infinity), not away from zero: that is
/// what JavaScript's `Math.round` does, and the average temperature can be
/// negative. `f64::round` would turn an average of -2.25 into -2.3 where the
/// legacy printed -2.2.
#[must_use]
pub fn round_1dp(v: f64) -> f64 {
    (v * 10.0 + 0.5).floor() / 10.0
}

#[cfg(test)]
mod tests {
    use super::*;
    use chrono::NaiveDate;

    fn d(y: i32, m: u32, day: u32) -> NaiveDate {
        NaiveDate::from_ymd_opt(y, m, day).unwrap()
    }

    #[test]
    fn heavy_rainfall_alone_marks_a_day_as_affected() {
        assert_eq!(
            weather_impact(Some("Sunny"), Some(10.1), None).as_deref(),
            Some("Heavy rainfall: 10.1mm")
        );
        // The threshold is strictly greater: exactly 10mm is not heavy.
        assert_eq!(weather_impact(Some("Sunny"), Some(10.0), None), None);
    }

    #[test]
    fn an_adverse_condition_marks_a_day_even_with_no_measurement() {
        for c in ["Rain", "Thunderstorm", "Showers", "Light drizzle", "STORM"] {
            assert!(
                weather_impact(Some(c), None, None).is_some(),
                "{c} should count"
            );
        }
        assert_eq!(weather_impact(Some("Sunny"), None, None), None);
    }

    #[test]
    fn the_condition_is_echoed_as_recorded_not_lowercased() {
        assert_eq!(
            weather_impact(Some("Heavy Rain"), None, None).as_deref(),
            Some("Adverse weather: Heavy Rain")
        );
    }

    #[test]
    fn a_weather_related_issue_counts_when_nothing_else_does() {
        assert_eq!(
            weather_impact(Some("Sunny"), Some(0.0), Some("Flooding on site")).as_deref(),
            Some("Weather-related issues reported")
        );
        assert_eq!(
            weather_impact(Some("Sunny"), Some(0.0), Some("Crane broke down")),
            None
        );
    }

    #[test]
    fn the_most_objective_evidence_is_the_one_reported() {
        // All three tests match; the measurement is what the report shows.
        let r = weather_impact(Some("Storm"), Some(40.0), Some("weather delay"));
        assert_eq!(r.as_deref(), Some("Heavy rainfall: 40mm"));
    }

    #[test]
    fn a_day_with_nothing_recorded_is_not_affected() {
        assert_eq!(weather_impact(None, None, None), None);
    }

    #[test]
    fn rainy_is_looser_than_affected() {
        // 2mm of rain is a rainy day but does not stop work.
        assert!(was_rainy(Some("Sunny"), Some(2.0)));
        assert_eq!(weather_impact(Some("Sunny"), Some(2.0), None), None);
        assert!(was_rainy(Some("Light rain"), None));
        assert!(!was_rainy(Some("Sunny"), Some(0.0)));
        assert!(!was_rainy(None, None));
    }

    // Exact comparison is the point here: the contract is that the function
    // yields precisely the f64 nearest to the printed value, and an epsilon
    // would hide a rounding rule that had drifted.
    #[allow(clippy::float_cmp)]
    #[test]
    fn totals_are_printed_to_one_decimal_place() {
        assert_eq!(round_1dp(12.34), 12.3);
        assert_eq!(round_1dp(12.35), 12.4);
        assert_eq!(round_1dp(0.0), 0.0);
        // Half rounds toward positive infinity, as Math.round does. f64::round
        // would give -2.3 here, and the average temperature can be negative.
        assert_eq!(round_1dp(-2.25), -2.2);
        assert_eq!(round_1dp(-2.26), -2.3);
        assert_eq!(round_1dp(2.25), 2.3);
    }

    #[test]
    fn severity_thresholds_are_strictly_greater_than() {
        assert_eq!(Severity::of(0), Severity::Low);
        assert_eq!(Severity::of(3), Severity::Low);
        assert_eq!(Severity::of(4), Severity::Medium);
        assert_eq!(Severity::of(7), Severity::Medium);
        assert_eq!(Severity::of(8), Severity::High);
        assert_eq!(Severity::of(14), Severity::High);
        assert_eq!(Severity::of(15), Severity::Critical);
        assert_eq!(Severity::of(365), Severity::Critical);
    }

    #[test]
    fn report_severity_does_not_match_the_dashboard_buckets() {
        // Deliberate: the dashboard groups 8..=14 days as "1-2 weeks" while
        // the reports call the same delay "high". Unifying them would change
        // numbers the client already reads.
        use crate::dashboard::DelayBucket;
        assert_eq!(DelayBucket::of(10), DelayBucket::OneToTwoWeeks);
        assert_eq!(Severity::of(10), Severity::High);
        assert_eq!(DelayBucket::of(20), DelayBucket::TwoToFourWeeks);
        assert_eq!(Severity::of(20), Severity::Critical);
    }

    #[test]
    fn severity_orders_from_low_to_critical() {
        assert!(Severity::Low < Severity::Medium);
        assert!(Severity::Medium < Severity::High);
        assert!(Severity::High < Severity::Critical);
    }

    #[test]
    fn any_overdue_item_makes_a_job_delayed() {
        assert_eq!(job_health(1), JobHealth::Delayed);
        assert_eq!(job_health(99), JobHealth::Delayed);
    }

    #[test]
    fn a_job_with_nothing_overdue_is_on_track() {
        assert_eq!(job_health(0), JobHealth::OnTrack);
    }

    #[test]
    fn at_risk_is_declared_but_never_produced() {
        // The legacy ternary reduces to delayed-or-on-track; reproducing that
        // exactly matters more than making the third state reachable, because
        // there is no evidence of what threshold it was meant to have.
        for n in 0..50 {
            assert_ne!(job_health(n), JobHealth::AtRisk);
        }
    }

    #[test]
    fn completion_is_a_rounded_percentage() {
        assert_eq!(completion_pct(0, 10), 0);
        assert_eq!(completion_pct(5, 10), 50);
        assert_eq!(completion_pct(10, 10), 100);
        // 1/3 = 33.33 -> 33
        assert_eq!(completion_pct(1, 3), 33);
        // 2/3 = 66.67 -> 67
        assert_eq!(completion_pct(2, 3), 67);
        // 1/8 = 12.5 -> 13, rounding half away from zero
        assert_eq!(completion_pct(1, 8), 13);
    }

    #[test]
    fn an_empty_programme_is_zero_percent_not_a_hundred() {
        assert_eq!(completion_pct(0, 0), 0);
    }

    #[test]
    fn an_inverted_report_range_is_rejected() {
        let r = ReportRange {
            from: Some(d(2026, 3, 10)),
            to: Some(d(2026, 3, 1)),
        };
        let e = r.validate().unwrap_err();
        assert!(matches!(e, DomainError::Invalid { field, .. } if field == "dateTo"));
    }

    #[test]
    fn a_single_day_range_is_valid() {
        let day = d(2026, 3, 4);
        let r = ReportRange {
            from: Some(day),
            to: Some(day),
        };
        assert!(r.validate().is_ok());
        assert!(r.contains(day));
    }

    #[test]
    fn an_open_ended_range_is_valid_at_either_end() {
        let from_only = ReportRange {
            from: Some(d(2026, 3, 4)),
            to: None,
        };
        assert!(from_only.validate().is_ok());
        assert!(from_only.contains(d(2030, 1, 1)));
        assert!(!from_only.contains(d(2020, 1, 1)));

        let to_only = ReportRange {
            from: None,
            to: Some(d(2026, 3, 4)),
        };
        assert!(to_only.validate().is_ok());
        assert!(to_only.contains(d(2020, 1, 1)));
        assert!(!to_only.contains(d(2030, 1, 1)));
    }

    #[test]
    fn an_unbounded_range_contains_everything() {
        let r = ReportRange::default();
        assert!(r.validate().is_ok());
        assert!(r.contains(d(1990, 1, 1)));
        assert!(r.contains(d(2099, 1, 1)));
    }

    #[test]
    fn range_bounds_are_inclusive() {
        let r = ReportRange {
            from: Some(d(2026, 3, 1)),
            to: Some(d(2026, 3, 31)),
        };
        assert!(r.contains(d(2026, 3, 1)));
        assert!(r.contains(d(2026, 3, 31)));
        assert!(!r.contains(d(2026, 2, 28)));
        assert!(!r.contains(d(2026, 4, 1)));
    }
}
