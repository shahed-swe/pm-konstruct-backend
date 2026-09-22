//! The live event stream.
//!
//! One long-lived SSE connection per browser tab, carrying whatever the user
//! is entitled to see. Events arrive from the database rather than from this
//! process, so a change made on another API instance reaches this client too.
//!
//! The token is never accepted in the query string: a URL ends up in access
//! logs, proxy logs and browser history, and this URL stays open for hours.

use axum::extract::State;
use axum::response::sse::{Event, KeepAlive, Sse};
use axum::response::IntoResponse;
use axum::routing::get;
use axum::Router;
use pmk_domain::access::Permission;
use std::convert::Infallible;
use std::time::Duration;
use tokio_stream::wrappers::BroadcastStream;
use tokio_stream::StreamExt as _;

use crate::extract::Entitled;
use crate::state::AppState;

/// How often a comment line is sent to keep intermediaries from closing an
/// idle connection. Shorter than the usual 60-second proxy timeout.
const HEARTBEAT: Duration = Duration::from_secs(25);

pub fn router() -> Router<AppState> {
    Router::new().route("/", get(stream))
}

async fn stream(State(state): State<AppState>, Entitled(session): Entitled) -> impl IntoResponse {
    let company = session.principal.company_id().get();
    let viewer = session.user.id;

    // Permissions are resolved once, at connect time. The alternative -- a
    // permission lookup per event per client -- would put a query on the hot
    // path of every broadcast, and a permission change already requires the
    // user to reconnect for other reasons.
    let permissions: Vec<Permission> = session.effective();

    // Job visibility is resolved once as well. A supervisor holds
    // `site-diary:read` for the whole company but may only see the jobs R3
    // gives them, so an event naming a job they are not on must not reach
    // them -- it would tell them that job exists.
    let visible_jobs: Option<Vec<i32>> = if session.user.role.sees_all_company_jobs() {
        None
    } else {
        Some(
            state
                .jobs
                .visible_job_ids(&session)
                .await
                .unwrap_or_default()
                .into_iter()
                .map(|j| j.get())
                .collect(),
        )
    };

    let events = BroadcastStream::new(state.events.subscribe()).filter_map(move |msg| {
        // A lagged receiver yields an error; skipping it is correct, because
        // the client refetches on reconnect and cannot act on an event it
        // never saw anyway.
        let event = msg.ok()?;
        if event.company_id != company {
            return None;
        }
        let has = |p: &Permission| permissions.contains(p);
        let sees_job = |j: i32| visible_jobs.as_ref().is_none_or(|ids| ids.contains(&j));
        if !event.audience.includes(viewer, &has, &sees_job) {
            return None;
        }
        Some(Ok::<Event, Infallible>(
            Event::default()
                .event(event.kind.clone())
                .data(event.payload.to_string()),
        ))
    });

    Sse::new(events).keep_alive(KeepAlive::new().interval(HEARTBEAT).text("heartbeat"))
}
