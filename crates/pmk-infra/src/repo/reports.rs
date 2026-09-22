//! `ReportsRepository` over Postgres.
//!
//! The legacy versions of these reports loaded every job and every
//! call-forward row in the database into Node and filtered in JavaScript --
//! `db.select().from(jobsTable)` with no predicate at all, once per report.
//! That was survivable at 22 jobs and would not be at 2,200. Here the
//! filtering, the aggregation and the ordering are all in SQL, and the service
//! does only the classification the domain owns.
//!
//! Two scopes apply to every query: RLS confines it to the company, and the
//! `JobScope` id list confines it to what R3 makes visible to the caller.

use async_trait::async_trait;
use pmk_domain::dashboard::calendar::CalendarFilterSupervisor;
use pmk_domain::dashboard::JobScope;
use pmk_domain::ids::{DiaryEntryId, JobId, ReportId, UserId};
use pmk_domain::tenant::TenantScope;
use pmk_ports::repository::{
    DailyProgressData, DailyTaskRow, DelayRow, DiaryReportRow, InspectionRow, JobProgressRow,
    ReportFilter, ReportMeta, ReportMetaJob, ReportsRepository, StageClaimRow, StoredReport,
    StoredReportInput, SupervisorCounts, UpcomingTaskRow, WeatherRow,
};
use pmk_ports::PortResult;
use sqlx::{PgPool, Postgres, Row, Transaction};

use super::user::map_sqlx;
use crate::db::tenant::set_tenant;

/// Rolls a diary entry's notes up by category.
///
/// The columns on `site_diary` itself (materials, issues, safety_notes…) are
/// legacy and always empty in production: the UI writes everything into
/// `diary_notes`. The report reads the notes and falls back to the column, so
/// both old and new rows render.
const DIARY_CATEGORY_AGG: &str = "
    COALESCE(NULLIF(string_agg(CASE WHEN dn.category = 'general' AND dn.content <> '' \
        THEN dn.content END, E'\\n' ORDER BY dn.sort_order, dn.created_at), ''), \
        COALESCE(sd.work_completed, '')) AS work_completed,
    COALESCE(NULLIF(string_agg(CASE WHEN dn.category = 'materials' AND dn.content <> '' \
        THEN dn.content END, E'\\n' ORDER BY dn.sort_order, dn.created_at), ''), \
        COALESCE(sd.materials, '')) AS materials,
    COALESCE(NULLIF(string_agg(CASE WHEN dn.category = 'trades' AND dn.content <> '' \
        THEN dn.content END, E'\\n' ORDER BY dn.sort_order, dn.created_at), ''), \
        COALESCE(sd.trades_on_site, '')) AS trades_on_site,
    COALESCE(NULLIF(string_agg(CASE WHEN dn.category = 'safety' AND dn.content <> '' \
        THEN dn.content END, E'\\n' ORDER BY dn.sort_order, dn.created_at), ''), \
        COALESCE(sd.safety_notes, '')) AS safety_notes,
    COALESCE(NULLIF(string_agg(CASE WHEN dn.category = 'client' AND dn.content <> '' \
        THEN dn.content END, E'\\n' ORDER BY dn.sort_order, dn.created_at), ''), \
        COALESCE(sd.client_instructions, '')) AS client_instructions,
    COALESCE(NULLIF(string_agg(CASE WHEN dn.category = 'issues' AND dn.content <> '' \
        THEN dn.content END, E'\\n' ORDER BY dn.sort_order, dn.created_at), ''), \
        COALESCE(sd.issues, '')) AS issues,
    COALESCE(NULLIF(string_agg(CASE WHEN dn.category = 'site_conditions' AND dn.content <> '' \
        THEN dn.content END, E'\\n' ORDER BY dn.sort_order, dn.created_at), ''), \
        COALESCE(sd.notes, '')) AS notes";

/// Jobs a supervisor is responsible for: primary, or assigned.
///
/// Used to filter *by* a chosen supervisor, which is a different question from
/// the caller's own visibility -- a manager filtering the report by Sarah is
/// not restricted to Sarah's jobs themselves.
const SUPERVISOR_JOBS: &str = "($3::int IS NULL OR j.supervisor_id = $3 OR EXISTS ( \
    SELECT 1 FROM job_assignments a WHERE a.job_id = j.id AND a.user_id = $3))";

#[derive(Debug, Clone)]
pub struct PgReportsRepository {
    pool: PgPool,
}

impl PgReportsRepository {
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

fn diary_row(r: &sqlx::postgres::PgRow) -> DiaryReportRow {
    DiaryReportRow {
        id: DiaryEntryId(r.get("id")),
        date: r.get("date"),
        job_id: JobId(r.get("job_id")),
        job_number: r.get("job_number"),
        job_name: r.get("job_name"),
        job_address: r.get("job_address"),
        author_id: r.get::<Option<i32>, _>("author_id").map(UserId),
        author_name: r.get("author_name"),
        weather: r.get("weather"),
        workforce: r.get("workforce"),
        work_completed: r.get("work_completed"),
        materials: r.get("materials"),
        trades_on_site: r.get("trades_on_site"),
        safety_notes: r.get("safety_notes"),
        client_instructions: r.get("client_instructions"),
        issues: r.get("issues"),
        notes: r.get("notes"),
    }
}

#[async_trait]
impl ReportsRepository for PgReportsRepository {
    async fn stored(
        &self,
        scope: TenantScope,
        jobs: &JobScope,
        job: Option<JobId>,
    ) -> PortResult<Vec<StoredReport>> {
        if jobs.is_empty() {
            return Ok(Vec::new());
        }
        let mut tx = self.begin(scope).await?;
        let rows = sqlx::query(
            "SELECT id, job_id, type, title, content, generated_at, created_at FROM reports \
             WHERE ($1::int[] IS NULL OR job_id = ANY($1)) \
               AND ($2::int IS NULL OR job_id = $2) \
             ORDER BY created_at",
        )
        .bind(jobs.ids().as_deref())
        .bind(job.map(JobId::get))
        .fetch_all(&mut *tx)
        .await
        .map_err(map_sqlx)?;
        tx.commit().await.map_err(map_sqlx)?;

        Ok(rows
            .into_iter()
            .map(|r| StoredReport {
                id: ReportId(r.get("id")),
                job_id: JobId(r.get("job_id")),
                report_type: r.get("type"),
                title: r.get("title"),
                content: r.get("content"),
                generated_at: r.get("generated_at"),
                created_at: r.get("created_at"),
            })
            .collect())
    }

    async fn create_stored(
        &self,
        scope: TenantScope,
        input: &StoredReportInput,
    ) -> PortResult<StoredReport> {
        let mut tx = self.begin(scope).await?;
        let r = sqlx::query(
            "INSERT INTO reports (job_id, type, title, content, generated_at) \
             VALUES ($1,$2,$3,$4,NOW()) \
             RETURNING id, job_id, type, title, content, generated_at, created_at",
        )
        .bind(input.job_id.get())
        .bind(&input.report_type)
        .bind(&input.title)
        .bind(&input.content)
        .fetch_one(&mut *tx)
        .await
        .map_err(map_sqlx)?;
        tx.commit().await.map_err(map_sqlx)?;

        Ok(StoredReport {
            id: ReportId(r.get("id")),
            job_id: JobId(r.get("job_id")),
            report_type: r.get("type"),
            title: r.get("title"),
            content: r.get("content"),
            generated_at: r.get("generated_at"),
            created_at: r.get("created_at"),
        })
    }

    async fn meta(&self, scope: TenantScope, jobs: &JobScope) -> PortResult<ReportMeta> {
        if jobs.is_empty() {
            return Ok(ReportMeta::default());
        }
        let ids = jobs.ids();
        let mut tx = self.begin(scope).await?;

        let job_rows = sqlx::query(
            "SELECT id, name, job_number, status, address FROM jobs \
             WHERE ($1::int[] IS NULL OR id = ANY($1)) ORDER BY job_number",
        )
        .bind(ids.as_deref())
        .fetch_all(&mut *tx)
        .await
        .map_err(map_sqlx)?;

        // Active supervisors only: a departed one would clutter the dropdown
        // with a filter that can never match new work.
        let sup_rows = sqlx::query(
            "SELECT id, name FROM ( \
               SELECT u.id, u.name FROM jobs j \
               JOIN users u ON u.id = j.supervisor_id \
               WHERE u.role = 'SUPERVISOR' AND u.active \
                 AND ($1::int[] IS NULL OR j.id = ANY($1)) \
               UNION \
               SELECT u.id, u.name FROM job_assignments a \
               JOIN users u ON u.id = a.user_id \
               WHERE u.role = 'SUPERVISOR' AND u.active \
                 AND ($1::int[] IS NULL OR a.job_id = ANY($1)) \
             ) s ORDER BY name, id",
        )
        .bind(ids.as_deref())
        .fetch_all(&mut *tx)
        .await
        .map_err(map_sqlx)?;
        tx.commit().await.map_err(map_sqlx)?;

        Ok(ReportMeta {
            jobs: job_rows
                .into_iter()
                .map(|r| ReportMetaJob {
                    id: JobId(r.get("id")),
                    name: r.get::<Option<String>, _>("name").unwrap_or_default(),
                    job_number: r.get::<Option<String>, _>("job_number").unwrap_or_default(),
                    status: r.get("status"),
                    address: r.get("address"),
                })
                .collect(),
            supervisors: sup_rows
                .into_iter()
                .map(|r| CalendarFilterSupervisor {
                    id: UserId(r.get("id")),
                    name: r.get::<Option<String>, _>("name").unwrap_or_default(),
                })
                .collect(),
        })
    }

    async fn job_progress(
        &self,
        scope: TenantScope,
        jobs: &JobScope,
        filter: ReportFilter,
        today: chrono::NaiveDate,
    ) -> PortResult<Vec<JobProgressRow>> {
        if jobs.is_empty() {
            return Ok(Vec::new());
        }
        let mut tx = self.begin(scope).await?;
        // Counted with FILTER in one pass rather than by pulling every
        // call-forward row back and counting in the service.
        //
        // The date filter keeps active jobs regardless of their planned end:
        // that date is a forecast, and a live job dropping off the report
        // because its forecast has passed is exactly when it matters most.
        let rows = sqlx::query(&format!(
            "SELECT j.id, j.job_number, j.name, j.client, j.address, j.status, \
                    j.start_date, j.end_date, j.supervisor_id, u.name AS supervisor_name, \
                    COUNT(cf.id) FILTER (WHERE cf.item_type <> 'HEADER') AS total_tasks, \
                    COUNT(cf.id) FILTER (WHERE cf.item_type <> 'HEADER' \
                        AND cf.status = 'not_started') AS not_started, \
                    COUNT(cf.id) FILTER (WHERE cf.item_type <> 'HEADER' \
                        AND cf.status = 'in_progress') AS in_progress, \
                    COUNT(cf.id) FILTER (WHERE cf.item_type <> 'HEADER' \
                        AND cf.status = 'completed') AS completed, \
                    COUNT(cf.id) FILTER (WHERE cf.item_type <> 'HEADER' \
                        AND cf.status = 'on_hold') AS on_hold, \
                    COUNT(cf.id) FILTER (WHERE cf.item_type <> 'HEADER' \
                        AND cf.status NOT IN ('completed','on_hold') \
                        AND cf.est_finish IS NOT NULL AND cf.est_finish < $6) AS delayed_count \
             FROM jobs j \
             LEFT JOIN users u ON u.id = j.supervisor_id \
             LEFT JOIN call_forward cf ON cf.job_id = j.id \
             WHERE ($1::int[] IS NULL OR j.id = ANY($1)) \
               AND ($2::int IS NULL OR j.id = $2) \
               AND {SUPERVISOR_JOBS} \
               AND ( $2::int IS NOT NULL OR j.status = 'active' \
                     OR (($4::date IS NULL OR j.end_date IS NULL OR j.end_date >= $4) \
                         AND ($5::date IS NULL OR j.start_date <= $5) \
                         AND j.start_date IS NOT NULL) ) \
             GROUP BY j.id, u.name \
             ORDER BY j.job_number"
        ))
        .bind(jobs.ids().as_deref())
        .bind(filter.job_id.map(JobId::get))
        .bind(filter.supervisor_id.map(UserId::get))
        .bind(filter.range.from)
        .bind(filter.range.to)
        .bind(today)
        .fetch_all(&mut *tx)
        .await
        .map_err(map_sqlx)?;
        tx.commit().await.map_err(map_sqlx)?;

        Ok(rows
            .into_iter()
            .map(|r| JobProgressRow {
                job_id: JobId(r.get("id")),
                job_number: r.get::<Option<String>, _>("job_number").unwrap_or_default(),
                job_name: r.get::<Option<String>, _>("name").unwrap_or_default(),
                client: r.get("client"),
                address: r.get("address"),
                status: r.get("status"),
                start_date: r.get("start_date"),
                end_date: r.get("end_date"),
                supervisor_id: r.get::<Option<i32>, _>("supervisor_id").map(UserId),
                supervisor_name: r.get("supervisor_name"),
                total_tasks: r.get("total_tasks"),
                not_started: r.get("not_started"),
                in_progress: r.get("in_progress"),
                completed: r.get("completed"),
                on_hold: r.get("on_hold"),
                delayed_count: r.get("delayed_count"),
            })
            .collect())
    }

    async fn delays(
        &self,
        scope: TenantScope,
        jobs: &JobScope,
        filter: ReportFilter,
        today: chrono::NaiveDate,
    ) -> PortResult<Vec<DelayRow>> {
        if jobs.is_empty() {
            return Ok(Vec::new());
        }
        let mut tx = self.begin(scope).await?;
        // `actual_finish IS NULL` as well as the status test: an item finished
        // late is not an active delay, whatever its status column says.
        let rows = sqlx::query(&format!(
            "SELECT cf.id AS task_id, cf.title, cf.item_type, cf.supplier_trade, \
                    cf.job_id, j.job_number, j.name AS job_name, j.address AS job_address, \
                    j.supervisor_id, u.name AS supervisor_name, \
                    cf.est_finish, cf.actual_finish, cf.status, \
                    ($6::date - cf.est_finish) AS delay_days \
             FROM call_forward cf \
             JOIN jobs j ON j.id = cf.job_id \
             LEFT JOIN users u ON u.id = j.supervisor_id \
             WHERE cf.item_type <> 'HEADER' \
               AND cf.est_finish IS NOT NULL \
               AND cf.est_finish < $6 \
               AND cf.actual_finish IS NULL \
               AND cf.status NOT IN ('completed','on_hold') \
               AND ($1::int[] IS NULL OR cf.job_id = ANY($1)) \
               AND ($2::int IS NULL OR cf.job_id = $2) \
               AND {SUPERVISOR_JOBS} \
               AND ($4::date IS NULL OR cf.est_finish >= $4) \
               AND ($5::date IS NULL OR cf.est_finish <= $5) \
             ORDER BY delay_days DESC, cf.id"
        ))
        .bind(jobs.ids().as_deref())
        .bind(filter.job_id.map(JobId::get))
        .bind(filter.supervisor_id.map(UserId::get))
        .bind(filter.range.from)
        .bind(filter.range.to)
        .bind(today)
        .fetch_all(&mut *tx)
        .await
        .map_err(map_sqlx)?;
        tx.commit().await.map_err(map_sqlx)?;

        Ok(rows
            .into_iter()
            .map(|r| DelayRow {
                task_id: r.get("task_id"),
                title: r.get("title"),
                item_type: r.get("item_type"),
                supplier_trade: r.get("supplier_trade"),
                job_id: JobId(r.get("job_id")),
                job_number: r.get::<Option<String>, _>("job_number").unwrap_or_default(),
                job_name: r.get::<Option<String>, _>("job_name").unwrap_or_default(),
                job_address: r.get("job_address"),
                supervisor_id: r.get::<Option<i32>, _>("supervisor_id").map(UserId),
                supervisor_name: r.get("supervisor_name"),
                est_finish: r.get("est_finish"),
                actual_finish: r.get("actual_finish"),
                delay_days: i64::from(r.get::<i32, _>("delay_days")),
                status: r.get("status"),
            })
            .collect())
    }

    async fn diary_summary(
        &self,
        scope: TenantScope,
        jobs: &JobScope,
        filter: ReportFilter,
    ) -> PortResult<Vec<DiaryReportRow>> {
        if jobs.is_empty() {
            return Ok(Vec::new());
        }
        let mut tx = self.begin(scope).await?;
        let rows = sqlx::query(&format!(
            "SELECT sd.id, sd.date, sd.job_id, \
                    COALESCE(j.job_number, sd.job_id::text) AS job_number, \
                    COALESCE(j.name, 'Unknown') AS job_name, \
                    j.address AS job_address, sd.author_id, u.name AS author_name, \
                    sd.weather, sd.workforce, {DIARY_CATEGORY_AGG} \
             FROM site_diary sd \
             LEFT JOIN jobs j ON j.id = sd.job_id \
             LEFT JOIN users u ON u.id = sd.author_id \
             LEFT JOIN diary_notes dn ON dn.diary_entry_id = sd.id AND dn.archived = false \
             WHERE ($1::int[] IS NULL OR sd.job_id = ANY($1)) \
               AND ($2::int IS NULL OR sd.job_id = $2) \
               AND ($3::int IS NULL OR sd.author_id = $3) \
               AND ($4::date IS NULL OR sd.date >= $4) \
               AND ($5::date IS NULL OR sd.date <= $5) \
             GROUP BY sd.id, j.job_number, j.name, j.address, u.name \
             ORDER BY sd.date DESC, sd.id DESC"
        ))
        .bind(jobs.ids().as_deref())
        .bind(filter.job_id.map(JobId::get))
        .bind(filter.author_id.map(UserId::get))
        .bind(filter.range.from)
        .bind(filter.range.to)
        .fetch_all(&mut *tx)
        .await
        .map_err(map_sqlx)?;
        tx.commit().await.map_err(map_sqlx)?;
        Ok(rows.iter().map(diary_row).collect())
    }

    async fn upcoming_tasks(
        &self,
        scope: TenantScope,
        jobs: &JobScope,
        filter: ReportFilter,
        today: chrono::NaiveDate,
    ) -> PortResult<Vec<UpcomingTaskRow>> {
        if jobs.is_empty() {
            return Ok(Vec::new());
        }
        let mut tx = self.begin(scope).await?;
        let rows = sqlx::query(&format!(
            "SELECT cf.id AS task_id, cf.title, cf.item_type, cf.supplier_trade, \
                    cf.job_id, j.job_number, j.name AS job_name, j.address AS job_address, \
                    j.supervisor_id, u.name AS supervisor_name, \
                    cf.est_start, cf.est_finish, cf.status, \
                    (cf.est_start - $6::date) AS days_until_start, \
                    (cf.est_finish - $6::date) AS days_until_finish \
             FROM call_forward cf \
             JOIN jobs j ON j.id = cf.job_id \
             LEFT JOIN users u ON u.id = j.supervisor_id \
             WHERE cf.item_type <> 'HEADER' \
               AND cf.status NOT IN ('completed','on_hold') \
               AND cf.actual_finish IS NULL \
               AND (cf.est_start >= $6 OR cf.est_finish >= $6) \
               AND ($1::int[] IS NULL OR cf.job_id = ANY($1)) \
               AND ($2::int IS NULL OR cf.job_id = $2) \
               AND {SUPERVISOR_JOBS} \
               AND ($4::date IS NULL OR COALESCE(cf.est_start, cf.est_finish) >= $4) \
               AND ($5::date IS NULL OR COALESCE(cf.est_start, cf.est_finish) <= $5) \
             ORDER BY cf.est_start NULLS LAST, cf.est_finish NULLS LAST, cf.id"
        ))
        .bind(jobs.ids().as_deref())
        .bind(filter.job_id.map(JobId::get))
        .bind(filter.supervisor_id.map(UserId::get))
        .bind(filter.range.from)
        .bind(filter.range.to)
        .bind(today)
        .fetch_all(&mut *tx)
        .await
        .map_err(map_sqlx)?;
        tx.commit().await.map_err(map_sqlx)?;

        Ok(rows
            .into_iter()
            .map(|r| UpcomingTaskRow {
                task_id: r.get("task_id"),
                title: r.get("title"),
                item_type: r.get("item_type"),
                supplier_trade: r.get("supplier_trade"),
                job_id: JobId(r.get("job_id")),
                job_number: r.get::<Option<String>, _>("job_number").unwrap_or_default(),
                job_name: r.get::<Option<String>, _>("job_name").unwrap_or_default(),
                job_address: r.get("job_address"),
                supervisor_id: r.get::<Option<i32>, _>("supervisor_id").map(UserId),
                supervisor_name: r.get("supervisor_name"),
                est_start: r.get("est_start"),
                est_finish: r.get("est_finish"),
                days_until_start: r.get::<Option<i32>, _>("days_until_start").map(i64::from),
                days_until_finish: r.get::<Option<i32>, _>("days_until_finish").map(i64::from),
                status: r.get("status"),
            })
            .collect())
    }

    async fn stage_claims(
        &self,
        scope: TenantScope,
        jobs: &JobScope,
        filter: ReportFilter,
    ) -> PortResult<Vec<StageClaimRow>> {
        if jobs.is_empty() {
            return Ok(Vec::new());
        }
        let mut tx = self.begin(scope).await?;
        let rows = sqlx::query(&format!(
            "SELECT cf.id, cf.title, cf.job_id, j.job_number, j.name AS job_name, \
                    j.address AS job_address, u.name AS supervisor_name, cf.supplier_trade, \
                    cf.est_finish, cf.actual_finish, cf.status \
             FROM call_forward cf \
             JOIN jobs j ON j.id = cf.job_id \
             LEFT JOIN users u ON u.id = j.supervisor_id \
             WHERE cf.item_type = 'STAGE_CLAIM' \
               AND ($1::int[] IS NULL OR cf.job_id = ANY($1)) \
               AND ($2::int IS NULL OR cf.job_id = $2) \
               AND {SUPERVISOR_JOBS} \
               AND ($4::date IS NULL OR cf.est_finish >= $4) \
               AND ($5::date IS NULL OR cf.est_finish <= $5) \
             ORDER BY cf.est_finish NULLS LAST, cf.id"
        ))
        .bind(jobs.ids().as_deref())
        .bind(filter.job_id.map(JobId::get))
        .bind(filter.supervisor_id.map(UserId::get))
        .bind(filter.range.from)
        .bind(filter.range.to)
        .fetch_all(&mut *tx)
        .await
        .map_err(map_sqlx)?;
        tx.commit().await.map_err(map_sqlx)?;

        Ok(rows
            .into_iter()
            .map(|r| StageClaimRow {
                id: r.get("id"),
                title: r.get("title"),
                job_id: JobId(r.get("job_id")),
                job_number: r.get::<Option<String>, _>("job_number").unwrap_or_default(),
                job_name: r.get::<Option<String>, _>("job_name").unwrap_or_default(),
                job_address: r.get("job_address"),
                supervisor_name: r.get("supervisor_name"),
                supplier_trade: r.get("supplier_trade"),
                est_finish: r.get("est_finish"),
                actual_finish: r.get("actual_finish"),
                status: r.get("status"),
            })
            .collect())
    }

    async fn inspections(
        &self,
        scope: TenantScope,
        jobs: &JobScope,
        filter: ReportFilter,
    ) -> PortResult<Vec<InspectionRow>> {
        if jobs.is_empty() {
            return Ok(Vec::new());
        }
        let mut tx = self.begin(scope).await?;
        // Inspection status is derived from the entry's notes in the service,
        // so both columns are read even though production keeps the text in
        // `diary_notes` rather than here.
        let rows = sqlx::query(
            "SELECT sd.id, sd.date, sd.job_id, \
                    COALESCE(j.job_number, sd.job_id::text) AS job_number, \
                    COALESCE(j.name, 'Unknown') AS job_name, j.address AS job_address, \
                    u.name AS inspector, sd.workforce, \
                    COALESCE(NULLIF(string_agg(CASE WHEN dn.category = 'safety' \
                        AND dn.content <> '' THEN dn.content END, E'\\n' \
                        ORDER BY dn.sort_order, dn.created_at), ''), \
                        COALESCE(sd.safety_notes, '')) AS safety_notes, \
                    COALESCE(NULLIF(string_agg(CASE WHEN dn.category = 'issues' \
                        AND dn.content <> '' THEN dn.content END, E'\\n' \
                        ORDER BY dn.sort_order, dn.created_at), ''), \
                        COALESCE(sd.issues, '')) AS issues, \
                    COALESCE(NULLIF(string_agg(CASE WHEN dn.category = 'trades' \
                        AND dn.content <> '' THEN dn.content END, E'\\n' \
                        ORDER BY dn.sort_order, dn.created_at), ''), \
                        COALESCE(sd.trades_on_site, '')) AS trades_on_site \
             FROM site_diary sd \
             LEFT JOIN jobs j ON j.id = sd.job_id \
             LEFT JOIN users u ON u.id = sd.author_id \
             LEFT JOIN diary_notes dn ON dn.diary_entry_id = sd.id AND dn.archived = false \
             WHERE ($1::int[] IS NULL OR sd.job_id = ANY($1)) \
               AND ($2::int IS NULL OR sd.job_id = $2) \
               AND ($3::int IS NULL OR sd.author_id = $3) \
               AND ($4::date IS NULL OR sd.date >= $4) \
               AND ($5::date IS NULL OR sd.date <= $5) \
             GROUP BY sd.id, j.job_number, j.name, j.address, u.name \
             ORDER BY sd.date DESC, sd.id DESC",
        )
        .bind(jobs.ids().as_deref())
        .bind(filter.job_id.map(JobId::get))
        .bind(filter.supervisor_id.map(UserId::get))
        .bind(filter.range.from)
        .bind(filter.range.to)
        .fetch_all(&mut *tx)
        .await
        .map_err(map_sqlx)?;
        tx.commit().await.map_err(map_sqlx)?;

        Ok(rows
            .into_iter()
            .map(|r| InspectionRow {
                id: DiaryEntryId(r.get("id")),
                date: r.get("date"),
                job_id: JobId(r.get("job_id")),
                job_number: r.get("job_number"),
                job_name: r.get("job_name"),
                job_address: r.get("job_address"),
                inspector: r.get("inspector"),
                safety_notes: r.get("safety_notes"),
                issues: r.get("issues"),
                workforce: r.get("workforce"),
                trades_on_site: r.get("trades_on_site"),
            })
            .collect())
    }

    async fn weather(
        &self,
        scope: TenantScope,
        jobs: &JobScope,
        filter: ReportFilter,
    ) -> PortResult<Vec<WeatherRow>> {
        if jobs.is_empty() {
            return Ok(Vec::new());
        }
        let mut tx = self.begin(scope).await?;
        // The numerics are cast to float8 here rather than carried as Decimal:
        // they are weather measurements to one decimal place, feeding averages
        // and comparisons, not money.
        let rows = sqlx::query(
            "SELECT sd.id, sd.date, sd.job_id, \
                    COALESCE(j.job_number, sd.job_id::text) AS job_number, \
                    COALESCE(j.name, 'Unknown') AS job_name, u.name AS author_name, \
                    sd.location_name, COALESCE(sd.weather_condition, sd.weather) AS condition, \
                    sd.temperature::float8 AS temperature, \
                    sd.rainfall_mm::float8 AS rainfall_mm, \
                    sd.wind_speed_kmh::float8 AS wind_speed_kmh, \
                    sd.issues, sd.workforce \
             FROM site_diary sd \
             LEFT JOIN jobs j ON j.id = sd.job_id \
             LEFT JOIN users u ON u.id = sd.author_id \
             WHERE ($1::int[] IS NULL OR sd.job_id = ANY($1)) \
               AND ($2::int IS NULL OR sd.job_id = $2) \
               AND ($3::date IS NULL OR sd.date >= $3) \
               AND ($4::date IS NULL OR sd.date <= $4) \
             ORDER BY sd.date DESC, sd.id DESC",
        )
        .bind(jobs.ids().as_deref())
        .bind(filter.job_id.map(JobId::get))
        .bind(filter.range.from)
        .bind(filter.range.to)
        .fetch_all(&mut *tx)
        .await
        .map_err(map_sqlx)?;
        tx.commit().await.map_err(map_sqlx)?;

        Ok(rows
            .into_iter()
            .map(|r| WeatherRow {
                id: DiaryEntryId(r.get("id")),
                date: r.get("date"),
                job_id: JobId(r.get("job_id")),
                job_number: r.get("job_number"),
                job_name: r.get("job_name"),
                author_name: r.get("author_name"),
                location_name: r.get("location_name"),
                weather_condition: r.get("condition"),
                temperature: r.get("temperature"),
                rainfall_mm: r.get("rainfall_mm"),
                wind_speed_kmh: r.get("wind_speed_kmh"),
                issues: r.get("issues"),
                workforce: r.get("workforce"),
            })
            .collect())
    }

    async fn daily_progress(
        &self,
        scope: TenantScope,
        job: JobId,
        date: chrono::NaiveDate,
    ) -> PortResult<Option<DailyProgressData>> {
        let mut tx = self.begin(scope).await?;

        let job_row = sqlx::query(
            "SELECT j.id, j.job_number, j.name, j.client, j.supervisor_id, \
                    u.name AS supervisor_name \
             FROM jobs j LEFT JOIN users u ON u.id = j.supervisor_id WHERE j.id = $1",
        )
        .bind(job.get())
        .fetch_optional(&mut *tx)
        .await
        .map_err(map_sqlx)?;

        let Some(j) = job_row else {
            tx.commit().await.map_err(map_sqlx)?;
            return Ok(None);
        };

        let tasks = sqlx::query(
            "SELECT id, title, item_type, supplier_trade, est_start, est_finish, \
                    actual_start, actual_finish, status, sort_order \
             FROM call_forward WHERE job_id = $1 ORDER BY sort_order, id",
        )
        .bind(job.get())
        .fetch_all(&mut *tx)
        .await
        .map_err(map_sqlx)?;

        let diary = sqlx::query(&format!(
            "SELECT sd.id, sd.date, sd.job_id, \
                    COALESCE(j.job_number, sd.job_id::text) AS job_number, \
                    COALESCE(j.name, 'Unknown') AS job_name, j.address AS job_address, \
                    sd.author_id, u.name AS author_name, sd.weather, sd.workforce, \
                    {DIARY_CATEGORY_AGG} \
             FROM site_diary sd \
             LEFT JOIN jobs j ON j.id = sd.job_id \
             LEFT JOIN users u ON u.id = sd.author_id \
             LEFT JOIN diary_notes dn ON dn.diary_entry_id = sd.id AND dn.archived = false \
             WHERE sd.job_id = $1 AND sd.date = $2 \
             GROUP BY sd.id, j.job_number, j.name, j.address, u.name \
             ORDER BY sd.id DESC LIMIT 1"
        ))
        .bind(job.get())
        .bind(date)
        .fetch_optional(&mut *tx)
        .await
        .map_err(map_sqlx)?;

        tx.commit().await.map_err(map_sqlx)?;

        Ok(Some(DailyProgressData {
            job_id: JobId(j.get("id")),
            job_number: j.get::<Option<String>, _>("job_number").unwrap_or_default(),
            job_name: j.get::<Option<String>, _>("name").unwrap_or_default(),
            client: j.get("client"),
            supervisor_id: j.get::<Option<i32>, _>("supervisor_id").map(UserId),
            supervisor_name: j.get("supervisor_name"),
            tasks: tasks
                .into_iter()
                .map(|t| DailyTaskRow {
                    id: t.get("id"),
                    title: t.get("title"),
                    item_type: t.get("item_type"),
                    supplier_trade: t.get("supplier_trade"),
                    est_start: t.get("est_start"),
                    est_finish: t.get("est_finish"),
                    actual_start: t.get("actual_start"),
                    actual_finish: t.get("actual_finish"),
                    status: t.get("status"),
                    sort_order: t.get("sort_order"),
                })
                .collect(),
            diary: diary.as_ref().map(diary_row),
        }))
    }

    async fn supervisor_counts(
        &self,
        scope: TenantScope,
        filter: ReportFilter,
        today: chrono::NaiveDate,
    ) -> PortResult<Vec<SupervisorCounts>> {
        let mut tx = self.begin(scope).await?;

        // One row per active supervisor, with every count aggregated in SQL.
        // The legacy ran a query per supervisor per metric.
        let rows = sqlx::query(
            "WITH sup AS ( \
               SELECT id, name, email FROM users \
               WHERE role = 'SUPERVISOR' AND active AND ($1::int IS NULL OR id = $1) \
             ), sup_jobs AS ( \
               SELECT s.id AS sup_id, j.id AS job_id, j.status, j.start_date, j.end_date \
               FROM sup s JOIN jobs j ON j.supervisor_id = s.id \
                 OR EXISTS (SELECT 1 FROM job_assignments a \
                            WHERE a.job_id = j.id AND a.user_id = s.id) \
             ) \
             SELECT s.id, s.name, s.email, \
               (SELECT COUNT(*) FROM sup_jobs sj WHERE sj.sup_id = s.id \
                  AND sj.status = 'active') AS active_jobs, \
               (SELECT COUNT(*) FROM sup_jobs sj WHERE sj.sup_id = s.id \
                  AND sj.status = 'completed') AS completed_jobs, \
               (SELECT COUNT(*) FROM sup_jobs sj WHERE sj.sup_id = s.id) AS total_jobs, \
               (SELECT COUNT(*) FROM call_forward cf \
                  JOIN sup_jobs sj ON sj.job_id = cf.job_id AND sj.sup_id = s.id \
                  WHERE cf.item_type <> 'HEADER' AND cf.actual_start IS NOT NULL \
                    AND cf.est_start IS NOT NULL) AS started_tasks, \
               (SELECT COUNT(*) FROM call_forward cf \
                  JOIN sup_jobs sj ON sj.job_id = cf.job_id AND sj.sup_id = s.id \
                  WHERE cf.item_type <> 'HEADER' AND cf.actual_start IS NOT NULL \
                    AND cf.est_start IS NOT NULL \
                    AND cf.actual_start <= cf.est_start) AS started_on_time, \
               (SELECT COUNT(*) FROM call_forward cf \
                  JOIN sup_jobs sj ON sj.job_id = cf.job_id AND sj.sup_id = s.id \
                  WHERE cf.item_type <> 'HEADER' AND cf.actual_finish IS NOT NULL \
                    AND cf.est_finish IS NOT NULL) AS finished_tasks, \
               (SELECT COUNT(*) FROM call_forward cf \
                  JOIN sup_jobs sj ON sj.job_id = cf.job_id AND sj.sup_id = s.id \
                  WHERE cf.item_type <> 'HEADER' AND cf.actual_finish IS NOT NULL \
                    AND cf.est_finish IS NOT NULL \
                    AND cf.actual_finish <= cf.est_finish) AS finished_on_time, \
               (SELECT COUNT(*) FROM call_forward cf \
                  JOIN sup_jobs sj ON sj.job_id = cf.job_id AND sj.sup_id = s.id \
                  WHERE cf.item_type <> 'HEADER') AS total_tasks, \
               (SELECT COUNT(*) FROM call_forward cf \
                  JOIN sup_jobs sj ON sj.job_id = cf.job_id AND sj.sup_id = s.id \
                  WHERE cf.item_type <> 'HEADER' AND cf.actual_finish IS NULL \
                    AND cf.status NOT IN ('completed','on_hold') \
                    AND cf.est_finish IS NOT NULL AND cf.est_finish < $4) AS delayed_tasks, \
               COALESCE((SELECT SUM($4::date - cf.est_finish) FROM call_forward cf \
                  JOIN sup_jobs sj ON sj.job_id = cf.job_id AND sj.sup_id = s.id \
                  WHERE cf.item_type <> 'HEADER' AND cf.actual_finish IS NULL \
                    AND cf.status NOT IN ('completed','on_hold') \
                    AND cf.est_finish IS NOT NULL AND cf.est_finish < $4), 0) AS total_delay_days, \
               (SELECT COUNT(DISTINCT (sd.job_id, sd.date)) FROM site_diary sd \
                  JOIN sup_jobs sj ON sj.job_id = sd.job_id AND sj.sup_id = s.id \
                  WHERE ($2::date IS NULL OR sd.date >= $2) \
                    AND ($3::date IS NULL OR sd.date <= $3)) AS diary_entries \
             FROM sup s ORDER BY s.name, s.id",
        )
        .bind(filter.supervisor_id.map(UserId::get))
        .bind(filter.range.from)
        .bind(filter.range.to)
        .bind(today)
        .fetch_all(&mut *tx)
        .await
        .map_err(map_sqlx)?;

        // Job windows come back separately so the service can count working
        // days without SQL needing a weekday calendar.
        let windows = sqlx::query(
            "SELECT s.id AS sup_id, j.start_date, LEAST(COALESCE(j.end_date, $4), $4) AS end_date \
             FROM users s JOIN jobs j ON j.supervisor_id = s.id \
               OR EXISTS (SELECT 1 FROM job_assignments a \
                          WHERE a.job_id = j.id AND a.user_id = s.id) \
             WHERE s.role = 'SUPERVISOR' AND s.active \
               AND ($1::int IS NULL OR s.id = $1) \
               AND j.status IN ('active','completed') \
               AND j.start_date IS NOT NULL",
        )
        .bind(filter.supervisor_id.map(UserId::get))
        .bind(filter.range.from)
        .bind(filter.range.to)
        .bind(today)
        .fetch_all(&mut *tx)
        .await
        .map_err(map_sqlx)?;
        tx.commit().await.map_err(map_sqlx)?;

        let mut by_sup: std::collections::HashMap<
            i32,
            Vec<(chrono::NaiveDate, chrono::NaiveDate)>,
        > = std::collections::HashMap::new();
        for w in &windows {
            let start: chrono::NaiveDate = w.get("start_date");
            let end: chrono::NaiveDate = w.get("end_date");
            // Clipped to the requested range, so a filter narrows the
            // denominator as well as the numerator.
            let start = filter.range.from.map_or(start, |f| start.max(f));
            let end = filter.range.to.map_or(end, |t| end.min(t));
            by_sup
                .entry(w.get("sup_id"))
                .or_default()
                .push((start, end));
        }

        Ok(rows
            .into_iter()
            .map(|r| {
                let id: i32 = r.get("id");
                SupervisorCounts {
                    supervisor_id: UserId(id),
                    name: r.get::<Option<String>, _>("name").unwrap_or_default(),
                    email: r.get::<Option<String>, _>("email").unwrap_or_default(),
                    active_jobs: r.get("active_jobs"),
                    completed_jobs: r.get("completed_jobs"),
                    total_jobs: r.get("total_jobs"),
                    started_tasks: r.get("started_tasks"),
                    started_on_time: r.get("started_on_time"),
                    finished_tasks: r.get("finished_tasks"),
                    finished_on_time: r.get("finished_on_time"),
                    total_tasks: r.get("total_tasks"),
                    delayed_tasks: r.get("delayed_tasks"),
                    total_delay_days: r.get("total_delay_days"),
                    diary_entries: r.get("diary_entries"),
                    job_windows: by_sup.remove(&id).unwrap_or_default(),
                }
            })
            .collect())
    }
}
