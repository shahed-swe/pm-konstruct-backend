//! `JobLinkRepository` over Postgres.
//!
//! Backed by `job_dropbox_folders`. The table and column names are historical
//! and deliberately unchanged so the data migration needs no transform; the
//! rows are cloud-storage URLs from any provider.

use async_trait::async_trait;
use pmk_domain::ids::JobId;
use pmk_domain::job::{JobLink, JobLinkInput};
use pmk_domain::tenant::TenantScope;
use pmk_ports::repository::JobLinkRepository;
use pmk_ports::{PortError, PortResult};
use sqlx::{PgPool, Postgres, Transaction};

use super::user::map_sqlx;
use crate::db::tenant::set_tenant;

// `path` holds the URL; `label` is what the user sees.
const COLS: &str = "id, job_id, label, path, sort_order, created_at";

#[derive(Debug, sqlx::FromRow)]
struct Row {
    id: i32,
    job_id: i32,
    label: String,
    path: String,
    sort_order: i32,
    created_at: chrono::DateTime<chrono::Utc>,
}

impl From<Row> for JobLink {
    fn from(r: Row) -> Self {
        Self {
            id: r.id,
            job_id: JobId(r.job_id),
            label: r.label,
            url: r.path,
            sort_order: r.sort_order,
            created_at: r.created_at,
        }
    }
}

#[derive(Debug, Clone)]
pub struct PgJobLinkRepository {
    pool: PgPool,
}

impl PgJobLinkRepository {
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
impl JobLinkRepository for PgJobLinkRepository {
    async fn list(&self, scope: TenantScope, job: JobId) -> PortResult<Vec<JobLink>> {
        let mut tx = self.begin(scope).await?;
        let sql = format!(
            "SELECT {COLS} FROM job_dropbox_folders WHERE job_id = $1 ORDER BY sort_order, id"
        );
        let rows: Vec<Row> = sqlx::query_as(&sql)
            .bind(job.get())
            .fetch_all(&mut *tx)
            .await
            .map_err(map_sqlx)?;
        tx.commit().await.map_err(map_sqlx)?;
        Ok(rows.into_iter().map(Into::into).collect())
    }

    async fn create(
        &self,
        scope: TenantScope,
        job: JobId,
        input: &JobLinkInput,
    ) -> PortResult<JobLink> {
        let mut tx = self.begin(scope).await?;
        // company_id is denormalised on this table, so it is taken from the
        // job rather than the caller -- RLS already guarantees they agree.
        let sql = format!(
            "INSERT INTO job_dropbox_folders (job_id, company_id, label, path, sort_order) \
             SELECT $1, j.company_id, $2, $3, \
               COALESCE($4, (SELECT COALESCE(MAX(sort_order), -1) + 1 \
                             FROM job_dropbox_folders WHERE job_id = $1)) \
             FROM jobs j WHERE j.id = $1 \
             RETURNING {COLS}"
        );
        let row: Option<Row> = sqlx::query_as(&sql)
            .bind(job.get())
            .bind(&input.label)
            .bind(&input.url)
            .bind(input.sort_order)
            .fetch_optional(&mut *tx)
            .await
            .map_err(map_sqlx)?;
        // No row means the job is invisible under RLS, so the SELECT matched
        // nothing -- a 404 rather than a foreign-key error.
        let row = row.ok_or(PortError::NotFound)?;
        tx.commit().await.map_err(map_sqlx)?;
        Ok(row.into())
    }

    async fn update(
        &self,
        scope: TenantScope,
        id: i32,
        input: &JobLinkInput,
    ) -> PortResult<Option<JobLink>> {
        let mut tx = self.begin(scope).await?;
        let sql = format!(
            "UPDATE job_dropbox_folders SET label = $2, path = $3, \
                sort_order = COALESCE($4, sort_order) \
             WHERE id = $1 RETURNING {COLS}"
        );
        let row: Option<Row> = sqlx::query_as(&sql)
            .bind(id)
            .bind(&input.label)
            .bind(&input.url)
            .bind(input.sort_order)
            .fetch_optional(&mut *tx)
            .await
            .map_err(map_sqlx)?;
        tx.commit().await.map_err(map_sqlx)?;
        Ok(row.map(Into::into))
    }

    async fn delete(&self, scope: TenantScope, id: i32) -> PortResult<bool> {
        let mut tx = self.begin(scope).await?;
        let r = sqlx::query("DELETE FROM job_dropbox_folders WHERE id = $1")
            .bind(id)
            .execute(&mut *tx)
            .await
            .map_err(map_sqlx)?;
        tx.commit().await.map_err(map_sqlx)?;
        Ok(r.rows_affected() > 0)
    }
}
