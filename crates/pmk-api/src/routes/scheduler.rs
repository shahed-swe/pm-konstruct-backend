//! Trade scheduler routes.
//!
//! Guards follow docs/contract/rbac-matrix.csv: reads need
//! `trade-scheduler:read`, mutations need `trade-scheduler:write`.

use axum::extract::{Path, Query, State};
use axum::http::StatusCode;
use axum::routing::{delete, get};
use axum::{Json, Router};

use crate::dto::{
    absence_id, allocation_id, job_id, worker_id, AbsenceDto, AbsenceRequest, AllocationDto,
    AllocationRequest, BoardDto, DayNoteDto, DayNoteRequest, MaintenanceJobDto,
    MaintenanceJobRequest, RangeQuery, WorkerDto, WorkerListQuery, WorkerRequest,
};
use crate::error::ApiError;
use crate::extract::{RequirePermission, TradeSchedulerRead, TradeSchedulerWrite};
use crate::state::AppState;

pub fn router() -> Router<AppState> {
    Router::new()
        // One call for a whole board window, replacing the legacy
        // per-worker-per-day fan-out.
        .route("/board", get(board))
        .route("/workers", get(workers).post(create_worker))
        .route(
            "/workers/{id}",
            axum::routing::put(update_worker).delete(deactivate_worker),
        )
        .route("/workers/{id}/permanent", delete(purge_worker))
        .route("/worker-absences", get(absences).post(create_absence))
        .route(
            "/worker-absences/{id}",
            axum::routing::put(update_absence).delete(delete_absence),
        )
        .route("/allocations", get(allocations).post(create_allocation))
        .route(
            "/allocations/{id}",
            axum::routing::put(update_allocation).delete(delete_allocation),
        )
        .route(
            "/maintenance-jobs",
            get(maintenance_jobs).post(create_maintenance_job),
        )
        .route(
            "/maintenance-jobs/{id}",
            axum::routing::put(update_maintenance_job).delete(delete_maintenance_job),
        )
        .route("/job-day-notes", get(day_notes).post(set_day_note))
        .route("/job-day-notes/{id}", delete(delete_day_note))
        // The legacy `/scheduler/jobs` returned the jobs the board can
        // allocate against; that is the ordinary job list.
        .route("/jobs", get(schedulable_jobs))
}

async fn board(
    State(state): State<AppState>,
    RequirePermission(session, ..): RequirePermission<TradeSchedulerRead>,
    Query(q): Query<RangeQuery>,
) -> Result<Json<BoardDto>, ApiError> {
    let range = q.into();
    let data = state.scheduler.board(&session, range).await?;
    Ok(Json(BoardDto::new(range, data)))
}

// ── workers ─────────────────────────────────────────────────────────────────

async fn workers(
    State(state): State<AppState>,
    RequirePermission(session, ..): RequirePermission<TradeSchedulerRead>,
    Query(q): Query<WorkerListQuery>,
) -> Result<Json<Vec<WorkerDto>>, ApiError> {
    let w = state
        .scheduler
        .workers(&session, q.include_inactive)
        .await?;
    Ok(Json(w.into_iter().map(Into::into).collect()))
}

async fn create_worker(
    State(state): State<AppState>,
    RequirePermission(session, ..): RequirePermission<TradeSchedulerWrite>,
    Json(req): Json<WorkerRequest>,
) -> Result<(StatusCode, Json<WorkerDto>), ApiError> {
    let w = state.scheduler.create_worker(&session, req.into()).await?;
    Ok((StatusCode::CREATED, Json(w.into())))
}

async fn update_worker(
    State(state): State<AppState>,
    RequirePermission(session, ..): RequirePermission<TradeSchedulerWrite>,
    Path(id): Path<i32>,
    Json(req): Json<WorkerRequest>,
) -> Result<Json<WorkerDto>, ApiError> {
    let w = state
        .scheduler
        .update_worker(&session, worker_id(id), req.into())
        .await?;
    Ok(Json(w.into()))
}

/// Deactivates. Past allocations stay on the board.
async fn deactivate_worker(
    State(state): State<AppState>,
    RequirePermission(session, ..): RequirePermission<TradeSchedulerWrite>,
    Path(id): Path<i32>,
) -> Result<StatusCode, ApiError> {
    state
        .scheduler
        .deactivate_worker(&session, worker_id(id))
        .await?;
    Ok(StatusCode::NO_CONTENT)
}

/// Deletes for real, cascading to allocations and absences.
async fn purge_worker(
    State(state): State<AppState>,
    RequirePermission(session, ..): RequirePermission<TradeSchedulerWrite>,
    Path(id): Path<i32>,
) -> Result<StatusCode, ApiError> {
    state
        .scheduler
        .purge_worker(&session, worker_id(id))
        .await?;
    Ok(StatusCode::NO_CONTENT)
}

// ── absences ────────────────────────────────────────────────────────────────

async fn absences(
    State(state): State<AppState>,
    RequirePermission(session, ..): RequirePermission<TradeSchedulerRead>,
    Query(q): Query<RangeQuery>,
) -> Result<Json<Vec<AbsenceDto>>, ApiError> {
    let a = state.scheduler.absences(&session, q.into()).await?;
    Ok(Json(a.into_iter().map(Into::into).collect()))
}

async fn create_absence(
    State(state): State<AppState>,
    RequirePermission(session, ..): RequirePermission<TradeSchedulerWrite>,
    Json(req): Json<AbsenceRequest>,
) -> Result<(StatusCode, Json<AbsenceDto>), ApiError> {
    let a = state
        .scheduler
        .create_absence(&session, req.into_input()?)
        .await?;
    Ok((StatusCode::CREATED, Json(a.into())))
}

async fn update_absence(
    State(state): State<AppState>,
    RequirePermission(session, ..): RequirePermission<TradeSchedulerWrite>,
    Path(id): Path<i32>,
    Json(req): Json<AbsenceRequest>,
) -> Result<Json<AbsenceDto>, ApiError> {
    let a = state
        .scheduler
        .update_absence(&session, absence_id(id), req.into_input()?)
        .await?;
    Ok(Json(a.into()))
}

async fn delete_absence(
    State(state): State<AppState>,
    RequirePermission(session, ..): RequirePermission<TradeSchedulerWrite>,
    Path(id): Path<i32>,
) -> Result<StatusCode, ApiError> {
    state
        .scheduler
        .delete_absence(&session, absence_id(id))
        .await?;
    Ok(StatusCode::NO_CONTENT)
}

// ── allocations ─────────────────────────────────────────────────────────────

async fn allocations(
    State(state): State<AppState>,
    RequirePermission(session, ..): RequirePermission<TradeSchedulerRead>,
    Query(q): Query<RangeQuery>,
) -> Result<Json<Vec<AllocationDto>>, ApiError> {
    let a = state.scheduler.allocations(&session, q.into()).await?;
    Ok(Json(a.into_iter().map(Into::into).collect()))
}

async fn create_allocation(
    State(state): State<AppState>,
    RequirePermission(session, ..): RequirePermission<TradeSchedulerWrite>,
    Json(req): Json<AllocationRequest>,
) -> Result<(StatusCode, Json<AllocationDto>), ApiError> {
    let a = state
        .scheduler
        .create_allocation(&session, req.into())
        .await?;
    Ok((StatusCode::CREATED, Json(a.into())))
}

async fn update_allocation(
    State(state): State<AppState>,
    RequirePermission(session, ..): RequirePermission<TradeSchedulerWrite>,
    Path(id): Path<i32>,
    Json(req): Json<AllocationRequest>,
) -> Result<Json<AllocationDto>, ApiError> {
    let a = state
        .scheduler
        .update_allocation(&session, allocation_id(id), req.into())
        .await?;
    Ok(Json(a.into()))
}

async fn delete_allocation(
    State(state): State<AppState>,
    RequirePermission(session, ..): RequirePermission<TradeSchedulerWrite>,
    Path(id): Path<i32>,
) -> Result<StatusCode, ApiError> {
    state
        .scheduler
        .delete_allocation(&session, allocation_id(id))
        .await?;
    Ok(StatusCode::NO_CONTENT)
}

// ── maintenance jobs ────────────────────────────────────────────────────────

async fn maintenance_jobs(
    State(state): State<AppState>,
    RequirePermission(session, ..): RequirePermission<TradeSchedulerRead>,
) -> Result<Json<Vec<MaintenanceJobDto>>, ApiError> {
    let m = state.scheduler.maintenance_jobs(&session).await?;
    Ok(Json(m.into_iter().map(Into::into).collect()))
}

async fn create_maintenance_job(
    State(state): State<AppState>,
    RequirePermission(session, ..): RequirePermission<TradeSchedulerWrite>,
    Json(req): Json<MaintenanceJobRequest>,
) -> Result<(StatusCode, Json<MaintenanceJobDto>), ApiError> {
    let m = state
        .scheduler
        .create_maintenance_job(&session, req.into())
        .await?;
    Ok((StatusCode::CREATED, Json(m.into())))
}

async fn update_maintenance_job(
    State(state): State<AppState>,
    RequirePermission(session, ..): RequirePermission<TradeSchedulerWrite>,
    Path(id): Path<i32>,
    Json(req): Json<MaintenanceJobRequest>,
) -> Result<Json<MaintenanceJobDto>, ApiError> {
    let m = state
        .scheduler
        .update_maintenance_job(&session, id, req.into())
        .await?;
    Ok(Json(m.into()))
}

async fn delete_maintenance_job(
    State(state): State<AppState>,
    RequirePermission(session, ..): RequirePermission<TradeSchedulerWrite>,
    Path(id): Path<i32>,
) -> Result<StatusCode, ApiError> {
    state.scheduler.delete_maintenance_job(&session, id).await?;
    Ok(StatusCode::NO_CONTENT)
}

// ── day notes ───────────────────────────────────────────────────────────────

async fn day_notes(
    State(state): State<AppState>,
    RequirePermission(session, ..): RequirePermission<TradeSchedulerRead>,
    Query(q): Query<RangeQuery>,
) -> Result<Json<Vec<DayNoteDto>>, ApiError> {
    let n = state.scheduler.day_notes(&session, q.into()).await?;
    Ok(Json(n.into_iter().map(Into::into).collect()))
}

/// Upserts: one note per job per day.
async fn set_day_note(
    State(state): State<AppState>,
    RequirePermission(session, ..): RequirePermission<TradeSchedulerWrite>,
    Json(req): Json<DayNoteRequest>,
) -> Result<Json<DayNoteDto>, ApiError> {
    let n = state
        .scheduler
        .set_day_note(&session, job_id(req.job_id), req.note_date, req.note)
        .await?;
    // 200, not 201: this is an upsert on (job_id, note_date), so the caller
    // cannot tell whether a row was created, and the legacy handler returned
    // a bare `res.json(row)` either way.
    Ok(Json(n.into()))
}

async fn delete_day_note(
    State(state): State<AppState>,
    RequirePermission(session, ..): RequirePermission<TradeSchedulerWrite>,
    Path(id): Path<i32>,
) -> Result<StatusCode, ApiError> {
    state.scheduler.delete_day_note(&session, id).await?;
    Ok(StatusCode::NO_CONTENT)
}

/// Jobs the board can allocate against.
async fn schedulable_jobs(
    State(state): State<AppState>,
    RequirePermission(session, ..): RequirePermission<TradeSchedulerRead>,
) -> Result<Json<Vec<crate::dto::JobDto>>, ApiError> {
    // Only active jobs: allocating someone to a completed job is a mistake.
    let filter = pmk_ports::repository::JobFilter {
        status: Some("active".into()),
        ..Default::default()
    };
    let jobs = state.jobs.list(&session, filter).await?;
    Ok(Json(jobs.into_iter().map(Into::into).collect()))
}
