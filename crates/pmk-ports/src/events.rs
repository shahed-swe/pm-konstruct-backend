//! Live event fan-out.
//!
//! The legacy broker kept its subscriber list in one Node process, so with two
//! API instances a client connected to instance A never saw anything instance
//! B published. Events therefore go through the database, which every instance
//! already shares.

use async_trait::async_trait;
use pmk_domain::access::Permission;
use pmk_domain::ids::UserId;
use pmk_domain::tenant::CompanyId;

use crate::PortResult;

/// Who an event is for.
///
/// Evaluated per connected client, so one publish can reach everyone entitled
/// to see it without the publisher knowing who is online.
#[derive(Debug, Clone, PartialEq, Eq, serde::Serialize, serde::Deserialize)]
#[serde(tag = "to", rename_all = "snake_case")]
pub enum Audience {
    /// Everyone in the company.
    Company,
    /// One person, wherever they are connected.
    User { user_id: i32 },
    /// Everyone in the company holding a permission.
    Permission { resource: String, action: String },
    /// Everyone who holds the permission *and* can see the job.
    ///
    /// Separate from `Permission` because job visibility is not a permission:
    /// a supervisor holds `site-diary:read` for the whole company but may only
    /// see the jobs R3 gives them, and an event naming a job they are not on
    /// would tell them that job exists.
    Job {
        job_id: i32,
        resource: String,
        action: String,
    },
}

impl Audience {
    /// Would this client receive the event?
    ///
    /// `sees_job` answers "is this job one the viewer may see", which for a
    /// manager or office user is every job in the company and for a supervisor
    /// is the R3 set resolved when they connected.
    #[must_use]
    pub fn includes(
        &self,
        viewer: UserId,
        has: &dyn Fn(&Permission) -> bool,
        sees_job: &dyn Fn(i32) -> bool,
    ) -> bool {
        let holds = |resource: &str, action: &str| {
            has(&Permission {
                resource: resource.to_string(),
                action: action.to_string(),
            })
        };
        match self {
            Self::Company => true,
            Self::User { user_id } => *user_id == viewer.get(),
            Self::Permission { resource, action } => holds(resource, action),
            Self::Job {
                job_id,
                resource,
                action,
            } => holds(resource, action) && sees_job(*job_id),
        }
    }
}

/// Something worth telling connected clients about.
#[derive(Debug, Clone, PartialEq, Eq, serde::Serialize, serde::Deserialize)]
pub struct BroadcastEvent {
    /// Scopes the event. A client only ever sees its own company's.
    pub company_id: i32,
    /// The SSE event name, e.g. `diary.updated`.
    pub kind: String,
    pub audience: Audience,
    /// The SSE `data:` line. Kept small -- this travels through a NOTIFY
    /// payload, which Postgres caps at 8000 bytes.
    pub payload: serde_json::Value,
}

/// The largest payload a single event may carry.
///
/// Postgres refuses a NOTIFY payload over 8000 bytes, and refusing it here
/// turns "the notification silently never arrived" into a logged error at the
/// point of publishing.
pub const MAX_EVENT_BYTES: usize = 7000;

#[async_trait]
pub trait EventBus: Send + Sync {
    /// Publishes to every instance, including this one.
    async fn publish(&self, event: &BroadcastEvent) -> PortResult<()>;

    /// A receiver for everything published from now on.
    ///
    /// Lagging receivers drop messages rather than blocking the bus: a browser
    /// that has stopped reading must not hold up everyone else's events.
    fn subscribe(&self) -> tokio::sync::broadcast::Receiver<BroadcastEvent>;
}

/// Convenience constructors, so call sites do not hand-build the struct.
impl BroadcastEvent {
    #[must_use]
    pub fn for_company(company: CompanyId, kind: &str, payload: serde_json::Value) -> Self {
        Self {
            company_id: company.get(),
            kind: kind.to_string(),
            audience: Audience::Company,
            payload,
        }
    }

    #[must_use]
    pub fn for_user(
        company: CompanyId,
        user: UserId,
        kind: &str,
        payload: serde_json::Value,
    ) -> Self {
        Self {
            company_id: company.get(),
            kind: kind.to_string(),
            audience: Audience::User {
                user_id: user.get(),
            },
            payload,
        }
    }

    /// For everyone entitled to see one job's activity.
    #[must_use]
    pub fn for_job(
        company: CompanyId,
        job: pmk_domain::ids::JobId,
        resource: &str,
        action: &str,
        kind: &str,
        payload: serde_json::Value,
    ) -> Self {
        Self {
            company_id: company.get(),
            kind: kind.to_string(),
            audience: Audience::Job {
                job_id: job.get(),
                resource: resource.to_string(),
                action: action.to_string(),
            },
            payload,
        }
    }

    #[must_use]
    pub fn for_permission(
        company: CompanyId,
        resource: &str,
        action: &str,
        kind: &str,
        payload: serde_json::Value,
    ) -> Self {
        Self {
            company_id: company.get(),
            kind: kind.to_string(),
            audience: Audience::Permission {
                resource: resource.to_string(),
                action: action.to_string(),
            },
            payload,
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    fn holds<'a>(wanted: &'a [&'a str]) -> impl Fn(&Permission) -> bool + 'a {
        move |p: &Permission| wanted.contains(&format!("{}:{}", p.resource, p.action).as_str())
    }

    #[test]
    fn a_company_event_reaches_everyone_in_it() {
        let a = Audience::Company;
        assert!(a.includes(UserId(1), &holds(&[]), &|_| true));
        assert!(a.includes(UserId(99), &holds(&[]), &|_| true));
    }

    #[test]
    fn a_user_event_reaches_only_that_user() {
        let a = Audience::User { user_id: 7 };
        assert!(a.includes(UserId(7), &holds(&[]), &|_| true));
        assert!(!a.includes(UserId(8), &holds(&[]), &|_| true));
    }

    #[test]
    fn a_permission_event_reaches_only_those_who_hold_it() {
        let a = Audience::Permission {
            resource: "site-diary".into(),
            action: "read".into(),
        };
        assert!(a.includes(UserId(1), &holds(&["site-diary:read"]), &|_| true));
        assert!(!a.includes(UserId(1), &holds(&["jobs:read"]), &|_| true));
        assert!(!a.includes(UserId(1), &holds(&[]), &|_| true));
    }

    #[test]
    fn a_job_event_needs_both_the_permission_and_visibility() {
        let a = Audience::Job {
            job_id: 7,
            resource: "site-diary".into(),
            action: "read".into(),
        };
        let can_read = holds(&["site-diary:read"]);
        // Holds the permission and is on the job.
        assert!(a.includes(UserId(1), &can_read, &|j| j == 7));
        // Holds the permission but is not on the job: an event naming it would
        // tell them the job exists.
        assert!(!a.includes(UserId(1), &can_read, &|j| j == 8));
        // On the job but cannot read diaries.
        assert!(!a.includes(UserId(1), &holds(&[]), &|_| true));
    }

    #[test]
    fn an_event_round_trips_through_json() {
        // It travels as a NOTIFY payload, so the wire form has to survive.
        let e = BroadcastEvent::for_permission(
            CompanyId::new(3),
            "site-diary",
            "read",
            "diary.updated",
            serde_json::json!({ "entryId": 12 }),
        );
        let wire = serde_json::to_string(&e).unwrap();
        let back: BroadcastEvent = serde_json::from_str(&wire).unwrap();
        assert_eq!(e, back);
    }

    // Postgres refuses a NOTIFY payload over 8000 bytes, and the JSON envelope
    // around the payload costs some of that. Checked at compile time, so
    // raising the cap past the limit cannot build.
    const _: () = assert!(MAX_EVENT_BYTES < 8000);

    #[test]
    fn an_oversized_event_is_detectable_before_it_is_sent() {
        let big = serde_json::json!({ "blob": "x".repeat(MAX_EVENT_BYTES) });
        let e = BroadcastEvent::for_company(CompanyId::new(1), "big", big);
        let wire = serde_json::to_string(&e).unwrap();
        assert!(
            wire.len() > MAX_EVENT_BYTES,
            "the publisher must be able to notice this and skip it"
        );
    }
}
