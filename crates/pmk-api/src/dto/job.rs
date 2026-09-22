//! Job DTOs. camelCase to match the legacy contract and the golden fixtures.

use pmk_domain::ids::UserId;
use pmk_domain::job::{Job, JobAssignment, JobInput, JobStatus};
use pmk_ports::repository::JobTask;
use serde::{Deserialize, Serialize};

#[derive(Debug, Serialize)]
#[serde(rename_all = "camelCase")]
pub struct JobDto {
    pub id: i32,
    pub company_id: i32,
    pub name: String,
    pub job_number: String,
    pub client: String,
    pub client_number: Option<String>,
    pub client_email: Option<String>,
    pub address: String,
    pub status: String,
    pub start_date: Option<chrono::NaiveDate>,
    pub end_date: Option<chrono::NaiveDate>,
    pub manager_id: Option<i32>,
    pub supervisor_id: Option<i32>,
    pub dropbox_path: Option<String>,
    pub description: Option<String>,
    pub contact2_name: Option<String>,
    pub contact2_phone: Option<String>,
    pub contact2_email: Option<String>,
    pub created_at: chrono::DateTime<chrono::Utc>,
    pub updated_at: chrono::DateTime<chrono::Utc>,
}

impl From<Job> for JobDto {
    fn from(j: Job) -> Self {
        Self {
            id: j.id.get(),
            company_id: j.company_id.get(),
            name: j.name,
            job_number: j.job_number,
            client: j.client,
            client_number: j.client_number,
            client_email: j.client_email,
            address: j.address,
            status: j.status.as_str().to_string(),
            start_date: j.start_date,
            end_date: j.end_date,
            manager_id: j.manager_id.map(UserId::get),
            supervisor_id: j.supervisor_id.map(UserId::get),
            dropbox_path: j.dropbox_path,
            description: j.description,
            contact2_name: j.contact2_name,
            contact2_phone: j.contact2_phone,
            contact2_email: j.contact2_email,
            created_at: j.created_at,
            updated_at: j.updated_at,
        }
    }
}

#[derive(Debug, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct JobUpsertRequest {
    pub name: String,
    pub job_number: String,
    pub client: String,
    pub client_number: Option<String>,
    pub client_email: Option<String>,
    pub address: String,
    pub status: Option<String>,
    pub start_date: Option<chrono::NaiveDate>,
    pub end_date: Option<chrono::NaiveDate>,
    pub manager_id: Option<i32>,
    pub supervisor_id: Option<i32>,
    pub dropbox_path: Option<String>,
    pub description: Option<String>,
    pub contact2_name: Option<String>,
    pub contact2_phone: Option<String>,
    pub contact2_email: Option<String>,
}

impl JobUpsertRequest {
    /// An unrecognised status is rejected here rather than reaching
    /// `jobs_status_check` and surfacing as a constraint error.
    pub fn into_input(self) -> Result<JobInput, pmk_domain::DomainError> {
        let status = match self.status.as_deref() {
            None | Some("") => None,
            Some(s) => Some(JobStatus::parse(s).ok_or_else(|| {
                pmk_domain::DomainError::invalid(
                    "status",
                    "must be one of active, completed, archived, on_hold",
                )
            })?),
        };
        Ok(JobInput {
            name: self.name,
            job_number: self.job_number,
            client: self.client,
            client_number: self.client_number,
            client_email: self.client_email,
            address: self.address,
            status,
            start_date: self.start_date,
            end_date: self.end_date,
            manager_id: self.manager_id.map(UserId),
            supervisor_id: self.supervisor_id.map(UserId),
            dropbox_path: self.dropbox_path,
            description: self.description,
            contact2_name: self.contact2_name,
            contact2_phone: self.contact2_phone,
            contact2_email: self.contact2_email,
        })
    }
}

#[derive(Debug, Serialize)]
#[serde(rename_all = "camelCase")]
pub struct AssignmentDto {
    /// The assigned user's id. The legacy response called this `id` while it
    /// held a user id; both are emitted so existing clients keep working.
    pub id: i32,
    pub user_id: i32,
    pub name: String,
    pub role: String,
    pub is_primary: bool,
    pub assigned_at: chrono::DateTime<chrono::Utc>,
}

impl From<JobAssignment> for AssignmentDto {
    fn from(a: JobAssignment) -> Self {
        Self {
            id: a.user_id.get(),
            user_id: a.user_id.get(),
            name: a.name,
            role: a.role.as_str().to_string(),
            is_primary: a.is_primary,
            assigned_at: a.assigned_at,
        }
    }
}

#[derive(Debug, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct AddAssignmentRequest {
    pub user_id: i32,
    #[serde(default)]
    pub is_primary: bool,
}

#[derive(Debug, Serialize)]
#[serde(rename_all = "camelCase")]
pub struct JobTaskDto {
    pub id: i32,
    pub job_id: i32,
    pub title: String,
    pub status: String,
    pub notes: Option<String>,
    pub sort_order: i32,
    pub created_at: chrono::DateTime<chrono::Utc>,
    pub updated_at: chrono::DateTime<chrono::Utc>,
}

impl From<JobTask> for JobTaskDto {
    fn from(t: JobTask) -> Self {
        Self {
            id: t.id,
            job_id: t.job_id.get(),
            title: t.title,
            status: t.status,
            notes: t.notes,
            sort_order: t.sort_order,
            created_at: t.created_at,
            updated_at: t.updated_at,
        }
    }
}

#[derive(Debug, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct JobTaskRequest {
    pub title: String,
    pub status: Option<String>,
    pub notes: Option<String>,
    pub sort_order: Option<i32>,
}

#[derive(Debug, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct TaskNotesRequest {
    pub notes: Option<String>,
}

#[derive(Debug, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct JobListQuery {
    pub status: Option<String>,
    pub supervisor_id: Option<i32>,
    pub search: Option<String>,
}
