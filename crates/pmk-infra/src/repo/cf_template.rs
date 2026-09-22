//! `CallForwardTemplateRepository` over Postgres.
//!
//! The items live in a `jsonb` column rather than their own table. That is the
//! legacy shape and it is the right one here: a template is read and written
//! whole, never queried into, and the local ids inside it are meaningful only
//! relative to the rest of the document.

use async_trait::async_trait;
use pmk_domain::call_forward::templates::{ProgrammeRow, Template, TemplateItem};
use pmk_domain::ids::{CallForwardTemplateId, JobId};
use pmk_domain::tenant::TenantScope;
use pmk_ports::repository::CallForwardTemplateRepository;
use pmk_ports::{PortError, PortResult};
use sqlx::{PgPool, Postgres, Row, Transaction};

use super::user::map_sqlx;
use crate::db::tenant::set_tenant;

const COLS: &str = "id, name, description, items, created_at, updated_at";

fn row(r: &sqlx::postgres::PgRow) -> Template {
    Template {
        id: CallForwardTemplateId(r.get("id")),
        name: r.get("name"),
        description: r.get("description"),
        // A template whose document cannot be read comes back empty rather
        // than failing the whole list: one corrupt row must not hide the rest.
        items: serde_json::from_value(r.get("items")).unwrap_or_default(),
        created_at: r.get("created_at"),
        updated_at: r.get("updated_at"),
    }
}

#[derive(Debug, Clone)]
pub struct PgCallForwardTemplateRepository {
    pool: PgPool,
}

impl PgCallForwardTemplateRepository {
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
impl CallForwardTemplateRepository for PgCallForwardTemplateRepository {
    async fn list(&self, scope: TenantScope) -> PortResult<Vec<Template>> {
        let mut tx = self.begin(scope).await?;
        let rows = sqlx::query(&format!(
            "SELECT {COLS} FROM call_forward_templates ORDER BY name, id"
        ))
        .fetch_all(&mut *tx)
        .await
        .map_err(map_sqlx)?;
        tx.commit().await.map_err(map_sqlx)?;
        Ok(rows.iter().map(row).collect())
    }

    async fn find(
        &self,
        scope: TenantScope,
        id: CallForwardTemplateId,
    ) -> PortResult<Option<Template>> {
        let mut tx = self.begin(scope).await?;
        let r = sqlx::query(&format!(
            "SELECT {COLS} FROM call_forward_templates WHERE id = $1"
        ))
        .bind(id.get())
        .fetch_optional(&mut *tx)
        .await
        .map_err(map_sqlx)?;
        tx.commit().await.map_err(map_sqlx)?;
        Ok(r.as_ref().map(row))
    }

    async fn job_programme(&self, scope: TenantScope, job: JobId) -> PortResult<Vec<ProgrammeRow>> {
        let mut tx = self.begin(scope).await?;
        let rows = sqlx::query(
            "SELECT id, parent_id, title, item_type, supplier_trade FROM call_forward \
             WHERE job_id = $1 ORDER BY sort_order, id",
        )
        .bind(job.get())
        .fetch_all(&mut *tx)
        .await
        .map_err(map_sqlx)?;
        tx.commit().await.map_err(map_sqlx)?;

        Ok(rows
            .into_iter()
            .map(|r| ProgrammeRow {
                id: r.get("id"),
                parent_id: r.get("parent_id"),
                title: r.get("title"),
                item_type: r.get("item_type"),
                supplier_trade: r.get("supplier_trade"),
            })
            .collect())
    }

    async fn create(
        &self,
        scope: TenantScope,
        name: &str,
        description: Option<&str>,
        items: &[TemplateItem],
    ) -> PortResult<Template> {
        let document = serde_json::to_value(items)
            .map_err(|e| PortError::Storage(format!("could not encode a template: {e}")))?;
        let mut tx = self.begin(scope).await?;
        let r = sqlx::query(&format!(
            "INSERT INTO call_forward_templates (company_id, name, description, items) \
             VALUES ($1,$2,$3,$4) RETURNING {COLS}"
        ))
        .bind(scope.company_id().get())
        .bind(name.trim())
        .bind(description)
        .bind(&document)
        .fetch_one(&mut *tx)
        .await
        .map_err(map_sqlx)?;
        tx.commit().await.map_err(map_sqlx)?;
        Ok(row(&r))
    }

    async fn rename(
        &self,
        scope: TenantScope,
        id: CallForwardTemplateId,
        name: &str,
        description: Option<&str>,
    ) -> PortResult<Option<Template>> {
        let mut tx = self.begin(scope).await?;
        let r = sqlx::query(&format!(
            "UPDATE call_forward_templates SET name = $2, description = $3, updated_at = NOW() \
             WHERE id = $1 RETURNING {COLS}"
        ))
        .bind(id.get())
        .bind(name.trim())
        .bind(description)
        .fetch_optional(&mut *tx)
        .await
        .map_err(map_sqlx)?;
        tx.commit().await.map_err(map_sqlx)?;
        Ok(r.as_ref().map(row))
    }

    async fn delete(&self, scope: TenantScope, id: CallForwardTemplateId) -> PortResult<bool> {
        let mut tx = self.begin(scope).await?;
        let n = sqlx::query("DELETE FROM call_forward_templates WHERE id = $1")
            .bind(id.get())
            .execute(&mut *tx)
            .await
            .map_err(map_sqlx)?
            .rows_affected();
        tx.commit().await.map_err(map_sqlx)?;
        Ok(n > 0)
    }

    async fn apply(
        &self,
        scope: TenantScope,
        id: CallForwardTemplateId,
        job: JobId,
        replace: bool,
    ) -> PortResult<usize> {
        let mut tx = self.begin(scope).await?;

        let template = sqlx::query("SELECT items FROM call_forward_templates WHERE id = $1")
            .bind(id.get())
            .fetch_optional(&mut *tx)
            .await
            .map_err(map_sqlx)?;
        let Some(template) = template else {
            return Err(PortError::NotFound);
        };
        let items: Vec<TemplateItem> =
            serde_json::from_value(template.get("items")).unwrap_or_default();
        if items.is_empty() {
            tx.commit().await.map_err(map_sqlx)?;
            return Ok(0);
        }

        // The whole application is one transaction: a half-written programme
        // with dangling parents is worse than none.
        if replace {
            sqlx::query("DELETE FROM call_forward WHERE job_id = $1")
                .bind(job.get())
                .execute(&mut *tx)
                .await
                .map_err(map_sqlx)?;
        }

        // Appended after whatever is already there, so applying a second
        // template does not interleave with the first.
        let base: i32 = sqlx::query_scalar(
            "SELECT COALESCE(MAX(sort_order), -1) + 1 FROM call_forward WHERE job_id = $1",
        )
        .bind(job.get())
        .fetch_one(&mut *tx)
        .await
        .map_err(map_sqlx)?;

        // Inserted one at a time because each child needs the id its parent
        // produced. The domain guarantees parents come first, so one pass is
        // enough.
        let mut local_to_db: std::collections::HashMap<i32, i32> = std::collections::HashMap::new();
        for item in &items {
            let parent = item
                .local_parent_id
                .and_then(|p| local_to_db.get(&p).copied());
            let new_id: i32 = sqlx::query_scalar(
                "INSERT INTO call_forward \
                   (job_id, title, item_type, supplier_trade, status, sort_order, parent_id) \
                 VALUES ($1,$2,$3,$4,'not_started',$5,$6) RETURNING id",
            )
            .bind(job.get())
            .bind(&item.title)
            .bind(&item.item_type)
            .bind(item.supplier_trade.as_deref())
            .bind(base + item.sort_order)
            .bind(parent)
            .fetch_one(&mut *tx)
            .await
            .map_err(map_sqlx)?;
            local_to_db.insert(item.local_id, new_id);
        }

        tx.commit().await.map_err(map_sqlx)?;
        Ok(items.len())
    }
}
