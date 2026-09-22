//! Forms routes: extras-to-order and the property inspection draft.
//!
//! Every inspection write carries the revision the client last read, as the
//! `x-inspection-revision` header for the photo routes and as a body field for
//! the draft save. A mismatch is a 409, never a silent overwrite -- two
//! supervisors walking the same site must not clobber each other's checklist.

use axum::extract::{Path, State};
use axum::http::{HeaderMap, StatusCode};
use axum::routing::{delete, post, put};
use axum::{Json, Router};
use pmk_domain::ids::{InspectionFormId, JobId};
use pmk_domain::media::UploadRequest;

use crate::dto::{
    ConfirmInspectionPhotos, DraftRequest, EtoRaisedDto, EtoRequest, InspectionDraftRequest,
    InspectionFormDto, PrepareUploadRequest, PreparedInspectionUploadDto,
};
use crate::error::ApiError;
use crate::extract::{RequirePermission, SiteDiaryWrite};
use crate::state::AppState;

/// Mounted at `/forms`.
pub fn router() -> Router<AppState> {
    Router::new()
        .route("/eto", post(raise_eto))
        .route("/email", post(email_form))
        .route("/property-inspection/draft", post(get_or_create_draft))
        .route("/property-inspection/{form_id}", put(save_draft))
        .route(
            "/property-inspection/{form_id}/items/{client_key}/photos/prepare",
            post(prepare_photos),
        )
        .route(
            "/property-inspection/{form_id}/items/{client_key}/photos",
            post(attach_photos),
        )
        .route(
            "/property-inspection/{form_id}/photos/{photo_id}",
            delete(delete_photo),
        )
}

// ── extras to order ─────────────────────────────────────────────────────────

async fn raise_eto(
    State(state): State<AppState>,
    RequirePermission(session, ..): RequirePermission<SiteDiaryWrite>,
    Json(req): Json<EtoRequest>,
) -> Result<(StatusCode, Json<EtoRaisedDto>), ApiError> {
    let raised = state.forms.raise_eto(&session, &req.into()).await?;
    Ok((StatusCode::CREATED, Json(raised.into())))
}

// ── property inspection ─────────────────────────────────────────────────────

async fn get_or_create_draft(
    State(state): State<AppState>,
    RequirePermission(session, ..): RequirePermission<SiteDiaryWrite>,
    Json(req): Json<DraftRequest>,
) -> Result<Json<InspectionFormDto>, ApiError> {
    // 200 rather than 201: the caller cannot tell whether their draft already
    // existed, and the legacy handler returned 200 either way.
    let form = state
        .forms
        .get_or_create_draft(&session, JobId(req.job_id))
        .await?;
    Ok(Json(form.into()))
}

async fn save_draft(
    State(state): State<AppState>,
    RequirePermission(session, ..): RequirePermission<SiteDiaryWrite>,
    Path(form_id): Path<i32>,
    Json(req): Json<InspectionDraftRequest>,
) -> Result<Json<InspectionFormDto>, ApiError> {
    let form = state
        .forms
        .save_draft(&session, InspectionFormId(form_id), &req.into())
        .await?;
    Ok(Json(form.into()))
}

async fn prepare_photos(
    State(state): State<AppState>,
    RequirePermission(session, ..): RequirePermission<SiteDiaryWrite>,
    Path((form_id, client_key)): Path<(i32, String)>,
    headers: HeaderMap,
    Json(req): Json<PrepareUploadRequest>,
) -> Result<Json<Vec<PreparedInspectionUploadDto>>, ApiError> {
    let revision = revision_header(&headers)?;
    let files: Vec<UploadRequest> = req.files.into_iter().map(Into::into).collect();
    let prepared = state
        .forms
        .prepare_photos(
            &session,
            InspectionFormId(form_id),
            &client_key,
            revision,
            files,
        )
        .await?;
    Ok(Json(prepared.into_iter().map(Into::into).collect()))
}

async fn attach_photos(
    State(state): State<AppState>,
    RequirePermission(session, ..): RequirePermission<SiteDiaryWrite>,
    Path((form_id, client_key)): Path<(i32, String)>,
    headers: HeaderMap,
    Json(req): Json<ConfirmInspectionPhotos>,
) -> Result<(StatusCode, Json<InspectionFormDto>), ApiError> {
    let revision = revision_header(&headers)?;
    let uploads: Vec<_> = req.files.into_iter().map(Into::into).collect();
    let form = state
        .forms
        .attach_photos(
            &session,
            InspectionFormId(form_id),
            &client_key,
            revision,
            &uploads,
        )
        .await?;
    Ok((StatusCode::CREATED, Json(form.into())))
}

async fn delete_photo(
    State(state): State<AppState>,
    RequirePermission(session, ..): RequirePermission<SiteDiaryWrite>,
    Path((form_id, photo_id)): Path<(i32, i32)>,
    headers: HeaderMap,
) -> Result<Json<InspectionFormDto>, ApiError> {
    let revision = revision_header(&headers)?;
    let form = state
        .forms
        .delete_photo(&session, InspectionFormId(form_id), revision, photo_id)
        .await?;
    Ok(Json(form.into()))
}

/// Reads `x-inspection-revision`.
///
/// Required, and required to be a non-negative integer: a missing header
/// cannot be defaulted to 0 without turning every stale client into a silent
/// overwrite of whoever saved first.
fn revision_header(headers: &HeaderMap) -> Result<i32, ApiError> {
    let invalid = || {
        ApiError::new(
            StatusCode::BAD_REQUEST,
            "A valid inspection revision is required",
        )
        .with_field("x-inspection-revision")
    };
    let raw = headers
        .get("x-inspection-revision")
        .ok_or_else(invalid)?
        .to_str()
        .map_err(|_| invalid())?;
    let revision: i32 = raw.trim().parse().map_err(|_| invalid())?;
    if revision < 0 {
        return Err(invalid());
    }
    Ok(revision)
}

/// Emails a form to colleagues on the job.
async fn email_form(
    State(state): State<AppState>,
    RequirePermission(session, ..): RequirePermission<SiteDiaryWrite>,
    Json(req): Json<crate::dto::EmailFormRequest>,
) -> Result<Json<crate::dto::EmailSentDto>, ApiError> {
    let sent = state
        .forms
        .email_form(
            &session,
            pmk_domain::ids::JobId(req.job_id),
            &req.to,
            &req.subject,
            &req.body,
        )
        .await?;
    Ok(Json(crate::dto::EmailSentDto::to(&sent)))
}

#[cfg(test)]
mod tests {
    use super::*;
    use axum::http::HeaderValue;

    fn headers(value: &str) -> HeaderMap {
        let mut h = HeaderMap::new();
        h.insert(
            "x-inspection-revision",
            HeaderValue::from_str(value).unwrap(),
        );
        h
    }

    #[test]
    fn a_revision_header_is_read_as_an_integer() {
        assert_eq!(revision_header(&headers("0")).unwrap(), 0);
        assert_eq!(revision_header(&headers("12")).unwrap(), 12);
        assert_eq!(revision_header(&headers(" 12 ")).unwrap(), 12);
    }

    #[test]
    fn a_missing_revision_is_a_400_not_a_default() {
        let e = revision_header(&HeaderMap::new()).unwrap_err();
        assert_eq!(e.status, StatusCode::BAD_REQUEST);
    }

    #[test]
    fn a_negative_or_unparseable_revision_is_rejected() {
        assert!(revision_header(&headers("-1")).is_err());
        assert!(revision_header(&headers("abc")).is_err());
        assert!(revision_header(&headers("")).is_err());
        assert!(revision_header(&headers("1.5")).is_err());
    }
}
