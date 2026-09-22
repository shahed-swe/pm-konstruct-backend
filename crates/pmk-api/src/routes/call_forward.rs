//! Call-forward routes.

use axum::extract::{Path, Query, State};
use axum::http::StatusCode;
use axum::routing::{get, post};
use axum::{Json, Router};
use pmk_domain::ids::{CallForwardItemId, JobId};

use crate::dto::{
    BulkCreateRequest, CallForwardDto, CallForwardListQuery, CallForwardRequest, ReorderRequest,
    ReorderResponse, UpcomingQuery,
};
use crate::error::ApiError;
use crate::extract::{CallForwardRead, CallForwardWrite, RequirePermission};
use crate::state::AppState;

pub fn router() -> Router<AppState> {
    Router::new()
        .route("/", get(list).post(create))
        .route("/upcoming", get(upcoming))
        .route("/bulk", post(create_bulk))
        .route("/reorder", post(reorder))
        .route("/{id}", get(get_one).put(update).delete(remove))
}

async fn list(
    State(state): State<AppState>,
    RequirePermission(session, ..): RequirePermission<CallForwardRead>,
    Query(q): Query<CallForwardListQuery>,
) -> Result<Json<Vec<CallForwardDto>>, ApiError> {
    let filter = pmk_ports::repository::CallForwardFilter {
        job_id: q.job_id.map(JobId),
        status: q.status,
        parent_id: q.parent_id.map(CallForwardItemId),
    };
    let items = state.call_forward.list(&session, filter).await?;
    Ok(Json(items.into_iter().map(Into::into).collect()))
}

async fn upcoming(
    State(state): State<AppState>,
    RequirePermission(session, ..): RequirePermission<CallForwardRead>,
    Query(q): Query<UpcomingQuery>,
) -> Result<Json<Vec<CallForwardDto>>, ApiError> {
    // Clamped so a caller cannot ask for an unbounded scan.
    let days = q.days.unwrap_or(14).clamp(1, 365);
    let items = state.call_forward.upcoming(&session, days).await?;
    Ok(Json(items.into_iter().map(Into::into).collect()))
}

async fn get_one(
    State(state): State<AppState>,
    RequirePermission(session, ..): RequirePermission<CallForwardRead>,
    Path(id): Path<i32>,
) -> Result<Json<CallForwardDto>, ApiError> {
    Ok(Json(
        state
            .call_forward
            .get(&session, CallForwardItemId(id))
            .await?
            .into(),
    ))
}

async fn create(
    State(state): State<AppState>,
    RequirePermission(session, ..): RequirePermission<CallForwardWrite>,
    Json(req): Json<CallForwardRequest>,
) -> Result<(StatusCode, Json<CallForwardDto>), ApiError> {
    let job = req.job_id.ok_or_else(|| {
        ApiError::new(StatusCode::BAD_REQUEST, "jobId is required").with_field("jobId")
    })?;
    let (input, _) = req.into_input()?;
    let item = state
        .call_forward
        .create(&session, JobId(job), input)
        .await?;
    Ok((StatusCode::CREATED, Json(item.into())))
}

async fn create_bulk(
    State(state): State<AppState>,
    RequirePermission(session, ..): RequirePermission<CallForwardWrite>,
    Json(req): Json<BulkCreateRequest>,
) -> Result<(StatusCode, Json<Vec<CallForwardDto>>), ApiError> {
    let mut items = Vec::with_capacity(req.items.len());
    for r in req.items {
        items.push(r.into_input()?);
    }
    let created = state
        .call_forward
        .create_bulk(&session, JobId(req.job_id), items)
        .await?;
    Ok((
        StatusCode::CREATED,
        Json(created.into_iter().map(Into::into).collect()),
    ))
}

async fn update(
    State(state): State<AppState>,
    RequirePermission(session, ..): RequirePermission<CallForwardWrite>,
    Path(id): Path<i32>,
    Json(req): Json<CallForwardRequest>,
) -> Result<Json<CallForwardDto>, ApiError> {
    let (input, _) = req.into_input()?;
    let item = state
        .call_forward
        .update(&session, CallForwardItemId(id), input)
        .await?;
    Ok(Json(item.into()))
}

async fn remove(
    State(state): State<AppState>,
    RequirePermission(session, ..): RequirePermission<CallForwardWrite>,
    Path(id): Path<i32>,
) -> Result<StatusCode, ApiError> {
    state
        .call_forward
        .delete(&session, CallForwardItemId(id))
        .await?;
    Ok(StatusCode::NO_CONTENT)
}

async fn reorder(
    State(state): State<AppState>,
    RequirePermission(session, ..): RequirePermission<CallForwardWrite>,
    Json(req): Json<ReorderRequest>,
) -> Result<Json<ReorderResponse>, ApiError> {
    let entries: Vec<pmk_ports::repository::ReorderEntry> = req
        .items
        .into_iter()
        .map(|i| pmk_ports::repository::ReorderEntry {
            id: CallForwardItemId(i.id),
            sort_order: i.sort_order,
            parent_id: i.parent_id.map(|p| p.map(CallForwardItemId)),
        })
        .collect();
    let updated = state.call_forward.reorder(&session, entries).await?;
    Ok(Json(ReorderResponse { updated }))
}
