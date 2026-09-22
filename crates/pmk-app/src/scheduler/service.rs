use std::sync::Arc;

use pmk_domain::ids::{JobId, SchedulerAbsenceId, SchedulerAllocationId, SchedulerWorkerId};
use pmk_domain::scheduler::{
    absence_covering, Absence, AbsenceInput, Allocation, AllocationInput, AllocationTarget, Worker,
    WorkerInput,
};
use pmk_domain::DomainError;
use pmk_ports::repository::{
    BoardData, DateRange, DayNote, JobRepository, MaintenanceJob, MaintenanceJobInput,
    SchedulerRepository,
};

use crate::identity::SessionUser;
use crate::{AppError, AppResult};

/// The widest window the board will render in one request.
///
/// The legacy UI shows a month; a year is already generous. Without a cap a
/// client could ask for a decade and make the server materialise it.
const MAX_BOARD_DAYS: i64 = 400;

pub struct SchedulerService {
    scheduler: Arc<dyn SchedulerRepository>,
    jobs: Arc<dyn JobRepository>,
}

impl std::fmt::Debug for SchedulerService {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        f.debug_struct("SchedulerService").finish_non_exhaustive()
    }
}

impl SchedulerService {
    #[must_use]
    pub fn new(scheduler: Arc<dyn SchedulerRepository>, jobs: Arc<dyn JobRepository>) -> Self {
        Self { scheduler, jobs }
    }

    fn check_range(range: DateRange) -> AppResult<()> {
        if range.from > range.to {
            return Err(AppError::Domain(DomainError::invalid(
                "to",
                "must be on or after the from date",
            )));
        }
        if (range.to - range.from).num_days() > MAX_BOARD_DAYS {
            return Err(AppError::Domain(DomainError::invalid(
                "to",
                format!("the window cannot exceed {MAX_BOARD_DAYS} days"),
            )));
        }
        Ok(())
    }

    // ── board ───────────────────────────────────────────────────────────────

    pub async fn board(&self, s: &SessionUser, range: DateRange) -> AppResult<BoardData> {
        Self::check_range(range)?;
        Ok(self.scheduler.board(s.principal.scope(), range).await?)
    }

    // ── workers ─────────────────────────────────────────────────────────────

    pub async fn workers(&self, s: &SessionUser, include_inactive: bool) -> AppResult<Vec<Worker>> {
        Ok(self
            .scheduler
            .workers(s.principal.scope(), include_inactive)
            .await?)
    }

    pub async fn create_worker(&self, s: &SessionUser, input: WorkerInput) -> AppResult<Worker> {
        let input = input.normalised();
        input.validate().map_err(AppError::Domain)?;
        Ok(self
            .scheduler
            .create_worker(s.principal.scope(), &input)
            .await?)
    }

    pub async fn update_worker(
        &self,
        s: &SessionUser,
        id: SchedulerWorkerId,
        input: WorkerInput,
    ) -> AppResult<Worker> {
        let input = input.normalised();
        input.validate().map_err(AppError::Domain)?;
        self.scheduler
            .update_worker(s.principal.scope(), id, &input)
            .await?
            .ok_or_else(|| AppError::Domain(DomainError::not_found("Worker")))
    }

    /// Deactivates rather than deletes, so past allocations stay on the board.
    pub async fn deactivate_worker(&self, s: &SessionUser, id: SchedulerWorkerId) -> AppResult<()> {
        if self
            .scheduler
            .deactivate_worker(s.principal.scope(), id)
            .await?
        {
            Ok(())
        } else {
            // Already inactive, or not ours. Idempotent either way; a second
            // delete should not fail.
            Ok(())
        }
    }

    /// The legacy `DELETE /workers/:id/permanent`. Cascades.
    pub async fn purge_worker(&self, s: &SessionUser, id: SchedulerWorkerId) -> AppResult<()> {
        if self.scheduler.purge_worker(s.principal.scope(), id).await? {
            Ok(())
        } else {
            Err(AppError::Domain(DomainError::not_found("Worker")))
        }
    }

    // ── absences ────────────────────────────────────────────────────────────

    pub async fn absences(&self, s: &SessionUser, range: DateRange) -> AppResult<Vec<Absence>> {
        Self::check_range(range)?;
        Ok(self.scheduler.absences(s.principal.scope(), range).await?)
    }

    pub async fn create_absence(&self, s: &SessionUser, input: AbsenceInput) -> AppResult<Absence> {
        // Validated first so an inverted range is a 400 rather than the raw
        // 22000 the overlap trigger would raise (domain-rules R12).
        input.validate().map_err(AppError::Domain)?;
        Ok(self
            .scheduler
            .create_absence(s.principal.scope(), &input)
            .await?)
    }

    pub async fn update_absence(
        &self,
        s: &SessionUser,
        id: SchedulerAbsenceId,
        input: AbsenceInput,
    ) -> AppResult<Absence> {
        input.validate().map_err(AppError::Domain)?;
        self.scheduler
            .update_absence(s.principal.scope(), id, &input)
            .await?
            .ok_or_else(|| AppError::Domain(DomainError::not_found("Absence")))
    }

    pub async fn delete_absence(&self, s: &SessionUser, id: SchedulerAbsenceId) -> AppResult<()> {
        if self
            .scheduler
            .delete_absence(s.principal.scope(), id)
            .await?
        {
            Ok(())
        } else {
            Err(AppError::Domain(DomainError::not_found("Absence")))
        }
    }

    // ── allocations ─────────────────────────────────────────────────────────

    pub async fn allocations(
        &self,
        s: &SessionUser,
        range: DateRange,
    ) -> AppResult<Vec<Allocation>> {
        Self::check_range(range)?;
        Ok(self
            .scheduler
            .allocations(s.principal.scope(), range)
            .await?)
    }

    /// Creates an allocation, refusing to book a worker who is away.
    ///
    /// The absence check is advisory in the sense that the database does not
    /// enforce it -- there is no constraint linking allocations to absences --
    /// so it lives here, and returns a 409 naming the conflicting absence so
    /// the UI can explain rather than just refuse.
    pub async fn create_allocation(
        &self,
        s: &SessionUser,
        input: AllocationInput,
    ) -> AppResult<Allocation> {
        input.validate().map_err(AppError::Domain)?;
        self.assert_target_visible(s, &input).await?;
        self.assert_worker_available(s, &input).await?;
        Ok(self
            .scheduler
            .create_allocation(s.principal.scope(), &input)
            .await?)
    }

    pub async fn update_allocation(
        &self,
        s: &SessionUser,
        id: SchedulerAllocationId,
        input: AllocationInput,
    ) -> AppResult<Allocation> {
        input.validate().map_err(AppError::Domain)?;
        self.assert_target_visible(s, &input).await?;
        self.assert_worker_available(s, &input).await?;
        self.scheduler
            .update_allocation(s.principal.scope(), id, &input)
            .await?
            .ok_or_else(|| AppError::Domain(DomainError::not_found("Allocation")))
    }

    pub async fn delete_allocation(
        &self,
        s: &SessionUser,
        id: SchedulerAllocationId,
    ) -> AppResult<()> {
        if self
            .scheduler
            .delete_allocation(s.principal.scope(), id)
            .await?
        {
            Ok(())
        } else {
            Err(AppError::Domain(DomainError::not_found("Allocation")))
        }
    }

    /// A real job must be one the caller can see; a maintenance job is
    /// tenant-scoped by RLS and needs no extra check.
    async fn assert_target_visible(
        &self,
        s: &SessionUser,
        input: &AllocationInput,
    ) -> AppResult<()> {
        if let AllocationTarget::Job(job) = input.target().map_err(AppError::Domain)? {
            self.jobs
                .find(
                    s.principal.scope(),
                    s.user.id,
                    s.user.role.sees_all_company_jobs(),
                    job,
                )
                .await?
                .ok_or_else(|| AppError::Domain(DomainError::not_found("Job")))?;
        }
        Ok(())
    }

    async fn assert_worker_available(
        &self,
        s: &SessionUser,
        input: &AllocationInput,
    ) -> AppResult<()> {
        let day = DateRange {
            from: input.assigned_date,
            to: input.assigned_date,
        };
        let absences = self.scheduler.absences(s.principal.scope(), day).await?;
        if let Some(conflict) = absence_covering(
            &absences,
            SchedulerWorkerId(input.worker_id),
            input.assigned_date,
        ) {
            return Err(AppError::Domain(DomainError::Conflict(format!(
                "That worker is on {} from {} to {}",
                conflict.absence_type.as_str(),
                conflict.start_date,
                conflict.end_date
            ))));
        }
        Ok(())
    }

    // ── maintenance jobs ────────────────────────────────────────────────────

    pub async fn maintenance_jobs(&self, s: &SessionUser) -> AppResult<Vec<MaintenanceJob>> {
        Ok(self.scheduler.maintenance_jobs(s.principal.scope()).await?)
    }

    pub async fn create_maintenance_job(
        &self,
        s: &SessionUser,
        input: MaintenanceJobInput,
    ) -> AppResult<MaintenanceJob> {
        Self::check_maintenance(&input)?;
        Ok(self
            .scheduler
            .create_maintenance_job(s.principal.scope(), &input)
            .await?)
    }

    pub async fn update_maintenance_job(
        &self,
        s: &SessionUser,
        id: i32,
        input: MaintenanceJobInput,
    ) -> AppResult<MaintenanceJob> {
        Self::check_maintenance(&input)?;
        self.scheduler
            .update_maintenance_job(s.principal.scope(), id, &input)
            .await?
            .ok_or_else(|| AppError::Domain(DomainError::not_found("Maintenance job")))
    }

    pub async fn delete_maintenance_job(&self, s: &SessionUser, id: i32) -> AppResult<()> {
        if self
            .scheduler
            .delete_maintenance_job(s.principal.scope(), id)
            .await?
        {
            Ok(())
        } else {
            Err(AppError::Domain(DomainError::not_found("Maintenance job")))
        }
    }

    /// Mirrors `scheduler_maintenance_jobs_status_check`.
    fn check_maintenance(input: &MaintenanceJobInput) -> AppResult<()> {
        if input.name.trim().is_empty() {
            return Err(AppError::Domain(DomainError::invalid(
                "name",
                "is required",
            )));
        }
        if let Some(status) = input.status.as_deref() {
            if !matches!(status, "active" | "completed" | "archived" | "on_hold") {
                return Err(AppError::Domain(DomainError::invalid(
                    "status",
                    "must be one of active, completed, archived, on_hold",
                )));
            }
        }
        Ok(())
    }

    // ── day notes ───────────────────────────────────────────────────────────

    pub async fn day_notes(&self, s: &SessionUser, range: DateRange) -> AppResult<Vec<DayNote>> {
        Self::check_range(range)?;
        Ok(self.scheduler.day_notes(s.principal.scope(), range).await?)
    }

    pub async fn set_day_note(
        &self,
        s: &SessionUser,
        job: JobId,
        date: chrono::NaiveDate,
        note: String,
    ) -> AppResult<DayNote> {
        if note.trim().is_empty() {
            return Err(AppError::Domain(DomainError::invalid(
                "note",
                "is required",
            )));
        }
        // The note hangs off a job, so the caller must be able to see it.
        self.jobs
            .find(
                s.principal.scope(),
                s.user.id,
                s.user.role.sees_all_company_jobs(),
                job,
            )
            .await?
            .ok_or_else(|| AppError::Domain(DomainError::not_found("Job")))?;
        Ok(self
            .scheduler
            .set_day_note(s.principal.scope(), job, date, &note)
            .await?)
    }

    pub async fn delete_day_note(&self, s: &SessionUser, id: i32) -> AppResult<()> {
        if self
            .scheduler
            .delete_day_note(s.principal.scope(), id)
            .await?
        {
            Ok(())
        } else {
            Err(AppError::Domain(DomainError::not_found("Note")))
        }
    }
}
