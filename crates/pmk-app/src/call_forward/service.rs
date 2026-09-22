use std::sync::Arc;

use pmk_domain::call_forward::{
    validate_parent, CallForwardInput, CallForwardItemWithDelay, ItemType,
};
use pmk_domain::ids::{CallForwardItemId, JobId};
use pmk_domain::DomainError;
use pmk_ports::repository::{
    CallForwardFilter, CallForwardRepository, JobRepository, ReorderEntry,
};
use pmk_ports::Clock;

use crate::identity::SessionUser;
use crate::{AppError, AppResult};

pub struct CallForwardService {
    items: Arc<dyn CallForwardRepository>,
    jobs: Arc<dyn JobRepository>,
    clock: Arc<dyn Clock>,
}

impl std::fmt::Debug for CallForwardService {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        f.debug_struct("CallForwardService").finish_non_exhaustive()
    }
}

impl CallForwardService {
    #[must_use]
    pub fn new(
        items: Arc<dyn CallForwardRepository>,
        jobs: Arc<dyn JobRepository>,
        clock: Arc<dyn Clock>,
    ) -> Self {
        Self { items, jobs, clock }
    }

    async fn visible_jobs(&self, s: &SessionUser) -> AppResult<Option<Vec<JobId>>> {
        if s.user.role.sees_all_company_jobs() {
            return Ok(None);
        }
        Ok(Some(
            self.jobs
                .visible_job_ids(s.principal.scope(), s.user.id)
                .await?,
        ))
    }

    /// Delay is computed at read time against "today" in the company's
    /// configured timezone (domain-rules R1), never stored.
    fn today(&self) -> chrono::NaiveDate {
        self.clock.today()
    }

    pub async fn list(
        &self,
        s: &SessionUser,
        filter: CallForwardFilter,
    ) -> AppResult<Vec<CallForwardItemWithDelay>> {
        let visible = self.visible_jobs(s).await?;
        let today = self.today();
        Ok(self
            .items
            .list(s.principal.scope(), visible.as_deref(), &filter)
            .await?
            .into_iter()
            .map(|i| i.with_delay(today))
            .collect())
    }

    pub async fn get(
        &self,
        s: &SessionUser,
        id: CallForwardItemId,
    ) -> AppResult<CallForwardItemWithDelay> {
        let visible = self.visible_jobs(s).await?;
        let today = self.today();
        self.items
            .find(s.principal.scope(), visible.as_deref(), id)
            .await?
            .map(|i| i.with_delay(today))
            .ok_or_else(|| AppError::Domain(DomainError::not_found("Call forward item")))
    }

    pub async fn upcoming(
        &self,
        s: &SessionUser,
        days: i64,
    ) -> AppResult<Vec<CallForwardItemWithDelay>> {
        let visible = self.visible_jobs(s).await?;
        let today = self.today();
        Ok(self
            .items
            .upcoming(s.principal.scope(), visible.as_deref(), today, days)
            .await?
            .into_iter()
            .map(|i| i.with_delay(today))
            .collect())
    }

    /// Confirms the caller can see the job before touching its items.
    async fn assert_job(&self, s: &SessionUser, job: JobId) -> AppResult<()> {
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

    /// Validates a proposed parent link (R2): same job, only under a HEADER,
    /// never a nested HEADER, never itself.
    async fn check_parent(
        &self,
        s: &SessionUser,
        child_type: ItemType,
        child_id: Option<CallForwardItemId>,
        parent_id: Option<i32>,
        job: JobId,
    ) -> AppResult<()> {
        let Some(pid) = parent_id else { return Ok(()) };
        let pid = CallForwardItemId(pid);
        let parent = self
            .items
            .type_and_job(s.principal.scope(), pid)
            .await?
            .ok_or_else(|| AppError::Domain(DomainError::invalid("parentId", "no such item")))?;
        validate_parent(child_type, child_id, Some((pid, parent.0, parent.1)), job)
            .map_err(AppError::Domain)
    }

    pub async fn create(
        &self,
        s: &SessionUser,
        job: JobId,
        input: CallForwardInput,
    ) -> AppResult<CallForwardItemWithDelay> {
        input.validate().map_err(AppError::Domain)?;
        self.assert_job(s, job).await?;
        let child_type = input.item_type.unwrap_or(ItemType::Task);
        self.check_parent(s, child_type, None, input.parent_id, job)
            .await?;
        let today = self.today();
        Ok(self
            .items
            .create(s.principal.scope(), job, &input)
            .await?
            .with_delay(today))
    }

    /// Creates a tree in one transaction.
    ///
    /// `local_parent` indexes into the same list, letting a client describe a
    /// hierarchy before any ids exist. Indices must point **backwards**, which
    /// both guarantees a valid insert order and makes a cycle unrepresentable.
    pub async fn create_bulk(
        &self,
        s: &SessionUser,
        job: JobId,
        items: Vec<(CallForwardInput, Option<usize>)>,
    ) -> AppResult<Vec<CallForwardItemWithDelay>> {
        if items.is_empty() {
            return Err(AppError::Domain(DomainError::invalid(
                "items",
                "is required",
            )));
        }
        if items.len() > 500 {
            return Err(AppError::Domain(DomainError::invalid(
                "items",
                "cannot exceed 500 per request",
            )));
        }
        self.assert_job(s, job).await?;

        for (idx, (input, local_parent)) in items.iter().enumerate() {
            input.validate().map_err(AppError::Domain)?;
            if let Some(p) = local_parent {
                if *p >= idx {
                    return Err(AppError::Domain(DomainError::invalid(
                        "localParent",
                        "must refer to an earlier item in the list",
                    )));
                }
                let parent_type = items[*p].0.item_type.unwrap_or(ItemType::Task);
                let child_type = input.item_type.unwrap_or(ItemType::Task);
                validate_parent(
                    child_type,
                    None,
                    Some((CallForwardItemId(0), parent_type, job)),
                    job,
                )
                .map_err(AppError::Domain)?;
            } else {
                let child_type = input.item_type.unwrap_or(ItemType::Task);
                self.check_parent(s, child_type, None, input.parent_id, job)
                    .await?;
            }
        }

        let today = self.today();
        Ok(self
            .items
            .create_bulk(s.principal.scope(), job, &items)
            .await?
            .into_iter()
            .map(|i| i.with_delay(today))
            .collect())
    }

    pub async fn update(
        &self,
        s: &SessionUser,
        id: CallForwardItemId,
        input: CallForwardInput,
    ) -> AppResult<CallForwardItemWithDelay> {
        let existing = self.get(s, id).await?;
        input.validate().map_err(AppError::Domain)?;
        let child_type = input.item_type.unwrap_or(existing.item.item_type);
        self.check_parent(
            s,
            child_type,
            Some(id),
            input.parent_id,
            existing.item.job_id,
        )
        .await?;
        let today = self.today();
        self.items
            .update(s.principal.scope(), id, &input)
            .await?
            .map(|i| i.with_delay(today))
            .ok_or_else(|| AppError::Domain(DomainError::not_found("Call forward item")))
    }

    pub async fn delete(&self, s: &SessionUser, id: CallForwardItemId) -> AppResult<()> {
        self.get(s, id).await?;
        if self.items.delete(s.principal.scope(), id).await? {
            Ok(())
        } else {
            Err(AppError::Domain(DomainError::not_found(
                "Call forward item",
            )))
        }
    }

    /// Bulk reorder. Every referenced item must be visible to the caller, so a
    /// reorder cannot be used to probe or move another tenant's rows.
    pub async fn reorder(&self, s: &SessionUser, entries: Vec<ReorderEntry>) -> AppResult<u64> {
        if entries.is_empty() {
            return Err(AppError::Domain(DomainError::invalid(
                "items",
                "is required",
            )));
        }
        let visible = self.visible_jobs(s).await?;
        let mut job: Option<JobId> = None;
        for e in &entries {
            let item = self
                .items
                .find(s.principal.scope(), visible.as_deref(), e.id)
                .await?
                .ok_or_else(|| AppError::Domain(DomainError::not_found("Call forward item")))?;
            // A reorder spanning two jobs would be a client bug and could move
            // an item between jobs as a side effect.
            match job {
                None => job = Some(item.job_id),
                Some(j) if j != item.job_id => {
                    return Err(AppError::Domain(DomainError::invalid(
                        "items",
                        "must all belong to the same job",
                    )))
                }
                Some(_) => {}
            }
        }
        Ok(self.items.reorder(s.principal.scope(), &entries).await?)
    }
}
