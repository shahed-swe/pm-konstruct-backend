//! Dashboard routes.
//!
//! Each panel is a separate endpoint so the page can load its counters first
//! and fill the drill-downs as they are expanded, rather than blocking on one
//! large aggregate.
//!
//! The permission on each route is the one guarding the data it returns, not a
//! single "dashboard" permission: the call-forward panels need
//! `call-forward:read`, the job panels `jobs:read`, the diary panel
//! `site-diary:read`. A user who cannot open the call-forward page cannot read
//! its numbers off the dashboard either.

use axum::extract::State;
use axum::routing::get;
use axum::{Json, Router};
use pmk_ports::repository::JobListFilter;

use crate::dto::{
    ActionItemDto, DashboardCallForwardDto, DashboardDiaryDto, DashboardJobDto, DashboardStatsDto,
    DelaySeverityDto, UpcomingClaimDto,
};
use crate::error::ApiError;
use crate::extract::{CallForwardRead, Entitled, JobsRead, RequirePermission, SiteDiaryRead};
use crate::state::AppState;

pub fn router() -> Router<AppState> {
    Router::new()
        .route("/stats", get(stats))
        .route("/action-items", get(action_items))
        .route("/upcoming-claims", get(upcoming_claims))
        .route("/jobs-list", get(jobs_list))
        .route("/active-jobs-list", get(active_jobs_list))
        .route("/completed-jobs-list", get(completed_jobs_list))
        .route("/open-cf-list", get(open_cf_list))
        .route("/overdue-list", get(overdue_list))
        .route("/recent-diary-list", get(recent_diary_list))
        .route("/delay-severity", get(delay_severity))
}

/// The headline counters.
///
/// Guarded only by entitlement, matching the legacy route: the numbers are
/// already scoped to what the caller may see, so a supervisor's counters
/// cover their jobs and nobody else's.
async fn stats(
    State(state): State<AppState>,
    Entitled(session): Entitled,
) -> Result<Json<DashboardStatsDto>, ApiError> {
    Ok(Json(state.dashboard.stats(&session).await?.into()))
}

async fn action_items(
    State(state): State<AppState>,
    Entitled(session): Entitled,
) -> Result<Json<Vec<ActionItemDto>>, ApiError> {
    let items = state.dashboard.action_items(&session).await?;
    Ok(Json(items.into_iter().map(Into::into).collect()))
}

async fn upcoming_claims(
    State(state): State<AppState>,
    RequirePermission(session, ..): RequirePermission<CallForwardRead>,
) -> Result<Json<Vec<UpcomingClaimDto>>, ApiError> {
    let claims = state.dashboard.upcoming_claims(&session).await?;
    Ok(Json(claims.into_iter().map(Into::into).collect()))
}

async fn jobs_list(
    State(state): State<AppState>,
    RequirePermission(session, ..): RequirePermission<JobsRead>,
) -> Result<Json<Vec<DashboardJobDto>>, ApiError> {
    let jobs = state
        .dashboard
        .jobs_list(&session, JobListFilter::All)
        .await?;
    Ok(Json(jobs.into_iter().map(Into::into).collect()))
}

async fn active_jobs_list(
    State(state): State<AppState>,
    RequirePermission(session, ..): RequirePermission<JobsRead>,
) -> Result<Json<Vec<DashboardJobDto>>, ApiError> {
    let jobs = state
        .dashboard
        .jobs_list(&session, JobListFilter::Active)
        .await?;
    Ok(Json(jobs.into_iter().map(Into::into).collect()))
}

async fn completed_jobs_list(
    State(state): State<AppState>,
    RequirePermission(session, ..): RequirePermission<JobsRead>,
) -> Result<Json<Vec<DashboardJobDto>>, ApiError> {
    let jobs = state
        .dashboard
        .jobs_list(&session, JobListFilter::Completed)
        .await?;
    Ok(Json(jobs.into_iter().map(Into::into).collect()))
}

async fn open_cf_list(
    State(state): State<AppState>,
    RequirePermission(session, ..): RequirePermission<CallForwardRead>,
) -> Result<Json<Vec<DashboardCallForwardDto>>, ApiError> {
    let items = state.dashboard.open_call_forwards(&session).await?;
    Ok(Json(items.into_iter().map(Into::into).collect()))
}

async fn overdue_list(
    State(state): State<AppState>,
    RequirePermission(session, ..): RequirePermission<CallForwardRead>,
) -> Result<Json<Vec<DashboardCallForwardDto>>, ApiError> {
    let items = state.dashboard.overdue_call_forwards(&session).await?;
    Ok(Json(items.into_iter().map(Into::into).collect()))
}

async fn recent_diary_list(
    State(state): State<AppState>,
    RequirePermission(session, ..): RequirePermission<SiteDiaryRead>,
) -> Result<Json<Vec<DashboardDiaryDto>>, ApiError> {
    let entries = state.dashboard.recent_diary(&session).await?;
    Ok(Json(entries.into_iter().map(Into::into).collect()))
}

async fn delay_severity(
    State(state): State<AppState>,
    RequirePermission(session, ..): RequirePermission<CallForwardRead>,
) -> Result<Json<Vec<DelaySeverityDto>>, ApiError> {
    let buckets = state.dashboard.delay_severity(&session).await?;
    Ok(Json(buckets.into_iter().map(Into::into).collect()))
}
