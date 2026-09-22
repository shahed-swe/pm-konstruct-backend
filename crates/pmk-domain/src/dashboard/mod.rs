//! Dashboard aggregates.
//!
//! The counting and bucketing live here rather than in SQL so they can be
//! tested against fixed inputs. The queries fetch rows; the shapes below decide
//! what they mean.

use crate::ids::{JobId, UserId};

/// The headline counters.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct DashboardStats {
    pub total_jobs: i64,
    pub active_jobs: i64,
    pub completed_jobs: i64,
    pub open_call_forwards: i64,
    pub overdue_call_forwards: i64,
    /// Managers only. `None` for every other role, because a supervisor has no
    /// business knowing the company's headcount.
    pub total_users: Option<i64>,
    pub recent_diary_entries: i64,
}

/// How far past its estimated finish an overdue item is.
///
/// Four fixed buckets, in this order, always present even when empty -- the
/// chart renders four bars and a missing bucket would shift the others.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum DelayBucket {
    UnderOneWeek,
    OneToTwoWeeks,
    TwoToFourWeeks,
    OverFourWeeks,
}

impl DelayBucket {
    /// The label the chart prints. En dashes, as in the legacy UI.
    #[must_use]
    pub const fn label(self) -> &'static str {
        match self {
            Self::UnderOneWeek => "< 1 week",
            Self::OneToTwoWeeks => "1–2 weeks",
            Self::TwoToFourWeeks => "2–4 weeks",
            Self::OverFourWeeks => "> 4 weeks",
        }
    }

    /// Every bucket, in chart order.
    #[must_use]
    pub const fn all() -> [Self; 4] {
        [
            Self::UnderOneWeek,
            Self::OneToTwoWeeks,
            Self::TwoToFourWeeks,
            Self::OverFourWeeks,
        ]
    }

    /// Boundaries are inclusive at the top: exactly 7 days late is "< 1 week".
    ///
    /// That reads oddly, and it is what the legacy code does -- `days <= 7`.
    /// The labels describe the bucket loosely; the thresholds are what the
    /// dashboard has always drawn, so they are preserved.
    #[must_use]
    pub const fn of(days_late: i64) -> Self {
        if days_late <= 7 {
            Self::UnderOneWeek
        } else if days_late <= 14 {
            Self::OneToTwoWeeks
        } else if days_late <= 28 {
            Self::TwoToFourWeeks
        } else {
            Self::OverFourWeeks
        }
    }
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub struct DelaySeverity {
    pub label: &'static str,
    pub count: i64,
    pub avg_days: i64,
    pub max_days: i64,
}

/// Buckets overdue items by how late they are.
///
/// `days_late` is measured against the company's today, so an item due
/// yesterday is 1 day late. Items not yet due do not reach here.
#[must_use]
pub fn delay_severity(days_late: &[i64]) -> Vec<DelaySeverity> {
    let mut counts = [0i64; 4];
    let mut sums = [0i64; 4];
    let mut maxes = [0i64; 4];

    for &days in days_late {
        let i = match DelayBucket::of(days) {
            DelayBucket::UnderOneWeek => 0,
            DelayBucket::OneToTwoWeeks => 1,
            DelayBucket::TwoToFourWeeks => 2,
            DelayBucket::OverFourWeeks => 3,
        };
        counts[i] += 1;
        sums[i] += days;
        maxes[i] = maxes[i].max(days);
    }

    DelayBucket::all()
        .into_iter()
        .enumerate()
        .map(|(i, bucket)| DelaySeverity {
            label: bucket.label(),
            count: counts[i],
            // Rounded to whole days, as the legacy `Math.round` did. An empty
            // bucket reports 0 rather than dividing by zero.
            avg_days: if counts[i] > 0 {
                round_div(sums[i], counts[i])
            } else {
                0
            },
            max_days: maxes[i],
        })
        .collect()
}

/// Integer division rounding half away from zero, matching `Math.round` for
/// the non-negative values this ever sees.
const fn round_div(sum: i64, count: i64) -> i64 {
    (sum * 2 + count) / (count * 2)
}

/// Which jobs a dashboard query may count.
///
/// `All` is a manager or office user: every job in the company. `Only` is a
/// supervisor, restricted to the jobs R3 makes visible to them -- their
/// assignments union the jobs they are primary supervisor of. An empty `Only`
/// means the answer is zero everywhere, without running a query.
#[derive(Debug, Clone, PartialEq, Eq)]
pub enum JobScope {
    All,
    Only(Vec<JobId>),
}

impl JobScope {
    /// True when the caller can see nothing, so every count is zero.
    #[must_use]
    pub fn is_empty(&self) -> bool {
        matches!(self, Self::Only(ids) if ids.is_empty())
    }

    #[must_use]
    pub fn ids(&self) -> Option<Vec<i32>> {
        match self {
            Self::All => None,
            Self::Only(ids) => Some(ids.iter().map(|i| i.get()).collect()),
        }
    }
}

/// A note awaiting action, as the dashboard lists it.
#[derive(Debug, Clone)]
pub struct ActionItem {
    pub note_id: i32,
    pub diary_entry_id: i32,
    pub category: String,
    pub content: String,
    pub action_status: String,
    pub action_raised_by: Option<UserId>,
    pub created_at: chrono::DateTime<chrono::Utc>,
    pub entry_date: chrono::NaiveDate,
    pub job_id: JobId,
    pub job_name: Option<String>,
    pub job_number: Option<String>,
    pub job_address: Option<String>,
    pub author_id: Option<UserId>,
    pub author_name: Option<String>,
    pub latest_comment_content: Option<String>,
    pub latest_comment_author: Option<String>,
    pub latest_comment_at: Option<chrono::DateTime<chrono::Utc>>,
}

pub mod calendar;

#[cfg(test)]
mod tests;
