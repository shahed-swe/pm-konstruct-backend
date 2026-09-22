//! AI assistant — not implemented in v1.
//!
//! The legacy assistant had no credential to migrate: the API key was literally
//! `_DUMMY_API_KEY_` and every request went to
//! `http://localhost:1106/modelfarm/openai`, Replit's ModelFarm sidecar.
//! Running it anywhere else needs a new paid provider account, and production
//! usage was five rows in `ai_usage_daily`.
//!
//! Dropped from v1 by decision, not by oversight — see
//! docs/adr/0002-scope-decisions.md. The route stays so clients get a clear
//! answer rather than a 404 they might mistake for a routing bug, and
//! `ai_usage_daily` is preserved so the quota history survives and the feature
//! can return without a migration.

use axum::http::StatusCode;
use axum::routing::post;
use axum::{Json, Router};
use serde::Serialize;

use crate::extract::Entitled;
use crate::state::AppState;

#[derive(Debug, Serialize)]
#[serde(rename_all = "camelCase")]
struct NotImplemented {
    error: &'static str,
    code: &'static str,
}

pub fn router() -> Router<AppState> {
    Router::new().route("/chat", post(chat))
}

/// Authenticated deliberately: an unauthenticated caller should still get 401
/// rather than learning which features exist.
async fn chat(Entitled(_session): Entitled) -> (StatusCode, Json<NotImplemented>) {
    (
        StatusCode::NOT_IMPLEMENTED,
        Json(NotImplemented {
            error: "The AI assistant is not available in this version.",
            code: "FEATURE_UNAVAILABLE",
        }),
    )
}
