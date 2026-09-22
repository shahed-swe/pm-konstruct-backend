//! Media routes.
//!
//! Uploads are presigned and direct: the client PUTs to storage, the API never
//! buffers the bytes. The server then verifies what actually arrived before
//! recording anything (domain-rules R10).

use axum::extract::{Path, Query, State};
use axum::http::StatusCode;
use axum::routing::{delete, get, post};
use axum::{Json, Router};
use pmk_domain::ids::{DiaryEntryId, DiaryNoteId, JobId, MediaId};
use pmk_ports::repository::MediaOwner;

use crate::dto::{
    ConfirmUploadRequest, DownloadUrlDto, MediaDto, MediaScopeQuery, PrepareUploadRequest,
    PreparedUploadDto,
};
use crate::error::ApiError;
use crate::extract::{JobsRead, JobsWrite, RequirePermission, SiteDiaryRead, SiteDiaryWrite};
use crate::state::AppState;

/// Mounted under `/site-diary/{id}/media`.
pub fn diary_router() -> Router<AppState> {
    Router::new()
        .route("/", get(list_diary_media))
        .route("/prepare", post(prepare_diary_upload))
        .route("/confirm", post(confirm_diary_upload))
}

/// Mounted under `/jobs/{id}/media`.
pub fn job_router() -> Router<AppState> {
    Router::new()
        .route("/", get(list_job_media))
        .route("/prepare", post(prepare_job_upload))
        .route("/confirm", post(confirm_job_upload))
}

/// Mounted at `/media`, for operations that need only a media id.
pub fn router() -> Router<AppState> {
    Router::new()
        .route("/{id}/url", get(download_url))
        .route("/{id}", delete(remove))
}

// ── diary media ─────────────────────────────────────────────────────────────

async fn list_diary_media(
    State(state): State<AppState>,
    RequirePermission(session, ..): RequirePermission<SiteDiaryRead>,
    Path(id): Path<i32>,
) -> Result<Json<Vec<MediaDto>>, ApiError> {
    let m = state
        .media
        .list_for_diary(&session, DiaryEntryId(id))
        .await?;
    Ok(Json(m.into_iter().map(Into::into).collect()))
}

async fn prepare_diary_upload(
    State(state): State<AppState>,
    RequirePermission(session, ..): RequirePermission<SiteDiaryWrite>,
    Path(id): Path<i32>,
    Json(req): Json<PrepareUploadRequest>,
) -> Result<Json<Vec<PreparedUploadDto>>, ApiError> {
    let owner = MediaOwner::Diary {
        entry: DiaryEntryId(id),
        note: req.note_id.map(DiaryNoteId),
    };
    let files = req.files.into_iter().map(Into::into).collect();
    let prepared = state.media.prepare_uploads(&session, owner, files).await?;
    Ok(Json(prepared.into_iter().map(Into::into).collect()))
}

async fn confirm_diary_upload(
    State(state): State<AppState>,
    RequirePermission(session, ..): RequirePermission<SiteDiaryWrite>,
    Path(id): Path<i32>,
    Json(req): Json<ConfirmUploadRequest>,
) -> Result<(StatusCode, Json<MediaDto>), ApiError> {
    let owner = MediaOwner::Diary {
        entry: DiaryEntryId(id),
        note: req.note_id.map(DiaryNoteId),
    };
    let m = state
        .media
        .confirm_upload(
            &session,
            owner,
            &req.stored_name,
            &req.original_name,
            &req.mime_type,
        )
        .await?;
    Ok((StatusCode::CREATED, Json(m.into())))
}

// ── job media ───────────────────────────────────────────────────────────────

async fn list_job_media(
    State(state): State<AppState>,
    RequirePermission(session, ..): RequirePermission<JobsRead>,
    Path(id): Path<i32>,
) -> Result<Json<Vec<MediaDto>>, ApiError> {
    let m = state.media.list_for_job(&session, JobId(id)).await?;
    Ok(Json(m.into_iter().map(Into::into).collect()))
}

async fn prepare_job_upload(
    State(state): State<AppState>,
    RequirePermission(session, ..): RequirePermission<JobsWrite>,
    Path(id): Path<i32>,
    Json(req): Json<PrepareUploadRequest>,
) -> Result<Json<Vec<PreparedUploadDto>>, ApiError> {
    let owner = MediaOwner::Job { job: JobId(id) };
    let files = req.files.into_iter().map(Into::into).collect();
    let prepared = state.media.prepare_uploads(&session, owner, files).await?;
    Ok(Json(prepared.into_iter().map(Into::into).collect()))
}

async fn confirm_job_upload(
    State(state): State<AppState>,
    RequirePermission(session, ..): RequirePermission<JobsWrite>,
    Path(id): Path<i32>,
    Json(req): Json<ConfirmUploadRequest>,
) -> Result<(StatusCode, Json<MediaDto>), ApiError> {
    let owner = MediaOwner::Job { job: JobId(id) };
    let m = state
        .media
        .confirm_upload(
            &session,
            owner,
            &req.stored_name,
            &req.original_name,
            &req.mime_type,
        )
        .await?;
    Ok((StatusCode::CREATED, Json(m.into())))
}

// ── by media id ─────────────────────────────────────────────────────────────

/// A short-lived signed URL for one file.
///
/// Ownership is re-checked here rather than trusting that the caller obtained
/// the id from a list they were entitled to see.
async fn download_url(
    State(state): State<AppState>,
    crate::extract::Entitled(session): crate::extract::Entitled,
    Path(id): Path<i32>,
    Query(q): Query<MediaScopeQuery>,
) -> Result<Json<DownloadUrlDto>, ApiError> {
    let url = state
        .media
        .download_url(&session, MediaId(id), q.job_media)
        .await?;
    Ok(Json(DownloadUrlDto { url }))
}

async fn remove(
    State(state): State<AppState>,
    crate::extract::Entitled(session): crate::extract::Entitled,
    Path(id): Path<i32>,
    Query(q): Query<MediaScopeQuery>,
) -> Result<StatusCode, ApiError> {
    state
        .media
        .delete(&session, MediaId(id), q.job_media)
        .await?;
    Ok(StatusCode::NO_CONTENT)
}
