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
    BulkDeleteDto, BulkDeleteRequest, ConfirmUploadRequest, DownloadUrlDto, JobFileDto, MediaDto,
    MediaScopeQuery, PrepareUploadRequest, PreparedUploadDto,
};
use crate::error::ApiError;
use crate::extract::{JobsRead, RequirePermission, SiteDiaryRead, SiteDiaryWrite};
use crate::state::AppState;

/// Mounted under `/site-diary/{id}/media`.
pub fn diary_router() -> Router<AppState> {
    Router::new()
        .route("/", get(list_diary_media))
        .route("/prepare", post(prepare_diary_upload))
        .route("/confirm", post(confirm_diary_upload))
}

/// Mounted under `/jobs/{id}/media`.
///
/// Reading needs `jobs:read`; writing needs `site-diary:write`, not
/// `jobs:write`. That looks inconsistent and it is what the contract says
/// (`docs/contract/rbac-matrix.csv`, routes/jobs.ts:154): a job photo is site
/// evidence, so the people who record site evidence own it. Supervisors hold
/// `site-diary:write` by default and `jobs:write` by exception, so gating
/// uploads on `jobs:write` would stop the primary users of the feature.
pub fn job_router() -> Router<AppState> {
    Router::new()
        .route("/", get(list_job_media).delete(delete_selected))
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
    RequirePermission(session, ..): RequirePermission<SiteDiaryWrite>,
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
    RequirePermission(session, ..): RequirePermission<SiteDiaryWrite>,
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

/// The most photos one request may clear.
///
/// The legacy cap. It bounds the row lock the delete takes, so one request
/// cannot hold a large part of a busy job's gallery.
const MAX_BULK_DELETE: usize = 100;

/// Clears selected photos from a job's gallery.
///
/// All or nothing: if any selection is missing the whole request is a 404, and
/// if any is forbidden it is a 403. A partial delete would leave the user
/// unable to tell which of their selections survived.
async fn delete_selected(
    State(state): State<AppState>,
    RequirePermission(session, ..): RequirePermission<SiteDiaryWrite>,
    Path(id): Path<i32>,
    Json(req): Json<BulkDeleteRequest>,
) -> Result<Json<BulkDeleteDto>, ApiError> {
    if req.media.is_empty() {
        return Err(ApiError::new(
            StatusCode::BAD_REQUEST,
            "Select at least one photo to delete.",
        ));
    }
    if req.media.len() > MAX_BULK_DELETE {
        return Err(ApiError::new(
            StatusCode::BAD_REQUEST,
            format!("You can delete up to {MAX_BULK_DELETE} photos at a time."),
        ));
    }

    // Duplicates in the payload would otherwise be counted twice in the
    // response and locked twice in the delete.
    let mut seen = std::collections::HashSet::new();
    let selections: Vec<_> = req
        .media
        .into_iter()
        .map(Into::into)
        .filter(|s| seen.insert(*s))
        .collect();

    let plan = state
        .media
        .delete_selected(&session, JobId(id), &selections)
        .await?;

    if !plan.missing.is_empty() {
        return Err(ApiError::new(
            StatusCode::NOT_FOUND,
            "One or more selected photos no longer exist.",
        ));
    }
    if !plan.forbidden.is_empty() {
        return Err(ApiError::new(
            StatusCode::FORBIDDEN,
            "You can only delete photos you uploaded.",
        ));
    }

    Ok(Json(BulkDeleteDto {
        deleted: plan.deleted.len(),
        cleanup_pending: !plan.deleted.is_empty(),
    }))
}

/// The job's Files tab: uploaded documents plus inspection drafts.
pub async fn job_files(
    State(state): State<AppState>,
    RequirePermission(session, ..): RequirePermission<JobsRead>,
    Path(id): Path<i32>,
) -> Result<Json<Vec<JobFileDto>>, ApiError> {
    let files = state.media.job_files(&session, JobId(id)).await?;
    Ok(Json(files.into_iter().map(Into::into).collect()))
}
