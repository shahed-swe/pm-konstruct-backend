//! Notifications and web-push subscriptions.

use crate::error::{DomainError, DomainResult};
use crate::ids::{NotificationId, UserId};

#[derive(Debug, Clone)]
pub struct Notification {
    pub id: NotificationId,
    pub user_id: UserId,
    pub kind: String,
    pub title: String,
    pub body: Option<String>,
    /// Where clicking it takes the user, as an in-app path.
    pub link: Option<String>,
    pub read_at: Option<chrono::DateTime<chrono::Utc>>,
    pub created_at: chrono::DateTime<chrono::Utc>,
    pub resolved_at: Option<chrono::DateTime<chrono::Utc>>,
}

/// What a user wants to be told about.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct NotificationPrefs {
    pub notify_action_notes: bool,
    pub notify_call_forward: bool,
}

impl Default for NotificationPrefs {
    /// Matches the column defaults: action notes on, call-forward off.
    ///
    /// A user who has never opened the settings page has no row, and must see
    /// the same thing the database would have given them.
    fn default() -> Self {
        Self {
            notify_action_notes: true,
            notify_call_forward: false,
        }
    }
}

/// Hostname suffixes belonging to real browser push services.
///
/// An endpoint is a URL this server will later POST to, chosen by whoever
/// subscribed. Without an allowlist an authenticated user could point it at
/// anything reachable from the server -- an internal metadata service, a
/// database admin port -- and have the push worker fetch it for them. That is
/// server-side request forgery, and the allowlist is what stops it.
pub const ALLOWED_PUSH_HOSTS: [&str; 7] = [
    "fcm.googleapis.com",                // Chrome and Chromium
    "updates.push.services.mozilla.com", // Firefox
    "push.services.mozilla.com",         // Firefox, older builds
    "push.apple.com",                    // Safari
    "web.push.apple.com",                // Apple web push
    "notify.windows.com",                // Edge
    "wns.windows.com",                   // Windows Notification Service
];

/// A browser's push subscription.
#[derive(Debug, Clone)]
pub struct PushSubscription {
    pub endpoint: String,
    pub p256dh: String,
    pub auth: String,
}

impl PushSubscription {
    pub fn validate(&self) -> DomainResult<()> {
        validate_push_endpoint(&self.endpoint)?;
        if self.p256dh.trim().is_empty() || self.auth.trim().is_empty() {
            return Err(DomainError::invalid("keys", "Invalid subscription payload"));
        }
        Ok(())
    }
}

/// Checks a push endpoint is one this server is willing to call.
///
/// Parsed by hand rather than with a URL crate: the checks are few, and the
/// rules below are exactly the legacy ones, which a general parser's
/// normalisation could quietly change.
pub fn validate_push_endpoint(endpoint: &str) -> DomainResult<()> {
    let bad = |reason: &'static str| DomainError::invalid("endpoint", reason);

    let endpoint = endpoint.trim();
    if endpoint.is_empty() {
        return Err(bad("Invalid subscription payload"));
    }

    // HTTPS only. A plain-http endpoint would send the push payload, which
    // identifies the user, in the clear.
    let Some(rest) = endpoint.strip_prefix("https://") else {
        return Err(bad("Push endpoint must use HTTPS"));
    };

    let authority = rest.split(['/', '?', '#']).next().unwrap_or("");
    if authority.is_empty() {
        return Err(bad("Push endpoint must be a valid URL"));
    }

    // Credentials in the URL would be sent by the push worker on every call.
    if authority.contains('@') {
        return Err(bad("Push endpoint must not include credentials"));
    }

    // A non-default port means the endpoint is not a public push service,
    // whatever its hostname claims -- and is how an allowlisted name could be
    // pointed at an internal service.
    let (host, port) = match authority.rsplit_once(':') {
        Some((h, p)) => (h, Some(p)),
        None => (authority, None),
    };
    if let Some(port) = port {
        if port != "443" {
            return Err(bad("Push endpoint must use the default HTTPS port"));
        }
    }

    let host = host.to_ascii_lowercase();
    if host.is_empty() {
        return Err(bad("Push endpoint must be a valid URL"));
    }

    // The leading dot matters. Matching on a bare `ends_with(suffix)` would
    // also accept `notfcm.googleapis.com`, which is a hostname an attacker can
    // register.
    let allowed = ALLOWED_PUSH_HOSTS
        .iter()
        .any(|s| host == *s || host.ends_with(&format!(".{s}")));
    if !allowed {
        return Err(bad("Push endpoint host is not a recognised push service"));
    }

    Ok(())
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn real_push_endpoints_are_accepted() {
        for url in [
            "https://fcm.googleapis.com/fcm/send/abc123",
            "https://updates.push.services.mozilla.com/wpush/v2/xyz",
            "https://web.push.apple.com/Q123",
            "https://notify.windows.com/w/?token=abc",
            "https://android.googleapis.com.fcm.googleapis.com/x",
        ] {
            assert!(
                validate_push_endpoint(url).is_ok(),
                "{url} should be accepted"
            );
        }
    }

    #[test]
    fn a_subdomain_of_an_allowed_host_is_accepted() {
        assert!(validate_push_endpoint("https://a.b.fcm.googleapis.com/x").is_ok());
    }

    #[test]
    fn a_hostname_merely_ending_in_an_allowed_name_is_rejected() {
        // The dot matters: without it, `notfcm.googleapis.com` would pass.
        assert!(validate_push_endpoint("https://notfcm.googleapis.com/x").is_err());
        assert!(validate_push_endpoint("https://evil-push.apple.com.attacker.test/x").is_err());
    }

    #[test]
    fn plain_http_is_rejected() {
        assert!(validate_push_endpoint("http://fcm.googleapis.com/x").is_err());
    }

    #[test]
    fn internal_addresses_are_rejected_even_over_https() {
        // The whole point of the allowlist: an authenticated user must not be
        // able to steer the push worker at the host's own network.
        for url in [
            "https://169.254.169.254/latest/meta-data/",
            "https://localhost/admin",
            "https://127.0.0.1/",
            "https://10.0.0.5/internal",
            "https://db.internal/",
        ] {
            assert!(
                validate_push_endpoint(url).is_err(),
                "{url} should be rejected"
            );
        }
    }

    #[test]
    fn credentials_in_the_url_are_rejected() {
        assert!(validate_push_endpoint("https://user:pass@fcm.googleapis.com/x").is_err());
        assert!(validate_push_endpoint("https://user@fcm.googleapis.com/x").is_err());
    }

    #[test]
    fn a_non_default_port_is_rejected() {
        // An allowlisted hostname on port 8080 is not the push service.
        assert!(validate_push_endpoint("https://fcm.googleapis.com:8080/x").is_err());
        assert!(validate_push_endpoint("https://fcm.googleapis.com:443/x").is_ok());
    }

    #[test]
    fn the_host_is_matched_case_insensitively() {
        assert!(validate_push_endpoint("https://FCM.GoogleAPIs.COM/x").is_ok());
    }

    #[test]
    fn an_empty_or_malformed_endpoint_is_rejected() {
        for url in ["", "   ", "https://", "https:///path", "not a url"] {
            assert!(
                validate_push_endpoint(url).is_err(),
                "{url:?} should be rejected"
            );
        }
    }

    #[test]
    fn a_subscription_needs_both_keys() {
        let ok = PushSubscription {
            endpoint: "https://fcm.googleapis.com/x".into(),
            p256dh: "key".into(),
            auth: "auth".into(),
        };
        assert!(ok.validate().is_ok());

        let no_p256dh = PushSubscription {
            p256dh: "  ".into(),
            ..ok.clone()
        };
        assert!(no_p256dh.validate().is_err());

        let no_auth = PushSubscription {
            auth: String::new(),
            ..ok
        };
        assert!(no_auth.validate().is_err());
    }

    #[test]
    fn the_default_preferences_match_the_column_defaults() {
        // A user who has never opened settings has no row, and must see what
        // the database would have given them.
        let d = NotificationPrefs::default();
        assert!(d.notify_action_notes);
        assert!(!d.notify_call_forward);
    }
}
