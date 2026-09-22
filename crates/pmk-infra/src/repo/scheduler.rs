//! `SchedulerRepository` over Postgres.
//!
//! Every scheduler table carries `company_id` directly, so RLS scopes them
//! without a join. The three invariants in domain-rules R12 are enforced by the
//! database -- two unique constraints, a CHECK, and the overlap trigger -- and
//! mirrored in the domain so callers get a field-level 400 rather than a
//! constraint error.

use async_trait::async_trait;
use pmk_domain::ids::{JobId, SchedulerAbsenceId, SchedulerAllocationId, SchedulerWorkerId};
use pmk_domain::scheduler::{
    Absence, AbsenceInput, AbsenceType, Allocation, AllocationInput, AllocationTarget, Worker,
    WorkerInput,
};
use pmk_domain::tenant::{CompanyId, TenantScope};
use pmk_ports::repository::{
    BoardData, DateRange, DayNote, MaintenanceJob, MaintenanceJobInput, SchedulerRepository,
};
use pmk_ports::{PortError, PortResult};
use sqlx::{PgPool, Postgres, Transaction};

use super::user::map_sqlx;
use crate::db::tenant::set_tenant;

const WORKER_COLS: &str =
    "id, company_id, name, trade, color, active, on_leave, leave_from, leave_to";

#[derive(Debug, sqlx::FromRow)]
struct WorkerRow {
    id: i32,
    company_id: i32,
    name: String,
    trade: Option<String>,
    color: String,
    active: bool,
    on_leave: bool,
    leave_from: Option<chrono::NaiveDate>,
    leave_to: Option<chrono::NaiveDate>,
}

impl From<WorkerRow> for Worker {
    fn from(r: WorkerRow) -> Self {
        Self {
            id: SchedulerWorkerId(r.id),
            company_id: CompanyId(r.company_id),
            name: r.name,
            trade: r.trade,
            color: r.color,
            active: r.active,
            on_leave: r.on_leave,
            leave_from: r.leave_from,
            leave_to: r.leave_to,
        }
    }
}

const ABSENCE_COLS: &str = "id, worker_id, absence_type, start_date, end_date";

#[derive(Debug, sqlx::FromRow)]
struct AbsenceRow {
    id: i32,
    worker_id: i32,
    absence_type: String,
    start_date: chrono::NaiveDate,
    end_date: chrono::NaiveDate,
}

impl From<AbsenceRow> for Absence {
    fn from(r: AbsenceRow) -> Self {
        Self {
            id: SchedulerAbsenceId(r.id),
            worker_id: SchedulerWorkerId(r.worker_id),
            // Constrained by `scheduler_worker_absence_type_check`.
            absence_type: AbsenceType::parse(&r.absence_type).unwrap_or(AbsenceType::Leave),
            start_date: r.start_date,
            end_date: r.end_date,
        }
    }
}

const ALLOCATION_COLS: &str = "id, worker_id, job_id, maintenance_job_id, assigned_date, note";

#[derive(Debug, sqlx::FromRow)]
struct AllocationRow {
    id: i32,
    worker_id: i32,
    job_id: Option<i32>,
    maintenance_job_id: Option<i32>,
    assigned_date: chrono::NaiveDate,
    note: Option<String>,
}

impl From<AllocationRow> for Allocation {
    fn from(r: AllocationRow) -> Self {
        Self {
            id: SchedulerAllocationId(r.id),
            worker_id: SchedulerWorkerId(r.worker_id),
            // `scheduler_allocation_target_check` guarantees exactly one is
            // set; the job branch is checked first and maintenance is the
            // remaining case.
            target: match (r.job_id, r.maintenance_job_id) {
                (Some(j), _) => AllocationTarget::Job(JobId(j)),
                (None, Some(m)) => AllocationTarget::MaintenanceJob(m),
                // Unreachable while the constraint holds.
                (None, None) => AllocationTarget::MaintenanceJob(0),
            },
            assigned_date: r.assigned_date,
            note: r.note,
        }
    }
}

const MAINTENANCE_COLS: &str = "id, name, reference, address, status";

#[derive(Debug, sqlx::FromRow)]
struct MaintenanceRow {
    id: i32,
    name: String,
    reference: Option<String>,
    address: Option<String>,
    status: String,
}

impl From<MaintenanceRow> for MaintenanceJob {
    fn from(r: MaintenanceRow) -> Self {
        Self {
            id: r.id,
            name: r.name,
            reference: r.reference,
            address: r.address,
            status: r.status,
        }
    }
}

#[derive(Debug, sqlx::FromRow)]
struct DayNoteRow {
    id: i32,
    job_id: i32,
    note_date: chrono::NaiveDate,
    note: String,
}

impl From<DayNoteRow> for DayNote {
    fn from(r: DayNoteRow) -> Self {
        Self {
            id: r.id,
            job_id: JobId(r.job_id),
            note_date: r.note_date,
            note: r.note,
        }
    }
}

#[derive(Debug, Clone)]
pub struct PgSchedulerRepository {
    pool: PgPool,
}

impl PgSchedulerRepository {
    #[must_use]
    pub const fn new(pool: PgPool) -> Self {
        Self { pool }
    }

    async fn begin(&self, scope: TenantScope) -> PortResult<Transaction<'_, Postgres>> {
        let mut tx = self.pool.begin().await.map_err(map_sqlx)?;
        set_tenant(&mut tx, scope).await.map_err(map_sqlx)?;
        Ok(tx)
    }

    /// Translates the overlap trigger's exception into a conflict.
    ///
    /// The trigger raises SQLSTATE `23P01` with the message
    /// "Worker absence periods cannot overlap". `map_sqlx` already maps `23P01`
    /// to `Conflict`, but the constraint name is absent because the error comes
    /// from `RAISE`, not from a named constraint -- so it is named here.
    fn map_absence_error(e: sqlx::Error) -> PortError {
        let text = e.to_string();
        if text.contains("cannot overlap") {
            return PortError::Conflict {
                constraint: Some("scheduler_worker_absence_overlap".into()),
            };
        }
        map_sqlx(e)
    }
}

#[async_trait]
impl SchedulerRepository for PgSchedulerRepository {
    // ── workers ─────────────────────────────────────────────────────────────

    async fn workers(&self, scope: TenantScope, include_inactive: bool) -> PortResult<Vec<Worker>> {
        let mut tx = self.begin(scope).await?;
        let sql = format!(
            "SELECT {WORKER_COLS} FROM scheduler_workers \
             WHERE ($1 OR active) ORDER BY name"
        );
        let rows: Vec<WorkerRow> = sqlx::query_as(&sql)
            .bind(include_inactive)
            .fetch_all(&mut *tx)
            .await
            .map_err(map_sqlx)?;
        tx.commit().await.map_err(map_sqlx)?;
        Ok(rows.into_iter().map(Into::into).collect())
    }

    async fn create_worker(&self, scope: TenantScope, input: &WorkerInput) -> PortResult<Worker> {
        let mut tx = self.begin(scope).await?;
        let sql = format!(
            "INSERT INTO scheduler_workers (company_id, name, trade, color, active) \
             VALUES ($1, $2, $3, COALESCE($4, '#3B82F6'), COALESCE($5, TRUE)) \
             RETURNING {WORKER_COLS}"
        );
        let row: WorkerRow = sqlx::query_as(&sql)
            .bind(scope.company_id().get())
            .bind(&input.name)
            .bind(input.trade.as_deref())
            .bind(input.color.as_deref())
            .bind(input.active)
            .fetch_one(&mut *tx)
            .await
            .map_err(map_sqlx)?;
        tx.commit().await.map_err(map_sqlx)?;
        Ok(row.into())
    }

    async fn update_worker(
        &self,
        scope: TenantScope,
        id: SchedulerWorkerId,
        input: &WorkerInput,
    ) -> PortResult<Option<Worker>> {
        let mut tx = self.begin(scope).await?;
        let sql = format!(
            "UPDATE scheduler_workers SET name = $2, trade = $3, \
                color = COALESCE($4, color), active = COALESCE($5, active), updated_at = NOW() \
             WHERE id = $1 RETURNING {WORKER_COLS}"
        );
        let row: Option<WorkerRow> = sqlx::query_as(&sql)
            .bind(id.get())
            .bind(&input.name)
            .bind(input.trade.as_deref())
            .bind(input.color.as_deref())
            .bind(input.active)
            .fetch_optional(&mut *tx)
            .await
            .map_err(map_sqlx)?;
        tx.commit().await.map_err(map_sqlx)?;
        Ok(row.map(Into::into))
    }

    async fn deactivate_worker(
        &self,
        scope: TenantScope,
        id: SchedulerWorkerId,
    ) -> PortResult<bool> {
        let mut tx = self.begin(scope).await?;
        // The default for DELETE /workers/{id}: past allocations stay on the
        // board, the worker simply stops being schedulable.
        let r = sqlx::query(
            "UPDATE scheduler_workers SET active = FALSE, updated_at = NOW() \
             WHERE id = $1 AND active",
        )
        .bind(id.get())
        .execute(&mut *tx)
        .await
        .map_err(map_sqlx)?;
        tx.commit().await.map_err(map_sqlx)?;
        Ok(r.rows_affected() > 0)
    }

    async fn purge_worker(&self, scope: TenantScope, id: SchedulerWorkerId) -> PortResult<bool> {
        let mut tx = self.begin(scope).await?;
        // Cascades to allocations and absences -- the legacy
        // `DELETE /workers/:id/permanent`.
        let r = sqlx::query("DELETE FROM scheduler_workers WHERE id = $1")
            .bind(id.get())
            .execute(&mut *tx)
            .await
            .map_err(map_sqlx)?;
        tx.commit().await.map_err(map_sqlx)?;
        Ok(r.rows_affected() > 0)
    }

    // ── absences ────────────────────────────────────────────────────────────

    async fn absences(&self, scope: TenantScope, range: DateRange) -> PortResult<Vec<Absence>> {
        let mut tx = self.begin(scope).await?;
        // Overlap, not containment: an absence that starts before the window
        // and ends inside it still makes those days unavailable. Matches
        // `idx_scheduler_worker_absences_company_dates`.
        let sql = format!(
            "SELECT {ABSENCE_COLS} FROM scheduler_worker_absences \
             WHERE start_date <= $2 AND end_date >= $1 \
             ORDER BY worker_id, start_date"
        );
        let rows: Vec<AbsenceRow> = sqlx::query_as(&sql)
            .bind(range.from)
            .bind(range.to)
            .fetch_all(&mut *tx)
            .await
            .map_err(map_sqlx)?;
        tx.commit().await.map_err(map_sqlx)?;
        Ok(rows.into_iter().map(Into::into).collect())
    }

    async fn create_absence(
        &self,
        scope: TenantScope,
        input: &AbsenceInput,
    ) -> PortResult<Absence> {
        let mut tx = self.begin(scope).await?;
        let sql = format!(
            "INSERT INTO scheduler_worker_absences \
                (company_id, worker_id, absence_type, start_date, end_date) \
             VALUES ($1, $2, $3, $4, $5) RETURNING {ABSENCE_COLS}"
        );
        let row: AbsenceRow = sqlx::query_as(&sql)
            .bind(scope.company_id().get())
            .bind(input.worker_id)
            .bind(input.absence_type.as_str())
            .bind(input.start_date)
            .bind(input.end_date)
            .fetch_one(&mut *tx)
            .await
            .map_err(Self::map_absence_error)?;
        tx.commit().await.map_err(map_sqlx)?;
        Ok(row.into())
    }

    async fn update_absence(
        &self,
        scope: TenantScope,
        id: SchedulerAbsenceId,
        input: &AbsenceInput,
    ) -> PortResult<Option<Absence>> {
        let mut tx = self.begin(scope).await?;
        let sql = format!(
            "UPDATE scheduler_worker_absences \
             SET worker_id = $2, absence_type = $3, start_date = $4, end_date = $5, \
                 updated_at = NOW() \
             WHERE id = $1 RETURNING {ABSENCE_COLS}"
        );
        let row: Option<AbsenceRow> = sqlx::query_as(&sql)
            .bind(id.get())
            .bind(input.worker_id)
            .bind(input.absence_type.as_str())
            .bind(input.start_date)
            .bind(input.end_date)
            .fetch_optional(&mut *tx)
            .await
            .map_err(Self::map_absence_error)?;
        tx.commit().await.map_err(map_sqlx)?;
        Ok(row.map(Into::into))
    }

    async fn delete_absence(&self, scope: TenantScope, id: SchedulerAbsenceId) -> PortResult<bool> {
        let mut tx = self.begin(scope).await?;
        let r = sqlx::query("DELETE FROM scheduler_worker_absences WHERE id = $1")
            .bind(id.get())
            .execute(&mut *tx)
            .await
            .map_err(map_sqlx)?;
        tx.commit().await.map_err(map_sqlx)?;
        Ok(r.rows_affected() > 0)
    }

    // ── allocations ─────────────────────────────────────────────────────────

    async fn allocations(
        &self,
        scope: TenantScope,
        range: DateRange,
    ) -> PortResult<Vec<Allocation>> {
        let mut tx = self.begin(scope).await?;
        // Matches `idx_scheduler_allocations_company_date`.
        let sql = format!(
            "SELECT {ALLOCATION_COLS} FROM scheduler_allocations \
             WHERE assigned_date BETWEEN $1 AND $2 \
             ORDER BY assigned_date, worker_id"
        );
        let rows: Vec<AllocationRow> = sqlx::query_as(&sql)
            .bind(range.from)
            .bind(range.to)
            .fetch_all(&mut *tx)
            .await
            .map_err(map_sqlx)?;
        tx.commit().await.map_err(map_sqlx)?;
        Ok(rows.into_iter().map(Into::into).collect())
    }

    async fn create_allocation(
        &self,
        scope: TenantScope,
        input: &AllocationInput,
    ) -> PortResult<Allocation> {
        let mut tx = self.begin(scope).await?;
        // `uq_scheduler_company_worker_date` makes double-booking impossible at
        // the database level, so two concurrent drags cannot both succeed.
        let sql = format!(
            "INSERT INTO scheduler_allocations \
                (company_id, worker_id, job_id, maintenance_job_id, assigned_date, note) \
             VALUES ($1, $2, $3, $4, $5, $6) RETURNING {ALLOCATION_COLS}"
        );
        let row: AllocationRow = sqlx::query_as(&sql)
            .bind(scope.company_id().get())
            .bind(input.worker_id)
            .bind(input.job_id)
            .bind(input.maintenance_job_id)
            .bind(input.assigned_date)
            .bind(input.note.as_deref())
            .fetch_one(&mut *tx)
            .await
            .map_err(map_sqlx)?;
        tx.commit().await.map_err(map_sqlx)?;
        Ok(row.into())
    }

    async fn update_allocation(
        &self,
        scope: TenantScope,
        id: SchedulerAllocationId,
        input: &AllocationInput,
    ) -> PortResult<Option<Allocation>> {
        let mut tx = self.begin(scope).await?;
        let sql = format!(
            "UPDATE scheduler_allocations \
             SET worker_id = $2, job_id = $3, maintenance_job_id = $4, \
                 assigned_date = $5, note = $6, updated_at = NOW() \
             WHERE id = $1 RETURNING {ALLOCATION_COLS}"
        );
        let row: Option<AllocationRow> = sqlx::query_as(&sql)
            .bind(id.get())
            .bind(input.worker_id)
            .bind(input.job_id)
            .bind(input.maintenance_job_id)
            .bind(input.assigned_date)
            .bind(input.note.as_deref())
            .fetch_optional(&mut *tx)
            .await
            .map_err(map_sqlx)?;
        tx.commit().await.map_err(map_sqlx)?;
        Ok(row.map(Into::into))
    }

    async fn delete_allocation(
        &self,
        scope: TenantScope,
        id: SchedulerAllocationId,
    ) -> PortResult<bool> {
        let mut tx = self.begin(scope).await?;
        let r = sqlx::query("DELETE FROM scheduler_allocations WHERE id = $1")
            .bind(id.get())
            .execute(&mut *tx)
            .await
            .map_err(map_sqlx)?;
        tx.commit().await.map_err(map_sqlx)?;
        Ok(r.rows_affected() > 0)
    }

    // ── maintenance jobs ────────────────────────────────────────────────────

    async fn maintenance_jobs(&self, scope: TenantScope) -> PortResult<Vec<MaintenanceJob>> {
        let mut tx = self.begin(scope).await?;
        let sql =
            format!("SELECT {MAINTENANCE_COLS} FROM scheduler_maintenance_jobs ORDER BY name");
        let rows: Vec<MaintenanceRow> = sqlx::query_as(&sql)
            .fetch_all(&mut *tx)
            .await
            .map_err(map_sqlx)?;
        tx.commit().await.map_err(map_sqlx)?;
        Ok(rows.into_iter().map(Into::into).collect())
    }

    async fn create_maintenance_job(
        &self,
        scope: TenantScope,
        input: &MaintenanceJobInput,
    ) -> PortResult<MaintenanceJob> {
        let mut tx = self.begin(scope).await?;
        let sql = format!(
            "INSERT INTO scheduler_maintenance_jobs \
                (company_id, name, reference, address, status) \
             VALUES ($1, $2, $3, $4, COALESCE($5, 'active')) RETURNING {MAINTENANCE_COLS}"
        );
        let row: MaintenanceRow = sqlx::query_as(&sql)
            .bind(scope.company_id().get())
            .bind(&input.name)
            .bind(input.reference.as_deref())
            .bind(input.address.as_deref())
            .bind(input.status.as_deref())
            .fetch_one(&mut *tx)
            .await
            .map_err(map_sqlx)?;
        tx.commit().await.map_err(map_sqlx)?;
        Ok(row.into())
    }

    async fn update_maintenance_job(
        &self,
        scope: TenantScope,
        id: i32,
        input: &MaintenanceJobInput,
    ) -> PortResult<Option<MaintenanceJob>> {
        let mut tx = self.begin(scope).await?;
        let sql = format!(
            "UPDATE scheduler_maintenance_jobs \
             SET name = $2, reference = $3, address = $4, \
                 status = COALESCE($5, status), updated_at = NOW() \
             WHERE id = $1 RETURNING {MAINTENANCE_COLS}"
        );
        let row: Option<MaintenanceRow> = sqlx::query_as(&sql)
            .bind(id)
            .bind(&input.name)
            .bind(input.reference.as_deref())
            .bind(input.address.as_deref())
            .bind(input.status.as_deref())
            .fetch_optional(&mut *tx)
            .await
            .map_err(map_sqlx)?;
        tx.commit().await.map_err(map_sqlx)?;
        Ok(row.map(Into::into))
    }

    async fn delete_maintenance_job(&self, scope: TenantScope, id: i32) -> PortResult<bool> {
        let mut tx = self.begin(scope).await?;
        let r = sqlx::query("DELETE FROM scheduler_maintenance_jobs WHERE id = $1")
            .bind(id)
            .execute(&mut *tx)
            .await
            .map_err(map_sqlx)?;
        tx.commit().await.map_err(map_sqlx)?;
        Ok(r.rows_affected() > 0)
    }

    // ── day notes ───────────────────────────────────────────────────────────

    async fn day_notes(&self, scope: TenantScope, range: DateRange) -> PortResult<Vec<DayNote>> {
        let mut tx = self.begin(scope).await?;
        let rows: Vec<DayNoteRow> = sqlx::query_as(
            "SELECT id, job_id, note_date, note FROM scheduler_job_day_notes \
             WHERE note_date BETWEEN $1 AND $2 ORDER BY note_date, job_id",
        )
        .bind(range.from)
        .bind(range.to)
        .fetch_all(&mut *tx)
        .await
        .map_err(map_sqlx)?;
        tx.commit().await.map_err(map_sqlx)?;
        Ok(rows.into_iter().map(Into::into).collect())
    }

    async fn set_day_note(
        &self,
        scope: TenantScope,
        job: JobId,
        date: chrono::NaiveDate,
        note: &str,
    ) -> PortResult<DayNote> {
        let mut tx = self.begin(scope).await?;
        // One note per job per day. DO UPDATE needs SELECT on the table, which
        // pmk_app has here -- unlike the media deletion queue, this table is
        // tenant-scoped, so reading it leaks nothing.
        let row: DayNoteRow = sqlx::query_as(
            "INSERT INTO scheduler_job_day_notes (company_id, job_id, note_date, note) \
             VALUES ($1, $2, $3, $4) \
             ON CONFLICT (job_id, note_date) \
             DO UPDATE SET note = EXCLUDED.note, updated_at = NOW() \
             RETURNING id, job_id, note_date, note",
        )
        .bind(scope.company_id().get())
        .bind(job.get())
        .bind(date)
        .bind(note)
        .fetch_one(&mut *tx)
        .await
        .map_err(map_sqlx)?;
        tx.commit().await.map_err(map_sqlx)?;
        Ok(row.into())
    }

    async fn delete_day_note(&self, scope: TenantScope, id: i32) -> PortResult<bool> {
        let mut tx = self.begin(scope).await?;
        let r = sqlx::query("DELETE FROM scheduler_job_day_notes WHERE id = $1")
            .bind(id)
            .execute(&mut *tx)
            .await
            .map_err(map_sqlx)?;
        tx.commit().await.map_err(map_sqlx)?;
        Ok(r.rows_affected() > 0)
    }

    // ── the board ───────────────────────────────────────────────────────────

    async fn board(&self, scope: TenantScope, range: DateRange) -> PortResult<BoardData> {
        // One transaction, four queries, so the board is a consistent snapshot:
        // an allocation created mid-render cannot appear without its worker.
        let mut tx = self.begin(scope).await?;

        let worker_sql =
            format!("SELECT {WORKER_COLS} FROM scheduler_workers WHERE active ORDER BY name");
        let workers: Vec<WorkerRow> = sqlx::query_as(&worker_sql)
            .fetch_all(&mut *tx)
            .await
            .map_err(map_sqlx)?;

        let absence_sql = format!(
            "SELECT {ABSENCE_COLS} FROM scheduler_worker_absences \
             WHERE start_date <= $2 AND end_date >= $1 ORDER BY worker_id, start_date"
        );
        let absences: Vec<AbsenceRow> = sqlx::query_as(&absence_sql)
            .bind(range.from)
            .bind(range.to)
            .fetch_all(&mut *tx)
            .await
            .map_err(map_sqlx)?;

        let alloc_sql = format!(
            "SELECT {ALLOCATION_COLS} FROM scheduler_allocations \
             WHERE assigned_date BETWEEN $1 AND $2 ORDER BY assigned_date, worker_id"
        );
        let allocations: Vec<AllocationRow> = sqlx::query_as(&alloc_sql)
            .bind(range.from)
            .bind(range.to)
            .fetch_all(&mut *tx)
            .await
            .map_err(map_sqlx)?;

        let notes: Vec<DayNoteRow> = sqlx::query_as(
            "SELECT id, job_id, note_date, note FROM scheduler_job_day_notes \
             WHERE note_date BETWEEN $1 AND $2 ORDER BY note_date, job_id",
        )
        .bind(range.from)
        .bind(range.to)
        .fetch_all(&mut *tx)
        .await
        .map_err(map_sqlx)?;

        tx.commit().await.map_err(map_sqlx)?;

        Ok(BoardData {
            workers: workers.into_iter().map(Into::into).collect(),
            absences: absences.into_iter().map(Into::into).collect(),
            allocations: allocations.into_iter().map(Into::into).collect(),
            day_notes: notes.into_iter().map(Into::into).collect(),
        })
    }
}
