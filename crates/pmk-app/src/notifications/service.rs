use std::sync::Arc;

use pmk_domain::ids::{NotificationId, UserId};
use pmk_domain::notifications::{Notification, NotificationPrefs, PushSubscription};
use pmk_domain::DomainError;
use pmk_ports::repository::{NotificationInput, NotificationRepository};

use crate::identity::SessionUser;
use crate::{AppError, AppResult};

/// The most notifications one request returns.
///
/// The bell menu shows a short list and the rest are reached by scrolling, so
/// an unbounded query would fetch years of history to render twenty rows.
const FEED_LIMIT: i64 = 100;

/// What the bell shows: the list and the badge.
#[derive(Debug, Clone)]
pub struct NotificationFeed {
    pub notifications: Vec<Notification>,
    pub unread: i64,
}

pub struct NotificationService {
    notifications: Arc<dyn NotificationRepository>,
    /// The VAPID public key the browser needs to subscribe. Empty when push is
    /// not configured, which is not an error: the app works without it.
    vapid_public_key: String,
}

impl std::fmt::Debug for NotificationService {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        f.debug_struct("NotificationService")
            .finish_non_exhaustive()
    }
}

impl NotificationService {
    #[must_use]
    pub fn new(notifications: Arc<dyn NotificationRepository>, vapid_public_key: String) -> Self {
        Self {
            notifications,
            vapid_public_key,
        }
    }

    /// The browser needs this to create a subscription. It is public by
    /// design -- the private half never leaves the server.
    #[must_use]
    pub fn vapid_public_key(&self) -> &str {
        &self.vapid_public_key
    }

    pub async fn feed(&self, s: &SessionUser) -> AppResult<NotificationFeed> {
        let scope = s.principal.scope();
        // Two queries rather than counting in Rust: the unread count has its
        // own partial index, and the list is capped, so counting from the
        // capped list would under-report the badge.
        let notifications = self
            .notifications
            .for_user(scope, s.user.id, FEED_LIMIT)
            .await?;
        let unread = self.notifications.unread_count(scope, s.user.id).await?;
        Ok(NotificationFeed {
            notifications,
            unread,
        })
    }

    pub async fn mark_read(&self, s: &SessionUser, id: NotificationId) -> AppResult<()> {
        if self
            .notifications
            .mark_read(s.principal.scope(), s.user.id, id)
            .await?
        {
            Ok(())
        } else {
            // 404 rather than 403: someone else's notification should not be
            // distinguishable from one that does not exist.
            Err(AppError::Domain(DomainError::not_found("Notification")))
        }
    }

    pub async fn mark_all_read(&self, s: &SessionUser) -> AppResult<i64> {
        Ok(self
            .notifications
            .mark_all_read(s.principal.scope(), s.user.id)
            .await?)
    }

    pub async fn prefs(&self, s: &SessionUser) -> AppResult<NotificationPrefs> {
        Ok(self
            .notifications
            .prefs(s.principal.scope(), s.user.id)
            .await?)
    }

    pub async fn set_prefs(
        &self,
        s: &SessionUser,
        prefs: NotificationPrefs,
    ) -> AppResult<NotificationPrefs> {
        Ok(self
            .notifications
            .set_prefs(s.principal.scope(), s.user.id, prefs)
            .await?)
    }

    /// Records a browser's push subscription.
    ///
    /// The endpoint is validated before it is stored, not only before it is
    /// used: a stored endpoint is a URL this server will later call, so the
    /// allowlist has to hold at the point of writing.
    pub async fn subscribe(&self, s: &SessionUser, sub: &PushSubscription) -> AppResult<()> {
        sub.validate().map_err(AppError::Domain)?;
        Ok(self
            .notifications
            .save_subscription(s.principal.scope(), s.user.id, sub)
            .await?)
    }

    pub async fn unsubscribe(&self, s: &SessionUser, endpoint: &str) -> AppResult<()> {
        if endpoint.trim().is_empty() {
            return Err(AppError::Domain(DomainError::invalid(
                "endpoint",
                "endpoint required",
            )));
        }
        // Not found is not an error: the browser is telling us it has dropped
        // the subscription, and the desired state is reached either way.
        self.notifications
            .remove_subscription(s.principal.scope(), s.user.id, endpoint)
            .await?;
        Ok(())
    }

    /// Raises notifications, honouring each recipient's preferences.
    ///
    /// Used by the diary and call-forward flows. Filtering here rather than at
    /// each call site means a new notification source cannot forget to check.
    pub async fn notify(
        &self,
        s: &SessionUser,
        recipients: &[UserId],
        kind: &str,
        title: &str,
        body: Option<&str>,
        link: Option<&str>,
    ) -> AppResult<Vec<Notification>> {
        let scope = s.principal.scope();
        let mut inputs = Vec::with_capacity(recipients.len());
        for &user in recipients {
            // Nobody is told about their own action.
            if user == s.user.id {
                continue;
            }
            let prefs = self.notifications.prefs(scope, user).await?;
            if !wants(kind, prefs) {
                continue;
            }
            inputs.push(NotificationInput {
                user_id: user,
                kind: kind.to_string(),
                title: title.to_string(),
                body: body.map(ToString::to_string),
                link: link.map(ToString::to_string),
            });
        }
        Ok(self.notifications.create_many(scope, &inputs).await?)
    }
}

/// Does this user want to hear about this kind of event?
///
/// Only the two categories the settings page offers are filtered. Anything
/// else -- an ETO awaiting approval, a billing warning -- is delivered
/// regardless: those are not preferences, they are things the recipient has to
/// act on.
fn wants(kind: &str, prefs: NotificationPrefs) -> bool {
    match kind {
        k if k.starts_with("action_note") => prefs.notify_action_notes,
        k if k.starts_with("call_forward") => prefs.notify_call_forward,
        _ => true,
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn the_two_settable_categories_are_honoured() {
        let all_off = NotificationPrefs {
            notify_action_notes: false,
            notify_call_forward: false,
        };
        assert!(!wants("action_note_raised", all_off));
        assert!(!wants("call_forward_due", all_off));

        let all_on = NotificationPrefs {
            notify_action_notes: true,
            notify_call_forward: true,
        };
        assert!(wants("action_note_raised", all_on));
        assert!(wants("call_forward_due", all_on));
    }

    #[test]
    fn everything_else_is_delivered_whatever_the_preferences() {
        // An ETO awaiting a manager's approval, or a billing warning, is not
        // a preference -- it is something the recipient has to act on.
        let all_off = NotificationPrefs {
            notify_action_notes: false,
            notify_call_forward: false,
        };
        assert!(wants("eto_approval_required", all_off));
        assert!(wants("billing_lapsed", all_off));
    }

    #[test]
    fn the_defaults_let_action_notes_through_and_hold_call_forward_back() {
        let d = NotificationPrefs::default();
        assert!(wants("action_note_raised", d));
        assert!(!wants("call_forward_due", d));
    }
}
