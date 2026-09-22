//! Notification shapes.

use pmk_app::notifications::NotificationFeed;
use pmk_domain::notifications::{Notification, NotificationPrefs, PushSubscription};
use serde::{Deserialize, Serialize};

#[derive(Debug, Serialize)]
#[serde(rename_all = "camelCase")]
pub struct NotificationDto {
    pub id: i32,
    pub user_id: i32,
    #[serde(rename = "type")]
    pub kind: String,
    pub title: String,
    pub body: Option<String>,
    pub link: Option<String>,
    pub read_at: Option<chrono::DateTime<chrono::Utc>>,
    pub created_at: chrono::DateTime<chrono::Utc>,
    pub resolved_at: Option<chrono::DateTime<chrono::Utc>>,
}

impl From<Notification> for NotificationDto {
    fn from(n: Notification) -> Self {
        Self {
            id: n.id.get(),
            user_id: n.user_id.get(),
            kind: n.kind,
            title: n.title,
            body: n.body,
            link: n.link,
            read_at: n.read_at,
            created_at: n.created_at,
            resolved_at: n.resolved_at,
        }
    }
}

/// The bell: a list plus the badge count.
#[derive(Debug, Serialize)]
#[serde(rename_all = "camelCase")]
pub struct NotificationFeedDto {
    pub notifications: Vec<NotificationDto>,
    pub unread: i64,
}

impl From<NotificationFeed> for NotificationFeedDto {
    fn from(f: NotificationFeed) -> Self {
        Self {
            notifications: f.notifications.into_iter().map(Into::into).collect(),
            unread: f.unread,
        }
    }
}

#[derive(Debug, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct NotificationPrefsDto {
    pub notify_action_notes: bool,
    pub notify_call_forward: bool,
}

impl From<NotificationPrefs> for NotificationPrefsDto {
    fn from(p: NotificationPrefs) -> Self {
        Self {
            notify_action_notes: p.notify_action_notes,
            notify_call_forward: p.notify_call_forward,
        }
    }
}

/// Absent fields keep their current value, so a client can toggle one switch
/// without having to know the other's state.
#[derive(Debug, Deserialize, Default)]
#[serde(rename_all = "camelCase")]
pub struct UpdatePrefsRequest {
    pub notify_action_notes: Option<bool>,
    pub notify_call_forward: Option<bool>,
}

impl UpdatePrefsRequest {
    #[must_use]
    pub fn apply(&self, current: NotificationPrefs) -> NotificationPrefs {
        NotificationPrefs {
            notify_action_notes: self
                .notify_action_notes
                .unwrap_or(current.notify_action_notes),
            notify_call_forward: self
                .notify_call_forward
                .unwrap_or(current.notify_call_forward),
        }
    }
}

#[derive(Debug, Deserialize)]
pub struct PushKeys {
    pub p256dh: String,
    pub auth: String,
}

/// The shape `PushSubscription.toJSON()` produces in the browser.
#[derive(Debug, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct SubscribeRequest {
    pub endpoint: String,
    pub keys: PushKeys,
}

impl From<SubscribeRequest> for PushSubscription {
    fn from(r: SubscribeRequest) -> Self {
        Self {
            endpoint: r.endpoint,
            p256dh: r.keys.p256dh,
            auth: r.keys.auth,
        }
    }
}

#[derive(Debug, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct UnsubscribeRequest {
    pub endpoint: String,
}

#[derive(Debug, Serialize)]
#[serde(rename_all = "camelCase")]
pub struct VapidKeyDto {
    /// Empty when push is not configured. The app works without it, so this
    /// is not an error state -- the frontend simply does not offer push.
    pub public_key: String,
}

/// `{ "ok": true }`, which several of these routes return verbatim.
#[derive(Debug, Serialize)]
pub struct OkDto {
    pub ok: bool,
}

impl OkDto {
    #[must_use]
    pub const fn yes() -> Self {
        Self { ok: true }
    }
}
