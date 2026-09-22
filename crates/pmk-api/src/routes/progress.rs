//! Progress-record routes.
//!
//! Reading is open to any entitled user, matching the legacy route; writing
//! needs `jobs:write`, because a progress record is a claim about a job's
//! state that feeds the reports.

use axum::extract::{Path, Query, State};
use axum::http::StatusCode;
use axum::routing::get;
use axum::{Json, Router};
use pmk_domain::ids::{JobId, ProgressId};

use crate::dto::{ProgressDto, ProgressListQuery, ProgressPatchRequest, ProgressRequest};
use crate::error::ApiError;
use crate::extract::{Entitled, JobsWrite, RequirePermission};
use crate::state::AppState;

pub fn router() -> Router<AppState> {
    Router::new()
        .route("/", get(list).post(create))
        .route("/{id}", axum::routing::put(update).delete(remove))
}

async fn list(
    State(state): State<AppState>,
    Entitled(session): Entitled,
    Query(q): Query<ProgressListQuery>,
) -> Result<Json<Vec<ProgressDto>>, ApiError> {
    let records = state.progress.list(&session, q.job_id.map(JobId)).await?;
    Ok(Json(records.into_iter().map(Into::into).collect()))
}

async fn create(
    State(state): State<AppState>,
    RequirePermission(session, ..): RequirePermission<JobsWrite>,
    Json(req): Json<ProgressRequest>,
) -> Result<(StatusCode, Json<ProgressDto>), ApiError> {
    let record = state.progress.create(&session, &req.into()).await?;
    Ok((StatusCode::CREATED, Json(record.into())))
}

async fn update(
    State(state): State<AppState>,
    RequirePermission(session, ..): RequirePermission<JobsWrite>,
    Path(id): Path<i32>,
    Json(req): Json<ProgressPatchRequest>,
) -> Result<Json<ProgressDto>, ApiError> {
    let current = state.progress.get(&session, ProgressId(id)).await?;
    let record = state
        .progress
        .update(&session, ProgressId(id), &req.apply(&current))
        .await?;
    Ok(Json(record.into()))
}

async fn remove(
    State(state): State<AppState>,
    RequirePermission(session, ..): RequirePermission<JobsWrite>,
    Path(id): Path<i32>,
) -> Result<StatusCode, ApiError> {
    state.progress.delete(&session, ProgressId(id)).await?;
    Ok(StatusCode::NO_CONTENT)
}
