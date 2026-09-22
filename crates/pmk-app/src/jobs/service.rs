use std::sync::Arc;

use pmk_domain::ids::{JobId, UserId};
use pmk_domain::job::{Job, JobAssignment, JobInput, JobLink, JobLinkInput};
use pmk_domain::DomainError;
use pmk_ports::repository::{
    JobFilter, JobLinkRepository, JobRepository, JobTask, JobTaskInput, JobTaskRepository,
};

use crate::identity::SessionUser;
use crate::{AppError, AppResult};

pub struct JobService {
    jobs: Arc<dyn JobRepository>,
    tasks: Arc<dyn JobTaskRepository>,
    links: Arc<dyn JobLinkRepository>,
}

impl std::fmt::Debug for JobService {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        f.debug_struct("JobService").finish_non_exhaustive()
    }
}

impl JobService {
    #[must_use]
    pub fn new(
        jobs: Arc<dyn JobRepository>,
        tasks: Arc<dyn JobTaskRepository>,
        links: Arc<dyn JobLinkRepository>,
    ) -> Self {
        Self { jobs, tasks, links }
    }

    pub async fn list(&self, s: &SessionUser, filter: JobFilter) -> AppResult<Vec<Job>> {
        Ok(self
            .jobs
            .list_visible(
                s.principal.scope(),
                s.user.id,
                s.user.role.sees_all_company_jobs(),
                &filter,
            )
            .await?)
    }

    /// Returns 404 rather than 403 when a job exists but is not visible: a
    /// supervisor must not be able to probe which job numbers exist.
    pub async fn get(&self, s: &SessionUser, id: JobId) -> AppResult<Job> {
        self.jobs
            .find(
                s.principal.scope(),
                s.user.id,
                s.user.role.sees_all_company_jobs(),
                id,
            )
            .await?
            .ok_or_else(|| AppError::Domain(DomainError::not_found("Job")))
    }

    pub async fn create(&self, s: &SessionUser, input: JobInput) -> AppResult<Job> {
        let input = input.normalised();
        input.validate().map_err(AppError::Domain)?;
        Ok(self.jobs.create(s.principal.scope(), &input).await?)
    }

    pub async fn update(&self, s: &SessionUser, id: JobId, input: JobInput) -> AppResult<Job> {
        // Visibility is checked first so a supervisor cannot blind-write a job
        // they cannot read.
        self.get(s, id).await?;
        let input = input.normalised();
        input.validate().map_err(AppError::Domain)?;
        self.jobs
            .update(s.principal.scope(), id, &input)
            .await?
            .ok_or_else(|| AppError::Domain(DomainError::not_found("Job")))
    }

    pub async fn delete(&self, s: &SessionUser, id: JobId) -> AppResult<()> {
        self.get(s, id).await?;
        if self.jobs.delete(s.principal.scope(), id).await? {
            Ok(())
        } else {
            Err(AppError::Domain(DomainError::not_found("Job")))
        }
    }

    pub async fn assignments(&self, s: &SessionUser, id: JobId) -> AppResult<Vec<JobAssignment>> {
        self.get(s, id).await?;
        Ok(self.jobs.assignments(s.principal.scope(), id).await?)
    }

    pub async fn add_assignment(
        &self,
        s: &SessionUser,
        id: JobId,
        user: UserId,
        primary: bool,
    ) -> AppResult<Vec<JobAssignment>> {
        self.get(s, id).await?;
        Ok(self
            .jobs
            .add_assignment(s.principal.scope(), id, user, primary)
            .await?)
    }

    pub async fn remove_assignment(
        &self,
        s: &SessionUser,
        id: JobId,
        user: UserId,
    ) -> AppResult<Vec<JobAssignment>> {
        self.get(s, id).await?;
        Ok(self
            .jobs
            .remove_assignment(s.principal.scope(), id, user)
            .await?)
    }

    pub async fn set_primary(
        &self,
        s: &SessionUser,
        id: JobId,
        user: UserId,
    ) -> AppResult<Vec<JobAssignment>> {
        self.get(s, id).await?;
        Ok(self
            .jobs
            .set_primary_assignment(s.principal.scope(), id, user)
            .await?)
    }

    // ── tasks ───────────────────────────────────────────────────────────────

    pub async fn list_tasks(&self, s: &SessionUser, job: JobId) -> AppResult<Vec<JobTask>> {
        self.get(s, job).await?;
        Ok(self.tasks.list(s.principal.scope(), job).await?)
    }

    pub async fn create_task(
        &self,
        s: &SessionUser,
        job: JobId,
        input: JobTaskInput,
    ) -> AppResult<JobTask> {
        self.get(s, job).await?;
        validate_task(&input)?;
        Ok(self.tasks.create(s.principal.scope(), job, &input).await?)
    }

    pub async fn update_task(
        &self,
        s: &SessionUser,
        id: i32,
        input: JobTaskInput,
    ) -> AppResult<JobTask> {
        validate_task(&input)?;
        // No explicit job check: `job_tasks` is reachable only through its
        // job's RLS policy, so a cross-tenant id simply matches nothing.
        self.tasks
            .update(s.principal.scope(), id, &input)
            .await?
            .ok_or_else(|| AppError::Domain(DomainError::not_found("Task")))
    }

    pub async fn update_task_notes(
        &self,
        s: &SessionUser,
        id: i32,
        notes: Option<String>,
    ) -> AppResult<JobTask> {
        self.tasks
            .update_notes(s.principal.scope(), id, notes.as_deref())
            .await?
            .ok_or_else(|| AppError::Domain(DomainError::not_found("Task")))
    }

    pub async fn delete_task(&self, s: &SessionUser, id: i32) -> AppResult<()> {
        if self.tasks.delete(s.principal.scope(), id).await? {
            Ok(())
        } else {
            Err(AppError::Domain(DomainError::not_found("Task")))
        }
    }
}

impl JobService {
    // ── cloud-storage links ─────────────────────────────────────────────────
    //
    // Not a Dropbox integration: these are shared URLs from any provider. See
    // docs/audit/integrations-reality.md.

    pub async fn links(&self, s: &SessionUser, job: JobId) -> AppResult<Vec<JobLink>> {
        self.get(s, job).await?;
        Ok(self.links.list(s.principal.scope(), job).await?)
    }

    pub async fn add_link(
        &self,
        s: &SessionUser,
        job: JobId,
        input: JobLinkInput,
    ) -> AppResult<JobLink> {
        self.get(s, job).await?;
        let input = input.normalised();
        input.validate().map_err(AppError::Domain)?;
        Ok(self.links.create(s.principal.scope(), job, &input).await?)
    }

    pub async fn update_link(
        &self,
        s: &SessionUser,
        job: JobId,
        id: i32,
        input: JobLinkInput,
    ) -> AppResult<JobLink> {
        self.get(s, job).await?;
        let input = input.normalised();
        input.validate().map_err(AppError::Domain)?;
        self.links
            .update(s.principal.scope(), id, &input)
            .await?
            .ok_or_else(|| AppError::Domain(DomainError::not_found("Link")))
    }

    pub async fn delete_link(&self, s: &SessionUser, job: JobId, id: i32) -> AppResult<()> {
        self.get(s, job).await?;
        if self.links.delete(s.principal.scope(), id).await? {
            Ok(())
        } else {
            Err(AppError::Domain(DomainError::not_found("Link")))
        }
    }
}

/// Mirrors `job_tasks_status_check` so the caller gets a field-level 400 rather
/// than the constraint backstop.
fn validate_task(input: &JobTaskInput) -> AppResult<()> {
    if input.title.trim().is_empty() {
        return Err(AppError::Domain(DomainError::invalid(
            "title",
            "is required",
        )));
    }
    if let Some(status) = input.status.as_deref() {
        if !matches!(status, "pending" | "in_progress" | "completed") {
            return Err(AppError::Domain(DomainError::invalid(
                "status",
                "must be one of pending, in_progress, completed",
            )));
        }
    }
    Ok(())
}
