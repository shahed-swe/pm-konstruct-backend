//! Liveness and readiness.
//!
//! Split deliberately. The legacy API served one probe that reported schema
//! readiness, and gated data routes behind it; an autoscaler could not tell
//! "starting" from "broken". `/healthz` answers "is the process alive",
//! `/readyz` answers "can it serve traffic".

use axum::extract::State;
use axum::http::StatusCode;
use axum::routing::get;
use axum::{Json, Router};

use crate::dto::{HealthResponse, ReadyResponse};
use crate::state::AppState;

pub fn router() -> Router<AppState> {
    Router::new()
        .route("/healthz", get(healthz))
        .route("/readyz", get(readyz))
        // The legacy API answered on `/api` itself; kept for compatibility.
        .route("/", get(healthz))
}

async fn healthz(State(state): State<AppState>) -> Json<HealthResponse> {
    Json(HealthResponse { status: "ok", ready: state.is_ready() })
}

async fn readyz(State(state): State<AppState>) -> (StatusCode, Json<ReadyResponse>) {
    let database = sqlx::query("SELECT 1").execute(&state.pool).await.is_ok();
    let migrations = state.is_ready();
    let ok = database && migrations;
    (
        if ok { StatusCode::OK } else { StatusCode::SERVICE_UNAVAILABLE },
        Json(ReadyResponse {
            status: if ok { "ready" } else { "starting" },
            database,
            migrations,
        }),
    )
}
