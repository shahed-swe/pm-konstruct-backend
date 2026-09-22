//! Report routes.
//!
//! Every report is gated on `reports:read`, and every one is scoped twice: by
//! RLS to the company, and by R3 to the jobs the caller may see. Naming a job
//! outside that scope is a 403 rather than an empty report -- the caller asked
//! about one job, so silence would read as "nothing to report" rather than
//! "not yours".

use axum::extract::{Query, State};
use axum::http::StatusCode;
use axum::routing::get;
use axum::{Json, Router};
use pmk_domain::ids::JobId;

use crate::dto::{
    CreateReportRequest, DailyProgressDto, DailyProgressQuery, DelayDto, DiaryReportDto,
    InspectionDto, JobProgressDto, ReportMetaDto, ReportQueryParams, StageClaimDto,
    StoredReportDto, SupervisorPerformanceDto, UpcomingTaskDto, WeatherImpactDto,
};
use crate::error::ApiError;
use crate::extract::{ManagerOnly, ReportsRead, RequirePermission, RequireRole};
use crate::state::AppState;

pub fn router() -> Router<AppState> {
    Router::new()
        .route("/", get(stored).post(create))
        .route("/meta", get(meta))
        .route("/job-progress", get(job_progress))
        .route("/delays", get(delays))
        .route("/site-diary-summary", get(diary_summary))
        .route("/upcoming-tasks", get(upcoming_tasks))
        .route("/daily-progress", get(daily_progress))
        .route("/weather-impact", get(weather))
        .route("/stage-claims", get(stage_claims))
        .route("/inspections", get(inspections))
}

/// Mounted at `/supervisor-performance`, which the legacy kept outside
/// `/reports` even though it is one.
pub fn performance_router() -> Router<AppState> {
    Router::new().route("/", get(supervisor_performance))
}

async fn stored(
    State(state): State<AppState>,
    RequirePermission(session, ..): RequirePermission<ReportsRead>,
    Query(q): Query<ReportQueryParams>,
) -> Result<Json<Vec<StoredReportDto>>, ApiError> {
    let rows = state.reports.stored(&session, &q.into()).await?;
    Ok(Json(rows.into_iter().map(Into::into).collect()))
}

/// Saving a generated report is manager-only, as in the legacy.
async fn create(
    State(state): State<AppState>,
    RequireRole(session): ManagerOnly,
    Json(req): Json<CreateReportRequest>,
) -> Result<(StatusCode, Json<StoredReportDto>), ApiError> {
    let report = state.reports.create_stored(&session, &req.into()).await?;
    Ok((StatusCode::CREATED, Json(report.into())))
}

async fn meta(
    State(state): State<AppState>,
    RequirePermission(session, ..): RequirePermission<ReportsRead>,
) -> Result<Json<ReportMetaDto>, ApiError> {
    Ok(Json(state.reports.meta(&session).await?.into()))
}

async fn job_progress(
    State(state): State<AppState>,
    RequirePermission(session, ..): RequirePermission<ReportsRead>,
    Query(q): Query<ReportQueryParams>,
) -> Result<Json<Vec<JobProgressDto>>, ApiError> {
    let rows = state.reports.job_progress(&session, &q.into()).await?;
    Ok(Json(rows.into_iter().map(Into::into).collect()))
}

async fn delays(
    State(state): State<AppState>,
    RequirePermission(session, ..): RequirePermission<ReportsRead>,
    Query(q): Query<ReportQueryParams>,
) -> Result<Json<Vec<DelayDto>>, ApiError> {
    let rows = state.reports.delays(&session, &q.into()).await?;
    Ok(Json(rows.into_iter().map(Into::into).collect()))
}

async fn diary_summary(
    State(state): State<AppState>,
    RequirePermission(session, ..): RequirePermission<ReportsRead>,
    Query(q): Query<ReportQueryParams>,
) -> Result<Json<Vec<DiaryReportDto>>, ApiError> {
    let rows = state.reports.diary_summary(&session, &q.into()).await?;
    Ok(Json(rows.into_iter().map(Into::into).collect()))
}

async fn upcoming_tasks(
    State(state): State<AppState>,
    RequirePermission(session, ..): RequirePermission<ReportsRead>,
    Query(q): Query<ReportQueryParams>,
) -> Result<Json<Vec<UpcomingTaskDto>>, ApiError> {
    let rows = state.reports.upcoming_tasks(&session, &q.into()).await?;
    Ok(Json(rows.into_iter().map(Into::into).collect()))
}

async fn stage_claims(
    State(state): State<AppState>,
    RequirePermission(session, ..): RequirePermission<ReportsRead>,
    Query(q): Query<ReportQueryParams>,
) -> Result<Json<Vec<StageClaimDto>>, ApiError> {
    let rows = state.reports.stage_claims(&session, &q.into()).await?;
    Ok(Json(rows.into_iter().map(Into::into).collect()))
}

async fn inspections(
    State(state): State<AppState>,
    RequirePermission(session, ..): RequirePermission<ReportsRead>,
    Query(q): Query<ReportQueryParams>,
) -> Result<Json<Vec<InspectionDto>>, ApiError> {
    let rows = state.reports.inspections(&session, &q.into()).await?;
    Ok(Json(rows.into_iter().map(Into::into).collect()))
}

async fn weather(
    State(state): State<AppState>,
    RequirePermission(session, ..): RequirePermission<ReportsRead>,
    Query(q): Query<ReportQueryParams>,
) -> Result<Json<WeatherImpactDto>, ApiError> {
    Ok(Json(
        state.reports.weather(&session, &q.into()).await?.into(),
    ))
}

/// One job on one day. Both parameters are required: there is no sensible
/// default job, and defaulting the date would make the report change under the
/// reader at midnight.
async fn daily_progress(
    State(state): State<AppState>,
    RequirePermission(session, ..): RequirePermission<ReportsRead>,
    Query(q): Query<DailyProgressQuery>,
) -> Result<Json<DailyProgressDto>, ApiError> {
    let report = state
        .reports
        .daily_progress(&session, JobId(q.job_id), q.date)
        .await?;
    Ok(Json(report.into()))
}

/// Manager-only: it grades named people, and a supervisor reading their peers'
/// scores is not something the legacy allowed either.
async fn supervisor_performance(
    State(state): State<AppState>,
    RequireRole(session): ManagerOnly,
    Query(q): Query<ReportQueryParams>,
) -> Result<Json<Vec<SupervisorPerformanceDto>>, ApiError> {
    let rows = state
        .reports
        .supervisor_performance(&session, &q.into())
        .await?;
    Ok(Json(rows.into_iter().map(Into::into).collect()))
}
