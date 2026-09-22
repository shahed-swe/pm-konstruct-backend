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

    /// The manager's name, which the jobs page shows rather than the id.
    ///
    /// Null when the job has no manager, or when the manager's user row has
    /// been deleted. Sent by the legacy under this name.
    #[serde(skip_serializing_if = "Option::is_none")]
    pub manager_name: Option<String>,

    /// The *first* assigned supervisor's name -- not necessarily the primary
    /// one. That is what the legacy sent here, and the jobs page's supervisor
    /// stack picks the primary out of `supervisors` itself.
    #[serde(skip_serializing_if = "Option::is_none")]
    pub supervisor_name: Option<String>,

    /// Everyone assigned, oldest assignment first.
    ///
    /// Present on the list and the detail response, absent on a create or
    /// update reply -- which is also what the legacy did, and why the jobs
    /// page refetches the list after saving rather than patching it in place.
    #[serde(skip_serializing_if = "Option::is_none")]
    pub supervisors: Option<Vec<JobSupervisorDto>>,
}

/// A supervisor on a job, as the list carries them.
#[derive(Debug, Serialize)]
#[serde(rename_all = "camelCase")]
pub struct JobSupervisorDto {
    /// The **user's** id. Named `id` because that is the field the existing
    /// client reads; `JobAssignment` in the domain calls it `user_id` for
    /// the reason recorded there.
    pub id: i32,
    pub name: String,
    pub is_primary: bool,
}

impl From<pmk_ports::repository::AssignedSupervisor> for JobSupervisorDto {
    fn from(s: pmk_ports::repository::AssignedSupervisor) -> Self {
        Self {
            id: s.user_id.get(),
            name: s.name,
            is_primary: s.is_primary,
        }
    }
}

impl From<pmk_app::jobs::JobView> for JobDto {
    fn from(v: pmk_app::jobs::JobView) -> Self {
        Self {
            manager_name: v.manager_name,
            supervisor_name: v.supervisor_name,
            supervisors: Some(v.supervisors.into_iter().map(Into::into).collect()),
            ..Self::from(v.job)
        }
    }
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
            manager_name: None,
            supervisor_name: None,
            supervisors: None,
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

/// A partial job update.
///
/// `PUT /jobs/{id}` has always been a *partial* update: the legacy service
/// spread whatever keys arrived onto the row and left the rest alone. Several
/// screens rely on it -- archiving from the row menu sends only `status`, the
/// notes panel autosaves only `description`, and clearing a stale cloud link
/// sends only `dropboxPath`. Requiring the whole job would make each of those
/// a read-modify-write that can clobber a concurrent edit.
///
/// An absent key leaves the field alone. An explicit `null` clears it, which
/// is why the nullable fields are `Option<Option<T>>` rather than `Option<T>`
/// -- without the distinction there is no way to erase a client email.
#[derive(Debug, Default, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct JobPatchRequest {
    pub name: Option<String>,
    pub job_number: Option<String>,
    pub client: Option<String>,
    pub address: Option<String>,
    pub status: Option<String>,

    #[serde(default, deserialize_with = "double_option")]
    pub client_number: Option<Option<String>>,
    #[serde(default, deserialize_with = "double_option")]
    pub client_email: Option<Option<String>>,
    #[serde(default, deserialize_with = "double_option")]
    pub start_date: Option<Option<chrono::NaiveDate>>,
    #[serde(default, deserialize_with = "double_option")]
    pub end_date: Option<Option<chrono::NaiveDate>>,
    #[serde(default, deserialize_with = "double_option")]
    pub manager_id: Option<Option<i32>>,
    #[serde(default, deserialize_with = "double_option")]
    pub supervisor_id: Option<Option<i32>>,
    #[serde(default, deserialize_with = "double_option")]
    pub dropbox_path: Option<Option<String>>,
    #[serde(default, deserialize_with = "double_option")]
    pub description: Option<Option<String>>,
    #[serde(default, deserialize_with = "double_option")]
    pub contact2_name: Option<Option<String>>,
    #[serde(default, deserialize_with = "double_option")]
    pub contact2_phone: Option<Option<String>>,
    #[serde(default, deserialize_with = "double_option")]
    pub contact2_email: Option<Option<String>>,
}

/// Distinguishes "key absent" from "key present and null".
///
/// Without it serde collapses both to `None`, and an update that clears a
/// field is indistinguishable from one that does not mention it.
fn double_option<'de, D, T>(de: D) -> Result<Option<Option<T>>, D::Error>
where
    D: serde::Deserializer<'de>,
    T: Deserialize<'de>,
{
    Deserialize::deserialize(de).map(Some)
}

impl JobPatchRequest {
    /// Applies the patch to the job as it stands.
    pub fn apply(self, current: &Job) -> Result<JobInput, pmk_domain::DomainError> {
        let status = match self.status.as_deref() {
            None => Some(current.status),
            Some("") => None,
            Some(s) => Some(JobStatus::parse(s).ok_or_else(|| {
                pmk_domain::DomainError::invalid(
                    "status",
                    "must be one of active, completed, archived, on_hold",
                )
            })?),
        };

        Ok(JobInput {
            name: self.name.unwrap_or_else(|| current.name.clone()),
            job_number: self
                .job_number
                .unwrap_or_else(|| current.job_number.clone()),
            client: self.client.unwrap_or_else(|| current.client.clone()),
            address: self.address.unwrap_or_else(|| current.address.clone()),
            status,
            client_number: self
                .client_number
                .unwrap_or_else(|| current.client_number.clone()),
            client_email: self
                .client_email
                .unwrap_or_else(|| current.client_email.clone()),
            start_date: self.start_date.unwrap_or(current.start_date),
            end_date: self.end_date.unwrap_or(current.end_date),
            manager_id: self
                .manager_id
                .map_or(current.manager_id, |v| v.map(UserId)),
            supervisor_id: self
                .supervisor_id
                .map_or(current.supervisor_id, |v| v.map(UserId)),
            dropbox_path: self
                .dropbox_path
                .unwrap_or_else(|| current.dropbox_path.clone()),
            description: self
                .description
                .unwrap_or_else(|| current.description.clone()),
            contact2_name: self
                .contact2_name
                .unwrap_or_else(|| current.contact2_name.clone()),
            contact2_phone: self
                .contact2_phone
                .unwrap_or_else(|| current.contact2_phone.clone()),
            contact2_email: self
                .contact2_email
                .unwrap_or_else(|| current.contact2_email.clone()),
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

/// A partial update to a task.
///
/// The task list toggles a status or renames a line without sending the
/// rest, as the legacy allowed. An explicit `null` clears the notes.
#[derive(Debug, Default, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct JobTaskPatchRequest {
    pub title: Option<String>,
    pub status: Option<String>,
    #[serde(default, deserialize_with = "double_option")]
    pub notes: Option<Option<String>>,
    pub sort_order: Option<i32>,
}

impl JobTaskPatchRequest {
    #[must_use]
    pub fn apply(
        self,
        current: &pmk_ports::repository::JobTask,
    ) -> pmk_ports::repository::JobTaskInput {
        pmk_ports::repository::JobTaskInput {
            title: self.title.unwrap_or_else(|| current.title.clone()),
            status: Some(self.status.unwrap_or_else(|| current.status.clone())),
            notes: self.notes.unwrap_or_else(|| current.notes.clone()),
            sort_order: Some(self.sort_order.unwrap_or(current.sort_order)),
        }
    }
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

/// A cloud-storage link on a job.
///
/// Stored in `job_dropbox_folders`, but provider-agnostic: the client's
/// requirement is "any cloud based server. Google, Dropbox etc."
#[derive(Debug, Serialize)]
#[serde(rename_all = "camelCase")]
pub struct JobLinkDto {
    pub id: i32,
    pub job_id: i32,
    pub label: String,
    pub url: String,
    pub sort_order: i32,
    pub created_at: chrono::DateTime<chrono::Utc>,
}

impl From<pmk_domain::job::JobLink> for JobLinkDto {
    fn from(l: pmk_domain::job::JobLink) -> Self {
        Self {
            id: l.id,
            job_id: l.job_id.get(),
            label: l.label,
            url: l.url,
            sort_order: l.sort_order,
            created_at: l.created_at,
        }
    }
}

#[derive(Debug, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct JobLinkRequest {
    pub label: String,
    /// Accepts `path` too, which is what the legacy column is called.
    #[serde(alias = "path")]
    pub url: String,
    pub sort_order: Option<i32>,
}

impl From<JobLinkRequest> for pmk_domain::job::JobLinkInput {
    fn from(r: JobLinkRequest) -> Self {
        Self {
            label: r.label,
            url: r.url,
            sort_order: r.sort_order,
        }
    }
}

/// `DELETE /jobs/{id}?purge=true` deletes for real instead of archiving.
///
/// Opt-in rather than the default because the cascade reaches diary entries,
/// media, call-forward items and scheduler allocations.
#[derive(Debug, Deserialize, Default)]
#[serde(rename_all = "camelCase")]
pub struct DeleteJobQuery {
    #[serde(default)]
    pub purge: bool,
}
