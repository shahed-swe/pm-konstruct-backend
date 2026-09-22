//! `JobTaskRepository` over Postgres.
//!
//! `job_tasks` has no `company_id`; it is tenant-scoped through its job by the
//! RLS policy in migration 0004, so setting the GUC is what constrains it.

use async_trait::async_trait;
use pmk_domain::ids::JobId;
use pmk_domain::tenant::TenantScope;
use pmk_ports::repository::{JobTask, JobTaskInput, JobTaskRepository};
use pmk_ports::PortResult;
use sqlx::{PgPool, Postgres, Transaction};

use super::user::map_sqlx;
use crate::db::tenant::set_tenant;

const COLS: &str = "id, job_id, title, status, notes, sort_order, created_at, updated_at";

#[derive(Debug, sqlx::FromRow)]
struct Row {
    id: i32,
    job_id: i32,
    title: String,
    status: String,
    notes: Option<String>,
    sort_order: i32,
    created_at: chrono::DateTime<chrono::Utc>,
    updated_at: chrono::DateTime<chrono::Utc>,
}

impl From<Row> for JobTask {
    fn from(r: Row) -> Self {
        Self {
            id: r.id,
            job_id: JobId(r.job_id),
            title: r.title,
            status: r.status,
            notes: r.notes,
            sort_order: r.sort_order,
            created_at: r.created_at,
            updated_at: r.updated_at,
        }
    }
}

#[derive(Debug, Clone)]
pub struct PgJobTaskRepository {
    pool: PgPool,
}

impl PgJobTaskRepository {
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
impl JobTaskRepository for PgJobTaskRepository {
    async fn list(&self, scope: TenantScope, job: JobId) -> PortResult<Vec<JobTask>> {
        let mut tx = self.begin(scope).await?;
        let sql = format!("SELECT {COLS} FROM job_tasks WHERE job_id = $1 ORDER BY sort_order, id");
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
        input: &JobTaskInput,
    ) -> PortResult<JobTask> {
        let mut tx = self.begin(scope).await?;
        // Appends to the end when no explicit order is given.
        let sql = format!(
            "INSERT INTO job_tasks (job_id, title, status, notes, sort_order) \
             VALUES ($1, $2, COALESCE($3,'pending'), $4, \
               COALESCE($5, (SELECT COALESCE(MAX(sort_order), -1) + 1 \
                             FROM job_tasks WHERE job_id = $1))) \
             RETURNING {COLS}"
        );
        let row: Row = sqlx::query_as(&sql)
            .bind(job.get())
            .bind(&input.title)
            .bind(input.status.as_deref())
            .bind(input.notes.as_deref())
            .bind(input.sort_order)
            .fetch_one(&mut *tx)
            .await
            .map_err(map_sqlx)?;
        tx.commit().await.map_err(map_sqlx)?;
        Ok(row.into())
    }

    async fn find(&self, scope: TenantScope, id: i32) -> PortResult<Option<JobTask>> {
        let mut tx = self.begin(scope).await?;
        let sql = format!("SELECT {COLS} FROM job_tasks WHERE id = $1");
        let row: Option<Row> = sqlx::query_as(&sql)
            .bind(id)
            .fetch_optional(&mut *tx)
            .await
            .map_err(map_sqlx)?;
        tx.commit().await.map_err(map_sqlx)?;
        Ok(row.map(Into::into))
    }

    async fn update(
        &self,
        scope: TenantScope,
        id: i32,
        input: &JobTaskInput,
    ) -> PortResult<Option<JobTask>> {
        let mut tx = self.begin(scope).await?;
        let sql = format!(
            "UPDATE job_tasks SET title = $2, status = COALESCE($3, status), \
                notes = $4, sort_order = COALESCE($5, sort_order), updated_at = NOW() \
             WHERE id = $1 RETURNING {COLS}"
        );
        let row: Option<Row> = sqlx::query_as(&sql)
            .bind(id)
            .bind(&input.title)
            .bind(input.status.as_deref())
            .bind(input.notes.as_deref())
            .bind(input.sort_order)
            .fetch_optional(&mut *tx)
            .await
            .map_err(map_sqlx)?;
        tx.commit().await.map_err(map_sqlx)?;
        Ok(row.map(Into::into))
    }

    async fn update_notes(
        &self,
        scope: TenantScope,
        id: i32,
        notes: Option<&str>,
    ) -> PortResult<Option<JobTask>> {
        let mut tx = self.begin(scope).await?;
        let sql = format!(
            "UPDATE job_tasks SET notes = $2, updated_at = NOW() WHERE id = $1 RETURNING {COLS}"
        );
        let row: Option<Row> = sqlx::query_as(&sql)
            .bind(id)
            .bind(notes)
            .fetch_optional(&mut *tx)
            .await
            .map_err(map_sqlx)?;
        tx.commit().await.map_err(map_sqlx)?;
        Ok(row.map(Into::into))
    }

    async fn delete(&self, scope: TenantScope, id: i32) -> PortResult<bool> {
        let mut tx = self.begin(scope).await?;
        let r = sqlx::query("DELETE FROM job_tasks WHERE id = $1")
            .bind(id)
            .execute(&mut *tx)
            .await
            .map_err(map_sqlx)?;
        tx.commit().await.map_err(map_sqlx)?;
        Ok(r.rows_affected() > 0)
    }
}
