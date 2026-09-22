//! `CallForwardRepository` over Postgres.
//!
//! The heaviest table in production (1,687 rows). Tenant-scoped through its job
//! by the RLS policy in 0004; job-level visibility (R3) is layered on top.

use async_trait::async_trait;
use pmk_domain::call_forward::{CallForwardInput, CallForwardItem, CfStatus, ItemType};
use pmk_domain::ids::{CallForwardItemId, JobId};
use pmk_domain::tenant::TenantScope;
use pmk_ports::repository::{CallForwardFilter, CallForwardRepository, ReorderEntry};
use pmk_ports::{PortError, PortResult};
use sqlx::{PgPool, Postgres, Transaction};

use super::user::map_sqlx;
use crate::db::tenant::set_tenant;

const COLS: &str = "id, job_id, title, item_type, supplier_trade, est_start, est_finish, \
     actual_start, actual_finish, status, notes, sort_order, parent_id, created_at, updated_at";

#[derive(Debug, sqlx::FromRow)]
struct Row {
    id: i32,
    job_id: i32,
    title: String,
    item_type: String,
    supplier_trade: Option<String>,
    est_start: Option<chrono::NaiveDate>,
    est_finish: Option<chrono::NaiveDate>,
    actual_start: Option<chrono::NaiveDate>,
    actual_finish: Option<chrono::NaiveDate>,
    status: String,
    notes: Option<String>,
    sort_order: i32,
    parent_id: Option<i32>,
    created_at: chrono::DateTime<chrono::Utc>,
    updated_at: chrono::DateTime<chrono::Utc>,
}

impl From<Row> for CallForwardItem {
    fn from(r: Row) -> Self {
        Self {
            id: CallForwardItemId(r.id),
            job_id: JobId(r.job_id),
            title: r.title,
            // Constrained by `call_forward_item_type_check`; default rather
            // than fail a whole list read on one corrupt row.
            item_type: ItemType::parse(&r.item_type).unwrap_or(ItemType::Task),
            supplier_trade: r.supplier_trade,
            est_start: r.est_start,
            est_finish: r.est_finish,
            actual_start: r.actual_start,
            actual_finish: r.actual_finish,
            // An unrecognised status falls through to the delay engine's
            // date-based inference, exactly as the legacy `if` chain did.
            status: CfStatus::parse(&r.status),
            notes: r.notes,
            sort_order: r.sort_order,
            parent_id: r.parent_id.map(CallForwardItemId),
            created_at: r.created_at,
            updated_at: r.updated_at,
        }
    }
}

#[derive(Debug, Clone)]
pub struct PgCallForwardRepository {
    pool: PgPool,
}

impl PgCallForwardRepository {
    #[must_use]
    pub const fn new(pool: PgPool) -> Self {
        Self { pool }
    }

    async fn begin(&self, scope: TenantScope) -> PortResult<Transaction<'_, Postgres>> {
        let mut tx = self.pool.begin().await.map_err(map_sqlx)?;
        set_tenant(&mut tx, scope).await.map_err(map_sqlx)?;
        Ok(tx)
    }

    fn job_ids(visible: Option<&[JobId]>) -> Option<Vec<i32>> {
        visible.map(|ids| ids.iter().map(|j| j.get()).collect())
    }

    /// Inserts one item inside an existing transaction.
    async fn insert_in(
        tx: &mut Transaction<'_, Postgres>,
        job: JobId,
        input: &CallForwardInput,
        parent: Option<i32>,
    ) -> PortResult<CallForwardItem> {
        let sql = format!(
            "INSERT INTO call_forward (job_id, title, item_type, supplier_trade, est_start, \
                est_finish, actual_start, actual_finish, status, notes, sort_order, parent_id) \
             VALUES ($1,$2,COALESCE($3,'TASK'),$4,$5,$6,$7,$8,COALESCE($9,'not_started'),$10, \
               COALESCE($11, (SELECT COALESCE(MAX(sort_order), -1) + 1 \
                              FROM call_forward WHERE job_id = $1)), $12) \
             RETURNING {COLS}"
        );
        let row: Row = sqlx::query_as(&sql)
            .bind(job.get())
            .bind(&input.title)
            .bind(input.item_type.map(ItemType::as_str))
            .bind(input.supplier_trade.as_deref())
            .bind(input.est_start)
            .bind(input.est_finish)
            .bind(input.actual_start)
            .bind(input.actual_finish)
            .bind(input.status.map(CfStatus::as_str))
            .bind(input.notes.as_deref())
            .bind(input.sort_order)
            .bind(parent)
            .fetch_one(&mut **tx)
            .await
            .map_err(map_sqlx)?;
        Ok(row.into())
    }
}

#[async_trait]
impl CallForwardRepository for PgCallForwardRepository {
    async fn list(
        &self,
        scope: TenantScope,
        visible_jobs: Option<&[JobId]>,
        filter: &CallForwardFilter,
    ) -> PortResult<Vec<CallForwardItem>> {
        let mut tx = self.begin(scope).await?;
        let sql = format!(
            "SELECT {COLS} FROM call_forward \
             WHERE ($1::int[] IS NULL OR job_id = ANY($1)) \
               AND ($2::int IS NULL OR job_id = $2) \
               AND ($3::text IS NULL OR status = $3) \
               AND ($4::int IS NULL OR parent_id = $4) \
             ORDER BY sort_order, id"
        );
        let rows: Vec<Row> = sqlx::query_as(&sql)
            .bind(Self::job_ids(visible_jobs))
            .bind(filter.job_id.map(JobId::get))
            .bind(filter.status.as_deref())
            .bind(filter.parent_id.map(CallForwardItemId::get))
            .fetch_all(&mut *tx)
            .await
            .map_err(map_sqlx)?;
        tx.commit().await.map_err(map_sqlx)?;
        Ok(rows.into_iter().map(Into::into).collect())
    }

    async fn find(
        &self,
        scope: TenantScope,
        visible_jobs: Option<&[JobId]>,
        id: CallForwardItemId,
    ) -> PortResult<Option<CallForwardItem>> {
        let mut tx = self.begin(scope).await?;
        let sql = format!(
            "SELECT {COLS} FROM call_forward \
             WHERE id = $1 AND ($2::int[] IS NULL OR job_id = ANY($2))"
        );
        let row: Option<Row> = sqlx::query_as(&sql)
            .bind(id.get())
            .bind(Self::job_ids(visible_jobs))
            .fetch_optional(&mut *tx)
            .await
            .map_err(map_sqlx)?;
        tx.commit().await.map_err(map_sqlx)?;
        Ok(row.map(Into::into))
    }

    async fn type_and_job(
        &self,
        scope: TenantScope,
        id: CallForwardItemId,
    ) -> PortResult<Option<(ItemType, JobId)>> {
        let mut tx = self.begin(scope).await?;
        let row: Option<(String, i32)> =
            sqlx::query_as("SELECT item_type, job_id FROM call_forward WHERE id = $1")
                .bind(id.get())
                .fetch_optional(&mut *tx)
                .await
                .map_err(map_sqlx)?;
        tx.commit().await.map_err(map_sqlx)?;
        Ok(row.map(|(t, j)| (ItemType::parse(&t).unwrap_or(ItemType::Task), JobId(j))))
    }

    async fn create(
        &self,
        scope: TenantScope,
        job: JobId,
        input: &CallForwardInput,
    ) -> PortResult<CallForwardItem> {
        let mut tx = self.begin(scope).await?;
        let out = Self::insert_in(&mut tx, job, input, input.parent_id).await?;
        tx.commit().await.map_err(map_sqlx)?;
        Ok(out)
    }

    async fn create_bulk(
        &self,
        scope: TenantScope,
        job: JobId,
        items: &[(CallForwardInput, Option<usize>)],
    ) -> PortResult<Vec<CallForwardItem>> {
        let mut tx = self.begin(scope).await?;
        let mut created: Vec<CallForwardItem> = Vec::with_capacity(items.len());

        // Inserted in order so a local parent index always refers to a row that
        // already exists. The service validates that indices only point
        // backwards before this runs.
        for (input, local_parent) in items {
            let parent = match local_parent {
                Some(idx) => {
                    let p = created.get(*idx).ok_or_else(|| {
                        PortError::Storage(format!("local parent index {idx} is out of range"))
                    })?;
                    Some(p.id.get())
                }
                None => input.parent_id,
            };
            created.push(Self::insert_in(&mut tx, job, input, parent).await?);
        }

        tx.commit().await.map_err(map_sqlx)?;
        Ok(created)
    }

    async fn update(
        &self,
        scope: TenantScope,
        id: CallForwardItemId,
        input: &CallForwardInput,
    ) -> PortResult<Option<CallForwardItem>> {
        let mut tx = self.begin(scope).await?;
        let sql = format!(
            "UPDATE call_forward SET title = $2, item_type = COALESCE($3, item_type), \
                supplier_trade = $4, est_start = $5, est_finish = $6, actual_start = $7, \
                actual_finish = $8, status = COALESCE($9, status), notes = $10, \
                sort_order = COALESCE($11, sort_order), parent_id = $12, updated_at = NOW() \
             WHERE id = $1 RETURNING {COLS}"
        );
        let row: Option<Row> = sqlx::query_as(&sql)
            .bind(id.get())
            .bind(&input.title)
            .bind(input.item_type.map(ItemType::as_str))
            .bind(input.supplier_trade.as_deref())
            .bind(input.est_start)
            .bind(input.est_finish)
            .bind(input.actual_start)
            .bind(input.actual_finish)
            .bind(input.status.map(CfStatus::as_str))
            .bind(input.notes.as_deref())
            .bind(input.sort_order)
            .bind(input.parent_id)
            .fetch_optional(&mut *tx)
            .await
            .map_err(map_sqlx)?;
        tx.commit().await.map_err(map_sqlx)?;
        Ok(row.map(Into::into))
    }

    async fn delete(&self, scope: TenantScope, id: CallForwardItemId) -> PortResult<bool> {
        let mut tx = self.begin(scope).await?;
        // `call_forward_parent_fk` (migration 0003) cascades, so deleting a
        // header removes its children rather than orphaning them -- the legacy
        // schema had no foreign key here at all.
        let r = sqlx::query("DELETE FROM call_forward WHERE id = $1")
            .bind(id.get())
            .execute(&mut *tx)
            .await
            .map_err(map_sqlx)?;
        tx.commit().await.map_err(map_sqlx)?;
        Ok(r.rows_affected() > 0)
    }

    async fn reorder(&self, scope: TenantScope, entries: &[ReorderEntry]) -> PortResult<u64> {
        if entries.is_empty() {
            return Ok(0);
        }
        let mut tx = self.begin(scope).await?;

        // One statement over unnested arrays rather than N round-trips: a
        // 200-item reorder is a single query, and the whole thing is atomic so
        // a partial reorder can never be observed.
        let ids: Vec<i32> = entries.iter().map(|e| e.id.get()).collect();
        let orders: Vec<i32> = entries.iter().map(|e| e.sort_order).collect();
        // `set_parent` distinguishes "leave parentage alone" from "detach to
        // root"; without it a plain sort-order reorder would null every parent.
        let set_parent: Vec<bool> = entries.iter().map(|e| e.parent_id.is_some()).collect();
        let parents: Vec<Option<i32>> = entries
            .iter()
            .map(|e| e.parent_id.flatten().map(|p| p.get()))
            .collect();

        let r = sqlx::query(
            "UPDATE call_forward c \
             SET sort_order = v.sort_order, \
                 parent_id = CASE WHEN v.set_parent THEN v.parent_id ELSE c.parent_id END, \
                 updated_at = NOW() \
             FROM (SELECT UNNEST($1::int[]) AS id, \
                          UNNEST($2::int[]) AS sort_order, \
                          UNNEST($3::int[]) AS parent_id, \
                          UNNEST($4::bool[]) AS set_parent) v \
             WHERE c.id = v.id",
        )
        .bind(&ids)
        .bind(&orders)
        .bind(&parents)
        .bind(&set_parent)
        .execute(&mut *tx)
        .await
        .map_err(map_sqlx)?;

        tx.commit().await.map_err(map_sqlx)?;
        Ok(r.rows_affected())
    }

    async fn upcoming(
        &self,
        scope: TenantScope,
        visible_jobs: Option<&[JobId]>,
        from: chrono::NaiveDate,
        days: i64,
    ) -> PortResult<Vec<CallForwardItem>> {
        let mut tx = self.begin(scope).await?;
        // Matches `idx_call_forward_open_finish`: only incomplete items can be
        // upcoming, so the partial index covers this exactly.
        let sql = format!(
            "SELECT {COLS} FROM call_forward \
             WHERE ($1::int[] IS NULL OR job_id = ANY($1)) \
               AND status <> 'completed' AND actual_finish IS NULL \
               AND est_finish IS NOT NULL \
               AND est_finish >= $2 AND est_finish <= $2 + $3::int \
             ORDER BY est_finish, sort_order, id"
        );
        let rows: Vec<Row> = sqlx::query_as(&sql)
            .bind(Self::job_ids(visible_jobs))
            .bind(from)
            .bind(i32::try_from(days).unwrap_or(30))
            .fetch_all(&mut *tx)
            .await
            .map_err(map_sqlx)?;
        tx.commit().await.map_err(map_sqlx)?;
        Ok(rows.into_iter().map(Into::into).collect())
    }
}
