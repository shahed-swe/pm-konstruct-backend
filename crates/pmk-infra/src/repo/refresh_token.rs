//! `RefreshTokenRepository` over Postgres.

use async_trait::async_trait;
use pmk_domain::ids::UserId;
use pmk_ports::repository::{RefreshTokenRecord, RefreshTokenRepository};
use pmk_ports::PortResult;
use sqlx::PgPool;

use super::user::map_sqlx;

#[derive(Debug, Clone)]
pub struct PgRefreshTokenRepository {
    pool: PgPool,
}

impl PgRefreshTokenRepository {
    #[must_use]
    pub const fn new(pool: PgPool) -> Self {
        Self { pool }
    }
}

#[derive(Debug, sqlx::FromRow)]
struct Row {
    id: i64,
    user_id: i32,
    family_id: uuid::Uuid,
    expires_at: chrono::DateTime<chrono::Utc>,
    revoked_at: Option<chrono::DateTime<chrono::Utc>>,
    used_at: Option<chrono::DateTime<chrono::Utc>>,
}

impl From<Row> for RefreshTokenRecord {
    fn from(r: Row) -> Self {
        Self {
            id: r.id,
            user_id: UserId(r.user_id),
            family_id: r.family_id,
            expires_at: r.expires_at,
            revoked_at: r.revoked_at,
            used_at: r.used_at,
        }
    }
}

#[async_trait]
impl RefreshTokenRepository for PgRefreshTokenRepository {
    async fn insert(
        &self,
        user_id: UserId,
        token_hash: &str,
        family_id: uuid::Uuid,
        expires_at: chrono::DateTime<chrono::Utc>,
    ) -> PortResult<()> {
        sqlx::query(
            "INSERT INTO refresh_tokens (user_id, token_hash, family_id, expires_at) \
             VALUES ($1, $2, $3, $4)",
        )
        .bind(user_id.get())
        .bind(token_hash)
        .bind(family_id)
        .bind(expires_at)
        .execute(&self.pool)
        .await
        .map_err(map_sqlx)?;
        Ok(())
    }

    async fn find_by_hash(&self, token_hash: &str) -> PortResult<Option<RefreshTokenRecord>> {
        let row: Option<Row> = sqlx::query_as(
            "SELECT id, user_id, family_id, expires_at, revoked_at, used_at \
             FROM refresh_tokens WHERE token_hash = $1",
        )
        .bind(token_hash)
        .fetch_optional(&self.pool)
        .await
        .map_err(map_sqlx)?;
        Ok(row.map(Into::into))
    }

    async fn mark_used(&self, id: i64) -> PortResult<()> {
        sqlx::query("UPDATE refresh_tokens SET used_at = NOW() WHERE id = $1 AND used_at IS NULL")
            .bind(id)
            .execute(&self.pool)
            .await
            .map_err(map_sqlx)?;
        Ok(())
    }

    async fn revoke_family(&self, family_id: uuid::Uuid) -> PortResult<u64> {
        let r = sqlx::query(
            "UPDATE refresh_tokens SET revoked_at = NOW() \
             WHERE family_id = $1 AND revoked_at IS NULL",
        )
        .bind(family_id)
        .execute(&self.pool)
        .await
        .map_err(map_sqlx)?;
        Ok(r.rows_affected())
    }

    async fn revoke_all_for_user(&self, user_id: UserId) -> PortResult<u64> {
        let r = sqlx::query(
            "UPDATE refresh_tokens SET revoked_at = NOW() \
             WHERE user_id = $1 AND revoked_at IS NULL",
        )
        .bind(user_id.get())
        .execute(&self.pool)
        .await
        .map_err(map_sqlx)?;
        Ok(r.rows_affected())
    }

    async fn delete_expired(&self) -> PortResult<u64> {
        // Keeps revoked rows briefly so reuse detection still has something to
        // match against just after a family is killed.
        let r = sqlx::query(
            "DELETE FROM refresh_tokens \
             WHERE expires_at < NOW() - INTERVAL '7 days'",
        )
        .execute(&self.pool)
        .await
        .map_err(map_sqlx)?;
        Ok(r.rows_affected())
    }
}
