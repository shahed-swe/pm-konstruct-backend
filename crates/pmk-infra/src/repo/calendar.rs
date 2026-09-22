//! `CalendarRepository` over Postgres.
//!
//! Overlap is tested on *effective* dates: `COALESCE` across actual and
//! estimated, start and finish, in the same precedence the domain's
//! `effective_range` uses. An item carrying only an estimated start still
//! overlaps the month it falls in, rather than being filtered out here and
//! then never drawn.

use async_trait::async_trait;
use pmk_domain::dashboard::calendar::{
    CalendarFilterJob, CalendarFilterSupervisor, EventKind, MonthWindow,
};
use pmk_domain::dashboard::JobScope;
use pmk_domain::ids::{JobId, UserId};
use pmk_domain::tenant::TenantScope;
use pmk_ports::repository::{CalendarFilter, CalendarItemRow, CalendarJobRow, CalendarRepository};
use pmk_ports::PortResult;
use sqlx::{PgPool, Postgres, Row, Transaction};

use super::user::map_sqlx;
use crate::db::tenant::set_tenant;

/// Earliest of the four dates, in the domain's precedence.
const EFFECTIVE_START: &str =
    "COALESCE(cf.actual_start, cf.est_start, cf.actual_finish, cf.est_finish)";
/// Latest of the four, likewise.
const EFFECTIVE_FINISH: &str =
    "COALESCE(cf.actual_finish, cf.est_finish, cf.actual_start, cf.est_start)";

#[derive(Debug, Clone)]
pub struct PgCalendarRepository {
    pool: PgPool,
}

impl PgCalendarRepository {
    #[must_use]
    pub const fn new(pool: PgPool) -> Self {
        Self { pool }
    }

    async fn begin(&self, scope: TenantScope) -> PortResult<Transaction<'_, Postgres>> {
        let mut tx = self.pool.begin().await.map_err(map_sqlx)?;
        set_tenant(&mut tx, scope).await.map_err(map_sqlx)?;
        Ok(tx)
    }
}

#[async_trait]
impl CalendarRepository for PgCalendarRepository {
    async fn jobs_in_window(
        &self,
        scope: TenantScope,
        jobs: &JobScope,
        window: MonthWindow,
        filter: CalendarFilter,
    ) -> PortResult<Vec<CalendarJobRow>> {
        if jobs.is_empty() {
            return Ok(Vec::new());
        }
        let mut tx = self.begin(scope).await?;
        // A job with no end date is open-ended, so it overlaps every window
        // from its start onwards.
        let rows = sqlx::query(
            "SELECT j.id, j.name, j.job_number, j.address, j.start_date, j.end_date, \
                    j.status, j.supervisor_id, u.name AS supervisor_name \
             FROM jobs j \
             LEFT JOIN users u ON u.id = j.supervisor_id \
             WHERE j.start_date <= $2 \
               AND (j.end_date >= $3 OR j.end_date IS NULL) \
               AND ($1::int[] IS NULL OR j.id = ANY($1)) \
               AND ($4::int IS NULL OR j.id = $4) \
               AND ($5::int IS NULL OR j.supervisor_id = $5)",
        )
        .bind(jobs.ids().as_deref())
        .bind(window.last)
        .bind(window.first)
        .bind(filter.job_id.map(JobId::get))
        .bind(filter.supervisor_id.map(UserId::get))
        .fetch_all(&mut *tx)
        .await
        .map_err(map_sqlx)?;
        tx.commit().await.map_err(map_sqlx)?;

        Ok(rows
            .into_iter()
            .map(|r| CalendarJobRow {
                id: JobId(r.get("id")),
                name: r.get("name"),
                job_number: r.get("job_number"),
                address: r.get("address"),
                start_date: r.get("start_date"),
                end_date: r.get("end_date"),
                status: r.get("status"),
                supervisor_id: r.get::<Option<i32>, _>("supervisor_id").map(UserId),
                supervisor_name: r.get("supervisor_name"),
            })
            .collect())
    }

    async fn items_in_window(
        &self,
        scope: TenantScope,
        jobs: &JobScope,
        window: MonthWindow,
        kind: EventKind,
        filter: CalendarFilter,
    ) -> PortResult<Vec<CalendarItemRow>> {
        if jobs.is_empty() {
            return Ok(Vec::new());
        }
        // Only the two call-forward kinds reach here; a job is not a row of
        // `call_forward` and is fetched by `jobs_in_window`.
        let item_type = match kind {
            EventKind::Claim => "STAGE_CLAIM",
            EventKind::Task => "TASK",
            EventKind::Job => return Ok(Vec::new()),
        };

        let mut tx = self.begin(scope).await?;
        let sql = format!(
            "SELECT cf.id, cf.title, cf.est_start, cf.est_finish, cf.actual_start, \
                    cf.actual_finish, cf.status, cf.supplier_trade, cf.job_id, \
                    j.name AS job_name, j.job_number, j.address AS job_address, \
                    j.supervisor_id, u.name AS supervisor_name \
             FROM call_forward cf \
             JOIN jobs j ON j.id = cf.job_id \
             LEFT JOIN users u ON u.id = j.supervisor_id \
             WHERE cf.item_type = $6 \
               AND {EFFECTIVE_START} <= $2 \
               AND {EFFECTIVE_FINISH} >= $3 \
               AND ($1::int[] IS NULL OR cf.job_id = ANY($1)) \
               AND ($4::int IS NULL OR cf.job_id = $4) \
               AND ($5::int IS NULL OR j.supervisor_id = $5)"
        );
        let rows = sqlx::query(&sql)
            .bind(jobs.ids().as_deref())
            .bind(window.last)
            .bind(window.first)
            .bind(filter.job_id.map(JobId::get))
            .bind(filter.supervisor_id.map(UserId::get))
            .bind(item_type)
            .fetch_all(&mut *tx)
            .await
            .map_err(map_sqlx)?;
        tx.commit().await.map_err(map_sqlx)?;

        Ok(rows
            .into_iter()
            .map(|r| CalendarItemRow {
                id: r.get("id"),
                title: r.get("title"),
                est_start: r.get("est_start"),
                est_finish: r.get("est_finish"),
                actual_start: r.get("actual_start"),
                actual_finish: r.get("actual_finish"),
                status: r.get("status"),
                supplier_trade: r.get("supplier_trade"),
                job_id: JobId(r.get("job_id")),
                job_name: r.get("job_name"),
                job_number: r.get("job_number"),
                job_address: r.get("job_address"),
                supervisor_id: r.get::<Option<i32>, _>("supervisor_id").map(UserId),
                supervisor_name: r.get("supervisor_name"),
            })
            .collect())
    }

    async fn filter_jobs(
        &self,
        scope: TenantScope,
        jobs: &JobScope,
    ) -> PortResult<Vec<CalendarFilterJob>> {
        if jobs.is_empty() {
            return Ok(Vec::new());
        }
        let mut tx = self.begin(scope).await?;
        let rows = sqlx::query(
            "SELECT id, name, job_number, address FROM jobs \
             WHERE ($1::int[] IS NULL OR id = ANY($1)) ORDER BY name",
        )
        .bind(jobs.ids().as_deref())
        .fetch_all(&mut *tx)
        .await
        .map_err(map_sqlx)?;
        tx.commit().await.map_err(map_sqlx)?;

        Ok(rows
            .into_iter()
            .map(|r| CalendarFilterJob {
                id: JobId(r.get("id")),
                name: r.get::<Option<String>, _>("name").unwrap_or_default(),
                job_number: r.get::<Option<String>, _>("job_number").unwrap_or_default(),
                address: r.get("address"),
            })
            .collect())
    }

    async fn filter_supervisors(
        &self,
        scope: TenantScope,
        jobs: &JobScope,
    ) -> PortResult<Vec<CalendarFilterSupervisor>> {
        if jobs.is_empty() {
            return Ok(Vec::new());
        }
        let mut tx = self.begin(scope).await?;
        // One UNION rather than the legacy's two queries plus a manual dedupe:
        // UNION already removes duplicates, and ordering in SQL means the list
        // does not depend on the locale the API happens to run under.
        let rows = sqlx::query(
            "SELECT id, name FROM ( \
               SELECT u.id, u.name FROM jobs j \
               JOIN users u ON u.id = j.supervisor_id \
               WHERE u.role = 'SUPERVISOR' AND ($1::int[] IS NULL OR j.id = ANY($1)) \
               UNION \
               SELECT u.id, u.name FROM job_assignments a \
               JOIN users u ON u.id = a.user_id \
               WHERE u.role = 'SUPERVISOR' AND ($1::int[] IS NULL OR a.job_id = ANY($1)) \
             ) s ORDER BY name, id",
        )
        .bind(jobs.ids().as_deref())
        .fetch_all(&mut *tx)
        .await
        .map_err(map_sqlx)?;
        tx.commit().await.map_err(map_sqlx)?;

        Ok(rows
            .into_iter()
            .map(|r| CalendarFilterSupervisor {
                id: UserId(r.get("id")),
                name: r.get::<Option<String>, _>("name").unwrap_or_default(),
            })
            .collect())
    }
}
