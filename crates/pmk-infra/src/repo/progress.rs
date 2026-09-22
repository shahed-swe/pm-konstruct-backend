//! `ProgressRepository` over Postgres.
//!
//! `progress` carries no `company_id`; RLS reaches it through `jobs`, and the
//! `JobScope` narrows it further to what the caller may see.

use async_trait::async_trait;
use pmk_domain::dashboard::JobScope;
use pmk_domain::ids::{JobId, ProgressId};
use pmk_domain::progress::{Progress, ProgressInput};
use pmk_domain::tenant::TenantScope;
use pmk_ports::repository::ProgressRepository;
use pmk_ports::PortResult;
use sqlx::{PgPool, Postgres, Row, Transaction};

use super::user::map_sqlx;
use crate::db::tenant::set_tenant;

const COLS: &str = "id, job_id, date, percent_complete, milestone, description, photos, \
                    created_at, updated_at";

fn row(r: &sqlx::postgres::PgRow) -> Progress {
    Progress {
        id: ProgressId(r.get("id")),
        job_id: JobId(r.get("job_id")),
        date: r.get("date"),
        percent_complete: r.get("percent_complete"),
        milestone: r.get("milestone"),
        description: r.get("description"),
        photos: r
            .get::<Option<Vec<String>>, _>("photos")
            .unwrap_or_default(),
        created_at: r.get("created_at"),
        updated_at: r.get("updated_at"),
    }
}

#[derive(Debug, Clone)]
pub struct PgProgressRepository {
    pool: PgPool,
}

impl PgProgressRepository {
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
impl ProgressRepository for PgProgressRepository {
    async fn list(
        &self,
        scope: TenantScope,
        jobs: &JobScope,
        job: Option<JobId>,
    ) -> PortResult<Vec<Progress>> {
        if jobs.is_empty() {
            return Ok(Vec::new());
        }
        let mut tx = self.begin(scope).await?;
        let rows = sqlx::query(&format!(
            "SELECT {COLS} FROM progress \
             WHERE ($1::int[] IS NULL OR job_id = ANY($1)) \
               AND ($2::int IS NULL OR job_id = $2) \
             ORDER BY date, id"
        ))
        .bind(jobs.ids().as_deref())
        .bind(job.map(JobId::get))
        .fetch_all(&mut *tx)
        .await
        .map_err(map_sqlx)?;
        tx.commit().await.map_err(map_sqlx)?;
        Ok(rows.iter().map(row).collect())
    }

    async fn find(&self, scope: TenantScope, id: ProgressId) -> PortResult<Option<Progress>> {
        let mut tx = self.begin(scope).await?;
        let r = sqlx::query(&format!("SELECT {COLS} FROM progress WHERE id = $1"))
            .bind(id.get())
            .fetch_optional(&mut *tx)
            .await
            .map_err(map_sqlx)?;
        tx.commit().await.map_err(map_sqlx)?;
        Ok(r.as_ref().map(row))
    }

    async fn create(&self, scope: TenantScope, input: &ProgressInput) -> PortResult<Progress> {
        let mut tx = self.begin(scope).await?;
        let r = sqlx::query(&format!(
            "INSERT INTO progress (job_id, date, percent_complete, milestone, description, photos) \
             VALUES ($1,$2,$3,$4,$5,$6) RETURNING {COLS}"
        ))
        .bind(input.job_id.get())
        .bind(input.date)
        .bind(input.percent_complete)
        .bind(input.milestone.as_deref())
        .bind(input.description.as_deref())
        .bind(&input.photos)
        .fetch_one(&mut *tx)
        .await
        .map_err(map_sqlx)?;
        tx.commit().await.map_err(map_sqlx)?;
        Ok(row(&r))
    }

    async fn update(
        &self,
        scope: TenantScope,
        id: ProgressId,
        input: &ProgressInput,
    ) -> PortResult<Option<Progress>> {
        let mut tx = self.begin(scope).await?;
        // `job_id` is absent from the SET list on purpose: moving a record to
        // another job would rewrite that job's history.
        let r = sqlx::query(&format!(
            "UPDATE progress SET date = $2, percent_complete = $3, milestone = $4, \
                    description = $5, photos = $6, updated_at = NOW() \
             WHERE id = $1 RETURNING {COLS}"
        ))
        .bind(id.get())
        .bind(input.date)
        .bind(input.percent_complete)
        .bind(input.milestone.as_deref())
        .bind(input.description.as_deref())
        .bind(&input.photos)
        .fetch_optional(&mut *tx)
        .await
        .map_err(map_sqlx)?;
        tx.commit().await.map_err(map_sqlx)?;
        Ok(r.as_ref().map(row))
    }

    async fn delete(&self, scope: TenantScope, id: ProgressId) -> PortResult<bool> {
        let mut tx = self.begin(scope).await?;
        let n = sqlx::query("DELETE FROM progress WHERE id = $1")
            .bind(id.get())
            .execute(&mut *tx)
            .await
            .map_err(map_sqlx)?
            .rows_affected();
        tx.commit().await.map_err(map_sqlx)?;
        Ok(n > 0)
    }
}
