//! Dashboard response shapes.

use pmk_domain::dashboard::{ActionItem, DashboardStats, DelaySeverity};
use pmk_ports::repository::{
    DashboardCallForward, DashboardDiaryEntry, DashboardJob, UpcomingClaim,
};
use serde::Serialize;

#[derive(Debug, Serialize)]
#[serde(rename_all = "camelCase")]
pub struct DashboardStatsDto {
    pub total_jobs: i64,
    pub active_jobs: i64,
    pub completed_jobs: i64,
    pub open_call_forwards: i64,
    pub overdue_call_forwards: i64,
    /// Null for everyone but a manager.
    pub total_users: Option<i64>,
    pub recent_diary_entries: i64,
}

impl From<DashboardStats> for DashboardStatsDto {
    fn from(s: DashboardStats) -> Self {
        Self {
            total_jobs: s.total_jobs,
            active_jobs: s.active_jobs,
            completed_jobs: s.completed_jobs,
            open_call_forwards: s.open_call_forwards,
            overdue_call_forwards: s.overdue_call_forwards,
            total_users: s.total_users,
            recent_diary_entries: s.recent_diary_entries,
        }
    }
}

#[derive(Debug, Serialize)]
#[serde(rename_all = "camelCase")]
pub struct DashboardJobDto {
    pub id: i32,
    pub job_number: Option<String>,
    pub address: Option<String>,
    pub name: Option<String>,
    pub status: String,
    pub start_date: Option<chrono::NaiveDate>,
    pub end_date: Option<chrono::NaiveDate>,
}

impl From<DashboardJob> for DashboardJobDto {
    fn from(j: DashboardJob) -> Self {
        Self {
            id: j.id.get(),
            job_number: j.job_number,
            address: j.address,
            name: j.name,
            status: j.status,
            start_date: j.start_date,
            end_date: j.end_date,
        }
    }
}

#[derive(Debug, Serialize)]
#[serde(rename_all = "camelCase")]
pub struct DashboardCallForwardDto {
    pub id: i32,
    pub job_id: i32,
    pub job_number: Option<String>,
    pub job_address: Option<String>,
    pub title: String,
    pub item_type: String,
    pub est_start: Option<chrono::NaiveDate>,
    pub est_finish: Option<chrono::NaiveDate>,
    pub status: String,
}

impl From<DashboardCallForward> for DashboardCallForwardDto {
    fn from(c: DashboardCallForward) -> Self {
        Self {
            id: c.id,
            job_id: c.job_id.get(),
            job_number: c.job_number,
            job_address: c.job_address,
            title: c.title,
            item_type: c.item_type,
            est_start: c.est_start,
            est_finish: c.est_finish,
            status: c.status,
        }
    }
}

#[derive(Debug, Serialize)]
#[serde(rename_all = "camelCase")]
pub struct DashboardDiaryDto {
    pub id: i32,
    pub job_id: i32,
    pub job_number: Option<String>,
    pub job_address: Option<String>,
    pub date: chrono::NaiveDate,
    pub work_completed: Option<String>,
    pub author_name: Option<String>,
}

impl From<DashboardDiaryEntry> for DashboardDiaryDto {
    fn from(e: DashboardDiaryEntry) -> Self {
        Self {
            id: e.id.get(),
            job_id: e.job_id.get(),
            job_number: e.job_number,
            job_address: e.job_address,
            date: e.date,
            work_completed: e.work_completed,
            author_name: e.author_name,
        }
    }
}

#[derive(Debug, Serialize)]
#[serde(rename_all = "camelCase")]
pub struct UpcomingClaimDto {
    pub id: i32,
    pub job_id: i32,
    pub job_name: Option<String>,
    pub job_address: Option<String>,
    pub job_number: Option<String>,
    pub title: String,
    pub est_start: Option<chrono::NaiveDate>,
    pub est_finish: Option<chrono::NaiveDate>,
    pub actual_start: Option<chrono::NaiveDate>,
    pub actual_finish: Option<chrono::NaiveDate>,
    pub status: String,
    pub notes: Option<String>,
}

impl From<UpcomingClaim> for UpcomingClaimDto {
    fn from(c: UpcomingClaim) -> Self {
        Self {
            id: c.id,
            job_id: c.job_id.get(),
            job_name: c.job_name,
            job_address: c.job_address,
            job_number: c.job_number,
            title: c.title,
            est_start: c.est_start,
            est_finish: c.est_finish,
            actual_start: c.actual_start,
            actual_finish: c.actual_finish,
            status: c.status,
            notes: c.notes,
        }
    }
}

#[derive(Debug, Serialize)]
#[serde(rename_all = "camelCase")]
pub struct DelaySeverityDto {
    pub label: &'static str,
    pub count: i64,
    pub avg_days: i64,
    pub max_days: i64,
}

impl From<DelaySeverity> for DelaySeverityDto {
    fn from(d: DelaySeverity) -> Self {
        Self {
            label: d.label,
            count: d.count,
            avg_days: d.avg_days,
            max_days: d.max_days,
        }
    }
}

#[derive(Debug, Serialize)]
#[serde(rename_all = "camelCase")]
pub struct ActionItemDto {
    pub note_id: i32,
    pub diary_entry_id: i32,
    pub category: String,
    pub content: String,
    pub action_status: String,
    pub action_raised_by: Option<i32>,
    pub created_at: chrono::DateTime<chrono::Utc>,
    pub entry_date: chrono::NaiveDate,
    pub job_id: i32,
    pub job_name: Option<String>,
    pub job_number: Option<String>,
    pub job_address: Option<String>,
    pub author_id: Option<i32>,
    pub author_name: Option<String>,
    pub latest_comment_content: Option<String>,
    pub latest_comment_author: Option<String>,
    pub latest_comment_at: Option<chrono::DateTime<chrono::Utc>>,
}

impl From<ActionItem> for ActionItemDto {
    fn from(a: ActionItem) -> Self {
        Self {
            note_id: a.note_id,
            diary_entry_id: a.diary_entry_id,
            category: a.category,
            content: a.content,
            action_status: a.action_status,
            action_raised_by: a.action_raised_by.map(|u| u.get()),
            created_at: a.created_at,
            entry_date: a.entry_date,
            job_id: a.job_id.get(),
            job_name: a.job_name,
            job_number: a.job_number,
            job_address: a.job_address,
            author_id: a.author_id.map(|u| u.get()),
            author_name: a.author_name,
            latest_comment_content: a.latest_comment_content,
            latest_comment_author: a.latest_comment_author,
            latest_comment_at: a.latest_comment_at,
        }
    }
}
