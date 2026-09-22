use std::sync::Arc;

use pmk_domain::dashboard::JobScope;
use pmk_domain::ids::{JobId, ProgressId};
use pmk_domain::progress::{Progress, ProgressInput};
use pmk_domain::DomainError;
use pmk_ports::repository::{JobRepository, ProgressRepository};

use crate::identity::SessionUser;
use crate::{AppError, AppResult};

pub struct ProgressService {
    progress: Arc<dyn ProgressRepository>,
    jobs: Arc<dyn JobRepository>,
}

impl std::fmt::Debug for ProgressService {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        f.debug_struct("ProgressService").finish_non_exhaustive()
    }
}

impl ProgressService {
    #[must_use]
    pub fn new(progress: Arc<dyn ProgressRepository>, jobs: Arc<dyn JobRepository>) -> Self {
        Self { progress, jobs }
    }

    async fn scope(&self, s: &SessionUser) -> AppResult<JobScope> {
        if s.user.role.sees_all_company_jobs() {
            return Ok(JobScope::All);
        }
        Ok(JobScope::Only(
            self.jobs
                .visible_job_ids(s.principal.scope(), s.user.id)
                .await?,
        ))
    }

    async fn assert_job_access(&self, s: &SessionUser, job: JobId) -> AppResult<()> {
        self.jobs
            .find(
                s.principal.scope(),
                s.user.id,
                s.user.role.sees_all_company_jobs(),
                job,
            )
            .await?
            .ok_or_else(|| AppError::Domain(DomainError::not_found("Job")))?;
        Ok(())
    }

    /// Loads a record the caller may see, or 404.
    ///
    /// The job check is separate from the row lookup: RLS confines the row to
    /// the company, but a supervisor must not reach a record on a job they are
    /// not on.
    async fn owned(&self, s: &SessionUser, id: ProgressId) -> AppResult<Progress> {
        let record = self
            .progress
            .find(s.principal.scope(), id)
            .await?
            .ok_or_else(|| AppError::Domain(DomainError::not_found("Progress record")))?;
        self.assert_job_access(s, record.job_id).await?;
        Ok(record)
    }

    pub async fn list(&self, s: &SessionUser, job: Option<JobId>) -> AppResult<Vec<Progress>> {
        if let Some(job) = job {
            self.assert_job_access(s, job).await?;
        }
        let scope = self.scope(s).await?;
        Ok(self.progress.list(s.principal.scope(), &scope, job).await?)
    }

    pub async fn create(&self, s: &SessionUser, input: &ProgressInput) -> AppResult<Progress> {
        input.validate().map_err(AppError::Domain)?;
        self.assert_job_access(s, input.job_id).await?;
        Ok(self.progress.create(s.principal.scope(), input).await?)
    }

    /// One record, so a partial update has something to merge onto.
    pub async fn get(&self, s: &SessionUser, id: ProgressId) -> AppResult<Progress> {
        self.owned(s, id).await
    }

    pub async fn update(
        &self,
        s: &SessionUser,
        id: ProgressId,
        input: &ProgressInput,
    ) -> AppResult<Progress> {
        // The record's own job decides access; the job named in the payload is
        // ignored, because a record cannot be moved between jobs.
        let existing = self.owned(s, id).await?;
        let input = ProgressInput {
            job_id: existing.job_id,
            ..input.clone()
        };
        input.validate().map_err(AppError::Domain)?;
        self.progress
            .update(s.principal.scope(), id, &input)
            .await?
            .ok_or_else(|| AppError::Domain(DomainError::not_found("Progress record")))
    }

    pub async fn delete(&self, s: &SessionUser, id: ProgressId) -> AppResult<()> {
        self.owned(s, id).await?;
        if self.progress.delete(s.principal.scope(), id).await? {
            Ok(())
        } else {
            Err(AppError::Domain(DomainError::not_found("Progress record")))
        }
    }
}
