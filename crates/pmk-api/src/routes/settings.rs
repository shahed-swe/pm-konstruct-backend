//! Company settings: branding and outbound email.
//!
//! Every write is manager-only. `GET /branding` is the one exception and is
//! deliberately open: the login page needs a logo before anyone has signed in,
//! and an unauthenticated caller gets the oldest company's branding rather
//! than any particular one.

use axum::extract::{Path, State};
use axum::http::StatusCode;
use axum::response::Redirect;
use axum::routing::{get, post};
use axum::{Json, Router};
use pmk_domain::media::UploadRequest;

use crate::dto::{
    BrandingDto, BrandingRequest, ConfirmBrandingRequest, EmailSettingsDto, EmailSettingsRequest,
    EmailStatusDto, OkDto, PrepareBrandingRequest, PreparedBrandingUploadDto, TestEmailRequest,
};
use crate::error::ApiError;
use crate::extract::{ManagerOnly, MaybeAuth, RequireRole};
use crate::state::AppState;

pub fn router() -> Router<AppState> {
    Router::new()
        .route("/branding", get(branding).put(set_branding))
        .route(
            "/branding/logo",
            get(logo).post(confirm_logo).delete(clear_logo),
        )
        .route("/branding/logo/prepare", post(prepare_logo))
        .route(
            "/branding/banner",
            get(banner).post(confirm_banner).delete(clear_banner),
        )
        .route("/branding/banner/prepare", post(prepare_banner))
        // The legacy served images by filename. The object key is not public,
        // so the filename is ignored and the company's current image is
        // returned -- which is what the caller wanted, and cannot be used to
        // reach another tenant's.
        .route("/branding/logo/{filename}", get(logo_by_name))
        .route("/email", get(email_settings).put(set_email_settings))
        .route("/email/status", get(email_status))
        .route("/email/test", post(test_email))
}

/// The branding for the caller's company, or the default one when there is no
/// session. The login page depends on this being reachable unauthenticated.
async fn branding(
    State(state): State<AppState>,
    MaybeAuth(session): MaybeAuth,
) -> Result<Json<BrandingDto>, ApiError> {
    let b = match session {
        Some(ref s) => state.settings.branding(s).await?,
        None => state.settings.default_branding().await?,
    };
    Ok(Json(b.into()))
}

async fn set_branding(
    State(state): State<AppState>,
    RequireRole(session): ManagerOnly,
    Json(req): Json<BrandingRequest>,
) -> Result<Json<BrandingDto>, ApiError> {
    let b = state
        .settings
        .set_branding(&session, &req.into_input()?)
        .await?;
    Ok(Json(b.into()))
}

// ── images ──────────────────────────────────────────────────────────────────

async fn prepare_logo(
    State(state): State<AppState>,
    RequireRole(session): ManagerOnly,
    Json(req): Json<PrepareBrandingRequest>,
) -> Result<Json<PreparedBrandingUploadDto>, ApiError> {
    prepare(state, session, false, req).await
}

async fn prepare_banner(
    State(state): State<AppState>,
    RequireRole(session): ManagerOnly,
    Json(req): Json<PrepareBrandingRequest>,
) -> Result<Json<PreparedBrandingUploadDto>, ApiError> {
    prepare(state, session, true, req).await
}

async fn prepare(
    state: AppState,
    session: pmk_app::identity::SessionUser,
    banner: bool,
    req: PrepareBrandingRequest,
) -> Result<Json<PreparedBrandingUploadDto>, ApiError> {
    let file = UploadRequest {
        original_name: req.file_name,
        mime: req.mime_type,
        size_bytes: req.size_bytes,
    };
    let p = state
        .settings
        .prepare_image(&session, banner, &file)
        .await?;
    Ok(Json(p.into()))
}

async fn confirm_logo(
    State(state): State<AppState>,
    RequireRole(session): ManagerOnly,
    Json(req): Json<ConfirmBrandingRequest>,
) -> Result<Json<BrandingDto>, ApiError> {
    confirm(state, session, false, req).await
}

async fn confirm_banner(
    State(state): State<AppState>,
    RequireRole(session): ManagerOnly,
    Json(req): Json<ConfirmBrandingRequest>,
) -> Result<Json<BrandingDto>, ApiError> {
    confirm(state, session, true, req).await
}

async fn confirm(
    state: AppState,
    session: pmk_app::identity::SessionUser,
    banner: bool,
    req: ConfirmBrandingRequest,
) -> Result<Json<BrandingDto>, ApiError> {
    let b = state
        .settings
        .confirm_image(
            &session,
            banner,
            &req.stored_name,
            &req.original_name,
            &req.mime_type,
        )
        .await?;
    Ok(Json(b.into()))
}

async fn clear_logo(
    State(state): State<AppState>,
    RequireRole(session): ManagerOnly,
) -> Result<Json<BrandingDto>, ApiError> {
    Ok(Json(
        state.settings.clear_image(&session, false).await?.into(),
    ))
}

async fn clear_banner(
    State(state): State<AppState>,
    RequireRole(session): ManagerOnly,
) -> Result<Json<BrandingDto>, ApiError> {
    Ok(Json(
        state.settings.clear_image(&session, true).await?.into(),
    ))
}

/// Redirects to a short-lived signed URL for the image.
///
/// Open to anyone, like the branding itself, because the login page renders
/// the logo. The signature is what protects the object, not the route.
async fn logo(
    State(state): State<AppState>,
    MaybeAuth(session): MaybeAuth,
) -> Result<Redirect, ApiError> {
    serve_image(state, session, false).await
}

async fn banner(
    State(state): State<AppState>,
    MaybeAuth(session): MaybeAuth,
) -> Result<Redirect, ApiError> {
    serve_image(state, session, true).await
}

/// The legacy filename route. The name is ignored -- see the router.
async fn logo_by_name(
    State(state): State<AppState>,
    MaybeAuth(session): MaybeAuth,
    Path(_filename): Path<String>,
) -> Result<Redirect, ApiError> {
    serve_image(state, session, false).await
}

async fn serve_image(
    state: AppState,
    session: Option<pmk_app::identity::SessionUser>,
    banner: bool,
) -> Result<Redirect, ApiError> {
    let url = match session {
        Some(ref s) => state.settings.image_url(s, banner).await?,
        // No session: the login page, which gets the default company's.
        None => state.settings.default_image_url(banner).await?,
    };
    Ok(Redirect::temporary(&url))
}

// ── email ───────────────────────────────────────────────────────────────────

async fn email_settings(
    State(state): State<AppState>,
    RequireRole(session): ManagerOnly,
) -> Result<Json<EmailSettingsDto>, ApiError> {
    Ok(Json(state.settings.email_settings(&session).await?.into()))
}

async fn set_email_settings(
    State(state): State<AppState>,
    RequireRole(session): ManagerOnly,
    Json(req): Json<EmailSettingsRequest>,
) -> Result<Json<EmailSettingsDto>, ApiError> {
    let e = state
        .settings
        .set_email_settings(&session, &req.into())
        .await?;
    Ok(Json(e.into()))
}

/// What is configured, without disclosing any of it.
async fn email_status(
    State(state): State<AppState>,
    RequireRole(session): ManagerOnly,
) -> Result<Json<EmailStatusDto>, ApiError> {
    Ok(Json(state.settings.email_status(&session).await?.into()))
}

async fn test_email(
    State(state): State<AppState>,
    RequireRole(session): ManagerOnly,
    Json(req): Json<TestEmailRequest>,
) -> Result<(StatusCode, Json<OkDto>), ApiError> {
    state.settings.send_test_email(&session, &req.to).await?;
    Ok((StatusCode::OK, Json(OkDto::yes())))
}
