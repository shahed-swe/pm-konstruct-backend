//! Notification routes.
//!
//! Everything here is about the caller's own notifications, so none of it is
//! permission-gated beyond being entitled: a supervisor and a manager see
//! their own bell and nobody else's.

use axum::extract::{Path, State};
use axum::routing::{get, patch, post};
use axum::{Json, Router};
use pmk_domain::ids::NotificationId;

use crate::dto::{
    NotificationFeedDto, NotificationPrefsDto, OkDto, SubscribeRequest, UnsubscribeRequest,
    UpdatePrefsRequest, VapidKeyDto,
};
use crate::error::ApiError;
use crate::extract::Entitled;
use crate::state::AppState;

pub fn router() -> Router<AppState> {
    Router::new()
        .route("/", get(list))
        .route("/read-all", patch(read_all))
        .route("/{id}/read", patch(read_one))
        .route("/vapid-public-key", get(vapid_key))
        .route("/push-subscribe", post(subscribe).delete(unsubscribe))
        .route("/prefs", get(prefs).put(set_prefs))
}

async fn list(
    State(state): State<AppState>,
    Entitled(session): Entitled,
) -> Result<Json<NotificationFeedDto>, ApiError> {
    Ok(Json(state.notifications.feed(&session).await?.into()))
}

async fn read_all(
    State(state): State<AppState>,
    Entitled(session): Entitled,
) -> Result<Json<OkDto>, ApiError> {
    state.notifications.mark_all_read(&session).await?;
    Ok(Json(OkDto::yes()))
}

async fn read_one(
    State(state): State<AppState>,
    Entitled(session): Entitled,
    Path(id): Path<i32>,
) -> Result<Json<OkDto>, ApiError> {
    state
        .notifications
        .mark_read(&session, NotificationId(id))
        .await?;
    Ok(Json(OkDto::yes()))
}

/// The browser needs this to create a subscription.
///
/// Unauthenticated in the legacy and here: it is a public key, and the private
/// half never leaves the server.
async fn vapid_key(State(state): State<AppState>) -> Json<VapidKeyDto> {
    Json(VapidKeyDto {
        public_key: state.notifications.vapid_public_key().to_string(),
    })
}

async fn subscribe(
    State(state): State<AppState>,
    Entitled(session): Entitled,
    Json(req): Json<SubscribeRequest>,
) -> Result<Json<OkDto>, ApiError> {
    state.notifications.subscribe(&session, &req.into()).await?;
    Ok(Json(OkDto::yes()))
}

async fn unsubscribe(
    State(state): State<AppState>,
    Entitled(session): Entitled,
    Json(req): Json<UnsubscribeRequest>,
) -> Result<Json<OkDto>, ApiError> {
    state
        .notifications
        .unsubscribe(&session, &req.endpoint)
        .await?;
    Ok(Json(OkDto::yes()))
}

async fn prefs(
    State(state): State<AppState>,
    Entitled(session): Entitled,
) -> Result<Json<NotificationPrefsDto>, ApiError> {
    Ok(Json(state.notifications.prefs(&session).await?.into()))
}

/// Absent fields keep their current value, so the UI can toggle one switch
/// without first reading the other.
async fn set_prefs(
    State(state): State<AppState>,
    Entitled(session): Entitled,
    Json(req): Json<UpdatePrefsRequest>,
) -> Result<Json<NotificationPrefsDto>, ApiError> {
    let current = state.notifications.prefs(&session).await?;
    let updated = state
        .notifications
        .set_prefs(&session, req.apply(current))
        .await?;
    Ok(Json(updated.into()))
}
