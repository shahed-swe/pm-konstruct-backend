//! `NotificationRepository` over Postgres.
//!
//! `notifications`, `push_subscriptions` and `user_notification_prefs` all
//! reach their company through `users`, so RLS scopes them by join. Every
//! method also takes the user id explicitly: RLS confines a query to the
//! tenant, but one colleague must not be able to read or clear another's
//! notifications, and that is not a tenancy question.

use async_trait::async_trait;
use pmk_domain::ids::{NotificationId, UserId};
use pmk_domain::notifications::{Notification, NotificationPrefs, PushSubscription};
use pmk_domain::tenant::TenantScope;
use pmk_ports::repository::{NotificationInput, NotificationRepository};
use pmk_ports::PortResult;
use sqlx::{PgPool, Postgres, Row, Transaction};

use super::user::map_sqlx;
use crate::db::tenant::set_tenant;

const COLS: &str = "id, user_id, type, title, body, link, read_at, created_at, resolved_at";

fn row(r: &sqlx::postgres::PgRow) -> Notification {
    Notification {
        id: NotificationId(r.get("id")),
        user_id: UserId(r.get("user_id")),
        kind: r.get("type"),
        title: r.get("title"),
        body: r.get("body"),
        link: r.get("link"),
        read_at: r.get("read_at"),
        created_at: r.get("created_at"),
        resolved_at: r.get("resolved_at"),
    }
}

#[derive(Debug, Clone)]
pub struct PgNotificationRepository {
    pool: PgPool,
}

impl PgNotificationRepository {
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
impl NotificationRepository for PgNotificationRepository {
    async fn for_user(
        &self,
        scope: TenantScope,
        user: UserId,
        limit: i64,
    ) -> PortResult<Vec<Notification>> {
        let mut tx = self.begin(scope).await?;
        // Matches `idx_notifications_user_all`.
        let rows = sqlx::query(&format!(
            "SELECT {COLS} FROM notifications WHERE user_id = $1 \
             ORDER BY created_at DESC, id DESC LIMIT $2"
        ))
        .bind(user.get())
        .bind(limit)
        .fetch_all(&mut *tx)
        .await
        .map_err(map_sqlx)?;
        tx.commit().await.map_err(map_sqlx)?;
        Ok(rows.iter().map(row).collect())
    }

    async fn unread_count(&self, scope: TenantScope, user: UserId) -> PortResult<i64> {
        let mut tx = self.begin(scope).await?;
        // Matches the partial index `idx_notifications_user_unread`.
        let n: i64 = sqlx::query_scalar(
            "SELECT count(*) FROM notifications WHERE user_id = $1 AND read_at IS NULL",
        )
        .bind(user.get())
        .fetch_one(&mut *tx)
        .await
        .map_err(map_sqlx)?;
        tx.commit().await.map_err(map_sqlx)?;
        Ok(n)
    }

    async fn mark_read(
        &self,
        scope: TenantScope,
        user: UserId,
        id: NotificationId,
    ) -> PortResult<bool> {
        let mut tx = self.begin(scope).await?;
        // `user_id` in the WHERE clause, not checked afterwards: reading
        // someone else's notification to find out it exists is itself a leak.
        // Already-read rows still match, so the call is idempotent.
        let n = sqlx::query(
            "UPDATE notifications SET read_at = COALESCE(read_at, NOW()) \
             WHERE id = $1 AND user_id = $2",
        )
        .bind(id.get())
        .bind(user.get())
        .execute(&mut *tx)
        .await
        .map_err(map_sqlx)?
        .rows_affected();
        tx.commit().await.map_err(map_sqlx)?;
        Ok(n > 0)
    }

    async fn mark_all_read(&self, scope: TenantScope, user: UserId) -> PortResult<i64> {
        let mut tx = self.begin(scope).await?;
        let n = sqlx::query(
            "UPDATE notifications SET read_at = NOW() \
             WHERE user_id = $1 AND read_at IS NULL",
        )
        .bind(user.get())
        .execute(&mut *tx)
        .await
        .map_err(map_sqlx)?
        .rows_affected();
        tx.commit().await.map_err(map_sqlx)?;
        Ok(i64::try_from(n).unwrap_or(i64::MAX))
    }

    async fn create_many(
        &self,
        scope: TenantScope,
        inputs: &[NotificationInput],
    ) -> PortResult<Vec<Notification>> {
        if inputs.is_empty() {
            return Ok(Vec::new());
        }
        let users: Vec<i32> = inputs.iter().map(|i| i.user_id.get()).collect();
        let kinds: Vec<String> = inputs.iter().map(|i| i.kind.clone()).collect();
        let titles: Vec<String> = inputs.iter().map(|i| i.title.clone()).collect();
        let bodies: Vec<Option<String>> = inputs.iter().map(|i| i.body.clone()).collect();
        let links: Vec<Option<String>> = inputs.iter().map(|i| i.link.clone()).collect();

        let mut tx = self.begin(scope).await?;
        // One statement via unnest: a diary note can notify every manager on a
        // job, and a statement each would make that N round-trips.
        let rows = sqlx::query(&format!(
            "INSERT INTO notifications (user_id, type, title, body, link) \
             SELECT * FROM unnest($1::int[], $2::text[], $3::text[], $4::text[], $5::text[]) \
             RETURNING {COLS}"
        ))
        .bind(&users)
        .bind(&kinds)
        .bind(&titles)
        .bind(&bodies)
        .bind(&links)
        .fetch_all(&mut *tx)
        .await
        .map_err(map_sqlx)?;
        tx.commit().await.map_err(map_sqlx)?;
        Ok(rows.iter().map(row).collect())
    }

    async fn prefs(&self, scope: TenantScope, user: UserId) -> PortResult<NotificationPrefs> {
        let mut tx = self.begin(scope).await?;
        let r = sqlx::query(
            "SELECT notify_action_notes, notify_call_forward \
             FROM user_notification_prefs WHERE user_id = $1",
        )
        .bind(user.get())
        .fetch_optional(&mut *tx)
        .await
        .map_err(map_sqlx)?;
        tx.commit().await.map_err(map_sqlx)?;

        // No row means the user has never opened the settings page, so they
        // get the same defaults the column would have given them.
        Ok(
            r.map_or_else(NotificationPrefs::default, |r| NotificationPrefs {
                notify_action_notes: r.get("notify_action_notes"),
                notify_call_forward: r.get("notify_call_forward"),
            }),
        )
    }

    async fn set_prefs(
        &self,
        scope: TenantScope,
        user: UserId,
        prefs: NotificationPrefs,
    ) -> PortResult<NotificationPrefs> {
        let mut tx = self.begin(scope).await?;
        let r = sqlx::query(
            "INSERT INTO user_notification_prefs \
               (user_id, notify_action_notes, notify_call_forward) \
             VALUES ($1,$2,$3) \
             ON CONFLICT (user_id) DO UPDATE \
               SET notify_action_notes = EXCLUDED.notify_action_notes, \
                   notify_call_forward = EXCLUDED.notify_call_forward, \
                   updated_at = NOW() \
             RETURNING notify_action_notes, notify_call_forward",
        )
        .bind(user.get())
        .bind(prefs.notify_action_notes)
        .bind(prefs.notify_call_forward)
        .fetch_one(&mut *tx)
        .await
        .map_err(map_sqlx)?;
        tx.commit().await.map_err(map_sqlx)?;

        Ok(NotificationPrefs {
            notify_action_notes: r.get("notify_action_notes"),
            notify_call_forward: r.get("notify_call_forward"),
        })
    }

    async fn save_subscription(
        &self,
        scope: TenantScope,
        user: UserId,
        sub: &PushSubscription,
    ) -> PortResult<()> {
        let mut tx = self.begin(scope).await?;
        // Keyed on the endpoint, which is unique across users: a shared
        // browser profile moving between accounts re-points the subscription
        // rather than leaving the previous user subscribed to it.
        sqlx::query(
            "INSERT INTO push_subscriptions (user_id, endpoint, p256dh, auth) \
             VALUES ($1,$2,$3,$4) \
             ON CONFLICT (endpoint) DO UPDATE \
               SET user_id = EXCLUDED.user_id, p256dh = EXCLUDED.p256dh, \
                   auth = EXCLUDED.auth, updated_at = NOW()",
        )
        .bind(user.get())
        .bind(&sub.endpoint)
        .bind(&sub.p256dh)
        .bind(&sub.auth)
        .execute(&mut *tx)
        .await
        .map_err(map_sqlx)?;
        tx.commit().await.map_err(map_sqlx)?;
        Ok(())
    }

    async fn remove_subscription(
        &self,
        scope: TenantScope,
        user: UserId,
        endpoint: &str,
    ) -> PortResult<bool> {
        let mut tx = self.begin(scope).await?;
        let n = sqlx::query("DELETE FROM push_subscriptions WHERE user_id = $1 AND endpoint = $2")
            .bind(user.get())
            .bind(endpoint)
            .execute(&mut *tx)
            .await
            .map_err(map_sqlx)?
            .rows_affected();
        tx.commit().await.map_err(map_sqlx)?;
        Ok(n > 0)
    }

    async fn subscriptions_for(
        &self,
        scope: TenantScope,
        users: &[UserId],
    ) -> PortResult<Vec<(UserId, PushSubscription)>> {
        if users.is_empty() {
            return Ok(Vec::new());
        }
        let ids: Vec<i32> = users.iter().map(|u| u.get()).collect();
        let mut tx = self.begin(scope).await?;
        let rows = sqlx::query(
            "SELECT user_id, endpoint, p256dh, auth FROM push_subscriptions \
             WHERE user_id = ANY($1)",
        )
        .bind(&ids)
        .fetch_all(&mut *tx)
        .await
        .map_err(map_sqlx)?;
        tx.commit().await.map_err(map_sqlx)?;

        Ok(rows
            .into_iter()
            .map(|r| {
                (
                    UserId(r.get("user_id")),
                    PushSubscription {
                        endpoint: r.get("endpoint"),
                        p256dh: r.get("p256dh"),
                        auth: r.get("auth"),
                    },
                )
            })
            .collect())
    }
}
