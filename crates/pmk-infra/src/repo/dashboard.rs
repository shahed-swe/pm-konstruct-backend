//! `DashboardRepository` over Postgres.
//!
//! Every query is scoped twice: RLS confines it to the company, and a job-id
//! list confines it further to what R3 makes visible to the caller. The second
//! is not redundant -- a supervisor is inside the tenant but must not see
//! counts for jobs they are not on.
//!
//! The legacy code ran the job filter as `inArray(...)` with the ids inlined,
//! which meant a company with 500 jobs sent 500 integers per query. Here the
//! list is bound once as an array and the predicate is
//! `($2::int[] IS NULL OR id = ANY($2))`, so the same SQL serves both scopes
//! and the plan is cached.

use async_trait::async_trait;
use pmk_domain::dashboard::{ActionItem, DashboardStats, JobScope};
use pmk_domain::ids::{DiaryEntryId, JobId, UserId};
use pmk_domain::tenant::TenantScope;
use pmk_ports::repository::{
    DashboardCallForward, DashboardDiaryEntry, DashboardJob, DashboardRepository, JobListFilter,
    UpcomingClaim,
};
use pmk_ports::PortResult;
use sqlx::{PgPool, Postgres, Row, Transaction};

use super::user::map_sqlx;
use crate::db::tenant::set_tenant;

#[derive(Debug, Clone)]
pub struct PgDashboardRepository {
    pool: PgPool,
}

impl PgDashboardRepository {
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

/// Maps a call-forward drill-down row.
fn cf_row(r: &sqlx::postgres::PgRow) -> DashboardCallForward {
    DashboardCallForward {
        id: r.get("id"),
        job_id: JobId(r.get("job_id")),
        job_number: r.get("job_number"),
        job_address: r.get("job_address"),
        title: r.get("title"),
        item_type: r.get("item_type"),
        est_start: r.get("est_start"),
        est_finish: r.get("est_finish"),
        status: r.get("status"),
    }
}

const CF_COLS: &str = "cf.id, cf.job_id, j.job_number, j.address AS job_address, \
                       cf.title, cf.item_type, cf.est_start, cf.est_finish, cf.status";

#[async_trait]
impl DashboardRepository for PgDashboardRepository {
    async fn stats(
        &self,
        scope: TenantScope,
        jobs: &JobScope,
        diary_since: chrono::NaiveDate,
        include_user_count: bool,
    ) -> PortResult<DashboardStats> {
        if jobs.is_empty() {
            // A supervisor with no jobs: every count is zero by definition, so
            // there is nothing worth asking the database.
            return Ok(DashboardStats {
                total_jobs: 0,
                active_jobs: 0,
                completed_jobs: 0,
                open_call_forwards: 0,
                overdue_call_forwards: 0,
                total_users: if include_user_count { Some(0) } else { None },
                recent_diary_entries: 0,
            });
        }

        let ids = jobs.ids();
        let mut tx = self.begin(scope).await?;

        let j = sqlx::query(
            "SELECT count(*) AS total, \
                    count(*) FILTER (WHERE status = 'active') AS active, \
                    count(*) FILTER (WHERE status = 'completed') AS completed \
             FROM jobs WHERE ($1::int[] IS NULL OR id = ANY($1))",
        )
        .bind(ids.as_deref())
        .fetch_one(&mut *tx)
        .await
        .map_err(map_sqlx)?;

        // HEADER rows are grouping labels in the call-forward tree, not work,
        // so they are excluded from every count and list here.
        let cf = sqlx::query(
            "SELECT \
               count(*) FILTER ( \
                 WHERE status IN ('not_started','in_progress') AND item_type <> 'HEADER' \
               ) AS open, \
               count(*) FILTER ( \
                 WHERE status NOT IN ('completed','on_hold') AND item_type <> 'HEADER' \
                   AND est_finish IS NOT NULL AND est_finish < CURRENT_DATE \
                   AND actual_finish IS NULL \
               ) AS overdue \
             FROM call_forward WHERE ($1::int[] IS NULL OR job_id = ANY($1))",
        )
        .bind(ids.as_deref())
        .fetch_one(&mut *tx)
        .await
        .map_err(map_sqlx)?;

        let diary: i64 = sqlx::query_scalar(
            "SELECT count(*) FROM site_diary \
             WHERE date >= $1 AND ($2::int[] IS NULL OR job_id = ANY($2))",
        )
        .bind(diary_since)
        .bind(ids.as_deref())
        .fetch_one(&mut *tx)
        .await
        .map_err(map_sqlx)?;

        // Headcount is a manager-only figure; RLS already confines it to the
        // company, so no extra predicate is needed.
        let total_users = if include_user_count {
            Some(
                sqlx::query_scalar::<_, i64>("SELECT count(*) FROM users")
                    .fetch_one(&mut *tx)
                    .await
                    .map_err(map_sqlx)?,
            )
        } else {
            None
        };

        tx.commit().await.map_err(map_sqlx)?;

        Ok(DashboardStats {
            total_jobs: j.get("total"),
            active_jobs: j.get("active"),
            completed_jobs: j.get("completed"),
            open_call_forwards: cf.get("open"),
            overdue_call_forwards: cf.get("overdue"),
            total_users,
            recent_diary_entries: diary,
        })
    }

    async fn jobs_list(
        &self,
        scope: TenantScope,
        jobs: &JobScope,
        filter: JobListFilter,
    ) -> PortResult<Vec<DashboardJob>> {
        if jobs.is_empty() {
            return Ok(Vec::new());
        }
        let status = match filter {
            JobListFilter::All => None,
            JobListFilter::Active => Some("active"),
            JobListFilter::Completed => Some("completed"),
        };
        let mut tx = self.begin(scope).await?;
        let rows = sqlx::query(
            "SELECT id, job_number, address, name, status, start_date, end_date \
             FROM jobs \
             WHERE ($1::int[] IS NULL OR id = ANY($1)) \
               AND ($2::text IS NULL OR status = $2) \
             ORDER BY updated_at DESC",
        )
        .bind(jobs.ids().as_deref())
        .bind(status)
        .fetch_all(&mut *tx)
        .await
        .map_err(map_sqlx)?;
        tx.commit().await.map_err(map_sqlx)?;

        Ok(rows
            .into_iter()
            .map(|r| DashboardJob {
                id: JobId(r.get("id")),
                job_number: r.get("job_number"),
                address: r.get("address"),
                name: r.get("name"),
                status: r.get("status"),
                start_date: r.get("start_date"),
                end_date: r.get("end_date"),
            })
            .collect())
    }

    async fn open_call_forwards(
        &self,
        scope: TenantScope,
        jobs: &JobScope,
    ) -> PortResult<Vec<DashboardCallForward>> {
        if jobs.is_empty() {
            return Ok(Vec::new());
        }
        let mut tx = self.begin(scope).await?;
        let rows = sqlx::query(&format!(
            "SELECT {CF_COLS} FROM call_forward cf \
             JOIN jobs j ON j.id = cf.job_id \
             WHERE cf.item_type <> 'HEADER' \
               AND cf.status IN ('not_started','in_progress') \
               AND ($1::int[] IS NULL OR cf.job_id = ANY($1)) \
             ORDER BY cf.est_start"
        ))
        .bind(jobs.ids().as_deref())
        .fetch_all(&mut *tx)
        .await
        .map_err(map_sqlx)?;
        tx.commit().await.map_err(map_sqlx)?;
        Ok(rows.iter().map(cf_row).collect())
    }

    async fn overdue_call_forwards(
        &self,
        scope: TenantScope,
        jobs: &JobScope,
        today: chrono::NaiveDate,
    ) -> PortResult<Vec<DashboardCallForward>> {
        if jobs.is_empty() {
            return Ok(Vec::new());
        }
        let mut tx = self.begin(scope).await?;
        let rows = sqlx::query(&format!(
            "SELECT {CF_COLS} FROM call_forward cf \
             JOIN jobs j ON j.id = cf.job_id \
             WHERE cf.item_type <> 'HEADER' \
               AND cf.actual_finish IS NULL \
               AND cf.status NOT IN ('completed','on_hold') \
               AND cf.est_finish < $2 \
               AND ($1::int[] IS NULL OR cf.job_id = ANY($1)) \
             ORDER BY cf.est_finish"
        ))
        .bind(jobs.ids().as_deref())
        .bind(today)
        .fetch_all(&mut *tx)
        .await
        .map_err(map_sqlx)?;
        tx.commit().await.map_err(map_sqlx)?;
        Ok(rows.iter().map(cf_row).collect())
    }

    async fn overdue_days(
        &self,
        scope: TenantScope,
        jobs: &JobScope,
        today: chrono::NaiveDate,
    ) -> PortResult<Vec<i64>> {
        if jobs.is_empty() {
            return Ok(Vec::new());
        }
        let mut tx = self.begin(scope).await?;
        // Subtracted in SQL so "how late" is computed in one place against one
        // date, rather than per row in the service.
        let rows: Vec<i32> = sqlx::query_scalar(
            "SELECT ($2::date - est_finish) FROM call_forward \
             WHERE item_type <> 'HEADER' AND actual_finish IS NULL \
               AND status NOT IN ('completed','on_hold') \
               AND est_finish < $2 \
               AND ($1::int[] IS NULL OR job_id = ANY($1))",
        )
        .bind(jobs.ids().as_deref())
        .bind(today)
        .fetch_all(&mut *tx)
        .await
        .map_err(map_sqlx)?;
        tx.commit().await.map_err(map_sqlx)?;
        Ok(rows.into_iter().map(i64::from).collect())
    }

    async fn recent_diary(
        &self,
        scope: TenantScope,
        jobs: &JobScope,
        since: chrono::NaiveDate,
    ) -> PortResult<Vec<DashboardDiaryEntry>> {
        if jobs.is_empty() {
            return Ok(Vec::new());
        }
        let mut tx = self.begin(scope).await?;
        let rows = sqlx::query(
            "SELECT sd.id, sd.job_id, j.job_number, j.address AS job_address, \
                    sd.date, sd.work_completed, u.name AS author_name \
             FROM site_diary sd \
             JOIN jobs j ON j.id = sd.job_id \
             LEFT JOIN users u ON u.id = sd.author_id \
             WHERE sd.date >= $2 AND ($1::int[] IS NULL OR sd.job_id = ANY($1)) \
             ORDER BY sd.date DESC",
        )
        .bind(jobs.ids().as_deref())
        .bind(since)
        .fetch_all(&mut *tx)
        .await
        .map_err(map_sqlx)?;
        tx.commit().await.map_err(map_sqlx)?;

        Ok(rows
            .into_iter()
            .map(|r| DashboardDiaryEntry {
                id: DiaryEntryId(r.get("id")),
                job_id: JobId(r.get("job_id")),
                job_number: r.get("job_number"),
                job_address: r.get("job_address"),
                date: r.get("date"),
                work_completed: r.get("work_completed"),
                author_name: r.get("author_name"),
            })
            .collect())
    }

    async fn upcoming_claims(
        &self,
        scope: TenantScope,
        jobs: &JobScope,
        cutoff: chrono::NaiveDate,
    ) -> PortResult<Vec<UpcomingClaim>> {
        if jobs.is_empty() {
            return Ok(Vec::new());
        }
        let mut tx = self.begin(scope).await?;
        // No lower bound, deliberately: an overdue claim must stay on the
        // dashboard until someone marks it complete.
        let rows = sqlx::query(
            "SELECT cf.id, cf.job_id, j.name AS job_name, j.address AS job_address, \
                    j.job_number, cf.title, cf.est_start, cf.est_finish, \
                    cf.actual_start, cf.actual_finish, cf.status, cf.notes \
             FROM call_forward cf \
             JOIN jobs j ON j.id = cf.job_id \
             WHERE cf.item_type = 'STAGE_CLAIM' \
               AND cf.status <> 'completed' \
               AND cf.actual_finish IS NULL \
               AND cf.est_finish <= $2 \
               AND ($1::int[] IS NULL OR cf.job_id = ANY($1)) \
             ORDER BY cf.est_finish ASC, cf.job_id ASC",
        )
        .bind(jobs.ids().as_deref())
        .bind(cutoff)
        .fetch_all(&mut *tx)
        .await
        .map_err(map_sqlx)?;
        tx.commit().await.map_err(map_sqlx)?;

        Ok(rows
            .into_iter()
            .map(|r| UpcomingClaim {
                id: r.get("id"),
                job_id: JobId(r.get("job_id")),
                job_name: r.get("job_name"),
                job_address: r.get("job_address"),
                job_number: r.get("job_number"),
                title: r.get("title"),
                est_start: r.get("est_start"),
                est_finish: r.get("est_finish"),
                actual_start: r.get("actual_start"),
                actual_finish: r.get("actual_finish"),
                status: r.get("status"),
                notes: r.get("notes"),
            })
            .collect())
    }

    async fn action_items(
        &self,
        scope: TenantScope,
        jobs: &JobScope,
    ) -> PortResult<Vec<ActionItem>> {
        if jobs.is_empty() {
            return Ok(Vec::new());
        }
        let mut tx = self.begin(scope).await?;
        // The LATERAL pulls each note's newest comment in the same pass; the
        // legacy code did the same, and without it this would be N+1 over
        // every actionable note in the company.
        let rows = sqlx::query(
            "SELECT dn.id AS note_id, dn.diary_entry_id, dn.category, dn.content, \
                    dn.action_status, dn.action_raised_by, dn.created_at, \
                    sd.date AS entry_date, sd.job_id, \
                    j.name AS job_name, j.job_number, j.address AS job_address, \
                    sd.author_id, u.name AS author_name, \
                    lc.content AS latest_comment_content, \
                    lc.author_name AS latest_comment_author, \
                    lc.created_at AS latest_comment_at \
             FROM diary_notes dn \
             JOIN site_diary sd ON sd.id = dn.diary_entry_id \
             JOIN jobs j ON j.id = sd.job_id \
             LEFT JOIN users u ON u.id = sd.author_id \
             LEFT JOIN LATERAL ( \
               SELECT c.content, c.created_at, u2.name AS author_name \
               FROM diary_note_comments c \
               LEFT JOIN users u2 ON u2.id = c.author_id \
               WHERE c.note_id = dn.id \
               ORDER BY c.created_at DESC LIMIT 1 \
             ) lc ON true \
             WHERE dn.archived = false \
               AND dn.action_status IN ('action','processing','completed') \
               AND ($1::int[] IS NULL OR sd.job_id = ANY($1)) \
             ORDER BY sd.date DESC, dn.created_at DESC",
        )
        .bind(jobs.ids().as_deref())
        .fetch_all(&mut *tx)
        .await
        .map_err(map_sqlx)?;
        tx.commit().await.map_err(map_sqlx)?;

        Ok(rows
            .into_iter()
            .map(|r| ActionItem {
                note_id: r.get("note_id"),
                diary_entry_id: r.get("diary_entry_id"),
                category: r.get("category"),
                content: r.get("content"),
                action_status: r.get("action_status"),
                action_raised_by: r.get::<Option<i32>, _>("action_raised_by").map(UserId),
                created_at: r.get("created_at"),
                entry_date: r.get("entry_date"),
                job_id: JobId(r.get("job_id")),
                job_name: r.get("job_name"),
                job_number: r.get("job_number"),
                job_address: r.get("job_address"),
                author_id: r.get::<Option<i32>, _>("author_id").map(UserId),
                author_name: r.get("author_name"),
                latest_comment_content: r.get("latest_comment_content"),
                latest_comment_author: r.get("latest_comment_author"),
                latest_comment_at: r.get("latest_comment_at"),
            })
            .collect())
    }
}
