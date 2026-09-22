//! Job routes.
//!
//! Guards mirror `docs/contract/rbac-matrix.csv`: reads need `jobs:read`,
//! writes need `jobs:write`, and assignment management is manager-only --
//! which is what the legacy `routes/jobs.ts` enforced via `requireJobWrite`
//! and `requireManager`.

use axum::extract::{Path, Query, State};
use axum::http::StatusCode;
use axum::routing::{delete, get, post, put};
use axum::{Json, Router};
use pmk_domain::ids::{JobId, UserId};

use crate::dto::{
    AddAssignmentRequest, AssignmentDto, DeleteJobQuery, JobDto, JobLinkDto, JobLinkRequest,
    JobListQuery, JobPatchRequest, JobTaskDto, JobTaskPatchRequest, JobTaskRequest,
    JobUpsertRequest, TaskNotesRequest,
};
use crate::error::ApiError;
use crate::extract::{JobsRead, JobsWrite, ManagerOnly, RequirePermission, RequireRole};
use crate::state::AppState;

pub fn router() -> Router<AppState> {
    Router::new()
        .route("/", get(list).post(create))
        .route("/{id}", get(get_one).put(update).delete(remove))
        // The Files tab lives with job media, so its handler does too.
        .route("/{id}/files", get(crate::routes::media::job_files))
        .route("/{id}/assignments", get(assignments).post(add_assignment))
        .route("/{id}/assignments/{user_id}", delete(remove_assignment))
        .route("/{id}/assignments/{user_id}/primary", put(set_primary))
        .route("/{id}/tasks", get(list_tasks).post(create_task))
        .route("/{id}/links", get(list_links).post(add_link))
        .route(
            "/{id}/links/{link_id}",
            put(update_link).delete(delete_link),
        )
}

/// Separate router for the top-level `/tasks` mount, which the legacy API
/// exposed alongside the per-job paths.
pub fn tasks_router() -> Router<AppState> {
    Router::new()
        .route("/{id}", put(update_task).delete(delete_task))
        .route("/{id}/notes", put(update_task_notes))
        .route("/", post(|| async { StatusCode::METHOD_NOT_ALLOWED }))
}

async fn list(
    State(state): State<AppState>,
    RequirePermission(session, ..): RequirePermission<JobsRead>,
    Query(q): Query<JobListQuery>,
) -> Result<Json<Vec<JobDto>>, ApiError> {
    let filter = pmk_ports::repository::JobFilter {
        status: q.status,
        supervisor_id: q.supervisor_id.map(UserId),
        search: q.search,
    };
    let jobs = state.jobs.list_with_people(&session, filter).await?;
    Ok(Json(jobs.into_iter().map(Into::into).collect()))
}

async fn get_one(
    State(state): State<AppState>,
    RequirePermission(session, ..): RequirePermission<JobsRead>,
    Path(id): Path<i32>,
) -> Result<Json<JobDto>, ApiError> {
    Ok(Json(
        state
            .jobs
            .get_with_people(&session, JobId(id))
            .await?
            .into(),
    ))
}

async fn create(
    State(state): State<AppState>,
    RequirePermission(session, ..): RequirePermission<JobsWrite>,
    Json(req): Json<JobUpsertRequest>,
) -> Result<(StatusCode, Json<JobDto>), ApiError> {
    let input = req.into_input()?;
    let job = state.jobs.create(&session, input).await?;
    Ok((StatusCode::CREATED, Json(job.into())))
}

/// Updates a job, merging the body onto what is already stored.
///
/// A partial update, as the legacy was: the jobs list archives a job by
/// sending `{"status":"archived"}` alone, and the notes panel autosaves
/// `{"description":"..."}`. Reading the job first also means the visibility
/// check runs before anything is written, so a supervisor cannot blind-write
/// a job they cannot see.
async fn update(
    State(state): State<AppState>,
    RequirePermission(session, ..): RequirePermission<JobsWrite>,
    Path(id): Path<i32>,
    Json(req): Json<JobPatchRequest>,
) -> Result<Json<JobDto>, ApiError> {
    let current = state.jobs.get(&session, JobId(id)).await?;
    let input = req.apply(&current)?;
    state.jobs.update(&session, JobId(id), input).await?;
    Ok(Json(
        state
            .jobs
            .get_with_people(&session, JobId(id))
            .await?
            .into(),
    ))
}

/// Archives a job, or purges it with `?purge=true`.
///
/// Manager-only either way, matching the legacy
/// `router.delete("/:id", requireManager, …)`. The default is now an archive:
/// a cascade from one job can take years of site diary with it, and that
/// should not be one mis-click away (docs/adr/0002-scope-decisions.md).
async fn remove(
    State(state): State<AppState>,
    RequireRole(session): ManagerOnly,
    Path(id): Path<i32>,
    Query(q): Query<DeleteJobQuery>,
) -> Result<StatusCode, ApiError> {
    if q.purge {
        state.jobs.purge(&session, JobId(id)).await?;
    } else {
        state.jobs.archive(&session, JobId(id)).await?;
    }
    Ok(StatusCode::NO_CONTENT)
}

async fn assignments(
    State(state): State<AppState>,
    RequirePermission(session, ..): RequirePermission<JobsRead>,
    Path(id): Path<i32>,
) -> Result<Json<Vec<AssignmentDto>>, ApiError> {
    let a = state.jobs.assignments(&session, JobId(id)).await?;
    Ok(Json(a.into_iter().map(Into::into).collect()))
}

async fn add_assignment(
    State(state): State<AppState>,
    RequireRole(session): ManagerOnly,
    Path(id): Path<i32>,
    Json(req): Json<AddAssignmentRequest>,
) -> Result<Json<Vec<AssignmentDto>>, ApiError> {
    let a = state
        .jobs
        .add_assignment(&session, JobId(id), UserId(req.user_id), req.is_primary)
        .await?;
    Ok(Json(a.into_iter().map(Into::into).collect()))
}

async fn remove_assignment(
    State(state): State<AppState>,
    RequireRole(session): ManagerOnly,
    Path((id, user_id)): Path<(i32, i32)>,
) -> Result<Json<Vec<AssignmentDto>>, ApiError> {
    let a = state
        .jobs
        .remove_assignment(&session, JobId(id), UserId(user_id))
        .await?;
    Ok(Json(a.into_iter().map(Into::into).collect()))
}

async fn set_primary(
    State(state): State<AppState>,
    RequireRole(session): ManagerOnly,
    Path((id, user_id)): Path<(i32, i32)>,
) -> Result<Json<Vec<AssignmentDto>>, ApiError> {
    let a = state
        .jobs
        .set_primary(&session, JobId(id), UserId(user_id))
        .await?;
    Ok(Json(a.into_iter().map(Into::into).collect()))
}

async fn list_tasks(
    State(state): State<AppState>,
    RequirePermission(session, ..): RequirePermission<JobsRead>,
    Path(id): Path<i32>,
) -> Result<Json<Vec<JobTaskDto>>, ApiError> {
    let t = state.jobs.list_tasks(&session, JobId(id)).await?;
    Ok(Json(t.into_iter().map(Into::into).collect()))
}

async fn create_task(
    State(state): State<AppState>,
    RequirePermission(session, ..): RequirePermission<JobsWrite>,
    Path(id): Path<i32>,
    Json(req): Json<JobTaskRequest>,
) -> Result<(StatusCode, Json<JobTaskDto>), ApiError> {
    let t = state
        .jobs
        .create_task(&session, JobId(id), to_task_input(req))
        .await?;
    Ok((StatusCode::CREATED, Json(t.into())))
}

/// Updates a task, merging the body onto what is stored.
///
/// Partial, as the legacy was: the task list toggles a status without
/// sending the title back.
async fn update_task(
    State(state): State<AppState>,
    RequirePermission(session, ..): RequirePermission<JobsWrite>,
    Path(id): Path<i32>,
    Json(req): Json<JobTaskPatchRequest>,
) -> Result<Json<JobTaskDto>, ApiError> {
    let current = state.jobs.get_task(&session, id).await?;
    Ok(Json(
        state
            .jobs
            .update_task(&session, id, req.apply(&current))
            .await?
            .into(),
    ))
}

async fn update_task_notes(
    State(state): State<AppState>,
    RequirePermission(session, ..): RequirePermission<JobsWrite>,
    Path(id): Path<i32>,
    Json(req): Json<TaskNotesRequest>,
) -> Result<Json<JobTaskDto>, ApiError> {
    Ok(Json(
        state
            .jobs
            .update_task_notes(&session, id, req.notes)
            .await?
            .into(),
    ))
}

async fn delete_task(
    State(state): State<AppState>,
    RequirePermission(session, ..): RequirePermission<JobsWrite>,
    Path(id): Path<i32>,
) -> Result<StatusCode, ApiError> {
    state.jobs.delete_task(&session, id).await?;
    Ok(StatusCode::NO_CONTENT)
}

fn to_task_input(r: JobTaskRequest) -> pmk_ports::repository::JobTaskInput {
    pmk_ports::repository::JobTaskInput {
        title: r.title,
        status: r.status,
        notes: r.notes,
        sort_order: r.sort_order,
    }
}

// ── cloud-storage links ─────────────────────────────────────────────────────

async fn list_links(
    State(state): State<AppState>,
    RequirePermission(session, ..): RequirePermission<JobsRead>,
    Path(id): Path<i32>,
) -> Result<Json<Vec<JobLinkDto>>, ApiError> {
    let l = state.jobs.links(&session, JobId(id)).await?;
    Ok(Json(l.into_iter().map(Into::into).collect()))
}

async fn add_link(
    State(state): State<AppState>,
    RequirePermission(session, ..): RequirePermission<JobsWrite>,
    Path(id): Path<i32>,
    Json(req): Json<JobLinkRequest>,
) -> Result<(StatusCode, Json<JobLinkDto>), ApiError> {
    let l = state.jobs.add_link(&session, JobId(id), req.into()).await?;
    Ok((StatusCode::CREATED, Json(l.into())))
}

async fn update_link(
    State(state): State<AppState>,
    RequirePermission(session, ..): RequirePermission<JobsWrite>,
    Path((id, link_id)): Path<(i32, i32)>,
    Json(req): Json<JobLinkRequest>,
) -> Result<Json<JobLinkDto>, ApiError> {
    let l = state
        .jobs
        .update_link(&session, JobId(id), link_id, req.into())
        .await?;
    Ok(Json(l.into()))
}

async fn delete_link(
    State(state): State<AppState>,
    RequirePermission(session, ..): RequirePermission<JobsWrite>,
    Path((id, link_id)): Path<(i32, i32)>,
) -> Result<StatusCode, ApiError> {
    state.jobs.delete_link(&session, JobId(id), link_id).await?;
    Ok(StatusCode::NO_CONTENT)
}
