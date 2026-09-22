//! Site diary routes.
//!
//! Guards follow docs/contract/rbac-matrix.csv: reads need `site-diary:read`,
//! mutations need `site-diary:write`.

use axum::extract::{Path, Query, State};
use axum::http::StatusCode;
use axum::routing::{get, patch, post};
use axum::{Json, Router};
use pmk_domain::ids::{DiaryEntryId, DiaryNoteId, JobId};

use crate::dto::{
    parse_action, ActionStatusRequest, CommentRequest, DiaryCommentDto, DiaryEntryDto,
    DiaryEntryRequest, DiaryListQuery, DiaryNoteDto, DiaryNoteRequest,
};
use crate::error::ApiError;
use crate::extract::{RequirePermission, SiteDiaryRead, SiteDiaryWrite};
use crate::state::AppState;

pub fn router() -> Router<AppState> {
    Router::new()
        .route("/", get(list).post(create))
        // Ahead of `/{id}` so "job" is never parsed as an entry id.
        .route("/job/{job_id}/etos", get(job_etos))
        .route("/{id}/weather-snapshot", get(weather_snapshot))
        .route("/{id}/email", post(email_entry))
        .route("/{id}", get(get_one).put(update).delete(remove))
        .route("/{id}/action-status", patch(set_action_status))
        .route("/{id}/notes", get(notes).post(add_note))
        .route(
            "/{id}/notes/{note_id}",
            patch(update_note).delete(delete_note),
        )
        .route(
            "/{id}/notes/{note_id}/action-status",
            patch(set_note_action),
        )
        .route("/{id}/notes/{note_id}/archive", patch(archive_note))
        .route("/{id}/notes/{note_id}/unarchive", patch(unarchive_note))
        .route(
            "/{id}/notes/{note_id}/comments",
            get(comments).post(add_comment),
        )
}

async fn list(
    State(state): State<AppState>,
    RequirePermission(session, ..): RequirePermission<SiteDiaryRead>,
    Query(q): Query<DiaryListQuery>,
) -> Result<Json<Vec<DiaryEntryDto>>, ApiError> {
    let filter = pmk_ports::repository::DiaryFilter {
        job_id: q.job_id.map(JobId),
        from: q.from,
        to: q.to,
        action_status: q.action_status,
    };
    let entries = state.diary.list_with_context(&session, filter).await?;
    Ok(Json(entries.into_iter().map(Into::into).collect()))
}

async fn get_one(
    State(state): State<AppState>,
    RequirePermission(session, ..): RequirePermission<SiteDiaryRead>,
    Path(id): Path<i32>,
) -> Result<Json<DiaryEntryDto>, ApiError> {
    Ok(Json(
        state
            .diary
            .get_with_context(&session, DiaryEntryId(id))
            .await?
            .into(),
    ))
}

async fn create(
    State(state): State<AppState>,
    RequirePermission(session, ..): RequirePermission<SiteDiaryWrite>,
    Json(req): Json<DiaryEntryRequest>,
) -> Result<(StatusCode, Json<DiaryEntryDto>), ApiError> {
    let entry = state.diary.create(&session, req.into_input()?).await?;
    Ok((StatusCode::CREATED, Json(entry.into())))
}

async fn update(
    State(state): State<AppState>,
    RequirePermission(session, ..): RequirePermission<SiteDiaryWrite>,
    Path(id): Path<i32>,
    Json(req): Json<DiaryEntryRequest>,
) -> Result<Json<DiaryEntryDto>, ApiError> {
    let entry = state
        .diary
        .update(&session, DiaryEntryId(id), req.into_input()?)
        .await?;
    Ok(Json(entry.into()))
}

async fn remove(
    State(state): State<AppState>,
    RequirePermission(session, ..): RequirePermission<SiteDiaryWrite>,
    Path(id): Path<i32>,
) -> Result<StatusCode, ApiError> {
    state.diary.delete(&session, DiaryEntryId(id)).await?;
    Ok(StatusCode::NO_CONTENT)
}

async fn set_action_status(
    State(state): State<AppState>,
    RequirePermission(session, ..): RequirePermission<SiteDiaryWrite>,
    Path(id): Path<i32>,
    Json(req): Json<ActionStatusRequest>,
) -> Result<Json<DiaryEntryDto>, ApiError> {
    let status = parse_action(req.action_status.as_deref())?;
    let entry = state
        .diary
        .set_action_status(&session, DiaryEntryId(id), status)
        .await?;
    Ok(Json(entry.into()))
}

async fn notes(
    State(state): State<AppState>,
    RequirePermission(session, ..): RequirePermission<SiteDiaryRead>,
    Path(id): Path<i32>,
    Query(q): Query<DiaryListQuery>,
) -> Result<Json<Vec<DiaryNoteDto>>, ApiError> {
    let n = state
        .diary
        .notes(&session, DiaryEntryId(id), q.include_archived)
        .await?;
    Ok(Json(n.into_iter().map(Into::into).collect()))
}

async fn add_note(
    State(state): State<AppState>,
    RequirePermission(session, ..): RequirePermission<SiteDiaryWrite>,
    Path(id): Path<i32>,
    Json(req): Json<DiaryNoteRequest>,
) -> Result<(StatusCode, Json<DiaryNoteDto>), ApiError> {
    let n = state
        .diary
        .add_note(&session, DiaryEntryId(id), req.into_input()?)
        .await?;
    Ok((StatusCode::CREATED, Json(n.into())))
}

async fn update_note(
    State(state): State<AppState>,
    RequirePermission(session, ..): RequirePermission<SiteDiaryWrite>,
    Path((id, note_id)): Path<(i32, i32)>,
    Json(req): Json<DiaryNoteRequest>,
) -> Result<Json<DiaryNoteDto>, ApiError> {
    let n = state
        .diary
        .update_note(
            &session,
            DiaryEntryId(id),
            DiaryNoteId(note_id),
            req.into_input()?,
        )
        .await?;
    Ok(Json(n.into()))
}

async fn set_note_action(
    State(state): State<AppState>,
    RequirePermission(session, ..): RequirePermission<SiteDiaryWrite>,
    Path((id, note_id)): Path<(i32, i32)>,
    Json(req): Json<ActionStatusRequest>,
) -> Result<Json<DiaryNoteDto>, ApiError> {
    let status = parse_action(req.action_status.as_deref())?;
    let n = state
        .diary
        .set_note_action_status(&session, DiaryEntryId(id), DiaryNoteId(note_id), status)
        .await?;
    Ok(Json(n.into()))
}

async fn archive_note(
    State(state): State<AppState>,
    RequirePermission(session, ..): RequirePermission<SiteDiaryWrite>,
    Path((id, note_id)): Path<(i32, i32)>,
) -> Result<Json<DiaryNoteDto>, ApiError> {
    let n = state
        .diary
        .set_note_archived(&session, DiaryEntryId(id), DiaryNoteId(note_id), true)
        .await?;
    Ok(Json(n.into()))
}

async fn unarchive_note(
    State(state): State<AppState>,
    RequirePermission(session, ..): RequirePermission<SiteDiaryWrite>,
    Path((id, note_id)): Path<(i32, i32)>,
) -> Result<Json<DiaryNoteDto>, ApiError> {
    let n = state
        .diary
        .set_note_archived(&session, DiaryEntryId(id), DiaryNoteId(note_id), false)
        .await?;
    Ok(Json(n.into()))
}

async fn delete_note(
    State(state): State<AppState>,
    RequirePermission(session, ..): RequirePermission<SiteDiaryWrite>,
    Path((id, note_id)): Path<(i32, i32)>,
) -> Result<StatusCode, ApiError> {
    state
        .diary
        .delete_note(&session, DiaryEntryId(id), DiaryNoteId(note_id))
        .await?;
    Ok(StatusCode::NO_CONTENT)
}

async fn comments(
    State(state): State<AppState>,
    RequirePermission(session, ..): RequirePermission<SiteDiaryRead>,
    Path((id, note_id)): Path<(i32, i32)>,
) -> Result<Json<Vec<DiaryCommentDto>>, ApiError> {
    let c = state
        .diary
        .comments(&session, DiaryEntryId(id), DiaryNoteId(note_id))
        .await?;
    Ok(Json(c.into_iter().map(Into::into).collect()))
}

async fn add_comment(
    State(state): State<AppState>,
    RequirePermission(session, ..): RequirePermission<SiteDiaryWrite>,
    Path((id, note_id)): Path<(i32, i32)>,
    Json(req): Json<CommentRequest>,
) -> Result<(StatusCode, Json<DiaryCommentDto>), ApiError> {
    let c = state
        .diary
        .add_comment(
            &session,
            DiaryEntryId(id),
            DiaryNoteId(note_id),
            req.content,
        )
        .await?;
    Ok((StatusCode::CREATED, Json(c.into())))
}

/// Approved ETOs raised against a job.
///
/// Lives under `/site-diary` rather than `/forms` because an ETO *is* a diary
/// note -- the form only created it. The number is parsed back out of the note
/// text, which is the only place it was ever stored.
async fn job_etos(
    State(state): State<AppState>,
    RequirePermission(session, ..): RequirePermission<SiteDiaryRead>,
    Path(job_id): Path<i32>,
) -> Result<Json<Vec<crate::dto::EtoListItemDto>>, ApiError> {
    let etos = state
        .forms
        .etos_for_job(&session, pmk_domain::ids::JobId(job_id))
        .await?;
    Ok(Json(etos.into_iter().map(Into::into).collect()))
}

/// The structured weather reading recorded against an entry.
///
/// Null rather than 404 when none was taken: an entry without a reading is
/// ordinary, and the UI renders an empty panel rather than an error.
async fn weather_snapshot(
    State(state): State<AppState>,
    RequirePermission(session, ..): RequirePermission<SiteDiaryRead>,
    Path(id): Path<i32>,
) -> Result<Json<Option<crate::dto::WeatherSnapshotDto>>, ApiError> {
    let snapshot = state
        .diary
        .weather_snapshot(&session, DiaryEntryId(id))
        .await?;
    Ok(Json(snapshot.map(Into::into)))
}

/// Emails an entry to colleagues on the job.
///
/// Recipients are restricted to active users who can see that job, so this
/// cannot be used to relay mail to arbitrary addresses, and the ETO internal
/// reason is stripped from every note on the way out.
async fn email_entry(
    State(state): State<AppState>,
    RequirePermission(session, ..): RequirePermission<SiteDiaryRead>,
    Path(id): Path<i32>,
    Json(req): Json<crate::dto::EmailEntryRequest>,
) -> Result<Json<crate::dto::EmailSentDto>, ApiError> {
    let sent = state
        .diary
        .email_entry(
            &session,
            DiaryEntryId(id),
            &pmk_app::diary::DiaryEmailRequest {
                to: req.to,
                subject: req.subject,
                custom_message: req.custom_message,
            },
        )
        .await?;
    Ok(Json(crate::dto::EmailSentDto::to(&sent)))
}
