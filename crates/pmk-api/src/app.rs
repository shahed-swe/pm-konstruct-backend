//! Router assembly and middleware stack.

use std::time::Duration;

use axum::http::{header, HeaderValue, Method};
use axum::Router;
use tower_http::compression::CompressionLayer;
use tower_http::cors::CorsLayer;
use tower_http::limit::RequestBodyLimitLayer;
use tower_http::set_header::SetResponseHeaderLayer;
use tower_http::timeout::TimeoutLayer;
use tower_http::trace::TraceLayer;

use crate::routes;
use crate::state::AppState;

/// Builds the `/api` router with the full middleware stack.
pub fn build_router(state: AppState) -> Router {
    let cfg = state.config.clone();

    // Explicit origin allowlist. The legacy API called `cors()` with no
    // arguments, i.e. a wildcard origin on an API that carries credentials.
    let origins: Vec<HeaderValue> = cfg
        .server
        .cors_allowed_origins
        .iter()
        .filter_map(|o| HeaderValue::from_str(o).ok())
        .collect();

    let cors = CorsLayer::new()
        .allow_origin(origins)
        .allow_credentials(true)
        .allow_methods([
            Method::GET,
            Method::POST,
            Method::PUT,
            Method::PATCH,
            Method::DELETE,
            Method::OPTIONS,
        ])
        .allow_headers([header::AUTHORIZATION, header::CONTENT_TYPE, header::ACCEPT])
        .max_age(Duration::from_secs(600));

    let api = Router::new()
        .merge(routes::health::router())
        .nest("/auth", routes::auth::router())
        .nest("/jobs", routes::jobs::router())
        .nest("/site-diary", routes::diary::router())
        .nest("/call-forward", routes::call_forward::router())
        .nest("/tasks", routes::jobs::tasks_router())
        .nest("/media", routes::media::router())
        .nest("/ai-assistant", routes::ai_assistant::router())
        .nest("/scheduler", routes::scheduler::router())
        .nest("/forms", routes::forms::router())
        .nest("/dashboard", routes::dashboard::router())
        .nest("/progress", routes::progress::router())
        // Nested under their parent so the parent id is in scope; media is
        // always reached through the entry or job that owns it.
        .nest("/site-diary/{id}/media", routes::media::diary_router())
        .nest("/jobs/{id}/media", routes::media::job_router());

    Router::new()
        .nest("/api", api)
        .layer(cors)
        .layer(CompressionLayer::new())
        .layer(TimeoutLayer::with_status_code(
            axum::http::StatusCode::REQUEST_TIMEOUT,
            Duration::from_secs(cfg.server.request_timeout_secs),
        ))
        .layer(RequestBodyLimitLayer::new(cfg.server.max_body_bytes))
        .layer(TraceLayer::new_for_http())
        // Security headers. CSP is tighter than the legacy policy, which
        // allowed 'unsafe-inline' styles.
        .layer(SetResponseHeaderLayer::overriding(
            header::X_CONTENT_TYPE_OPTIONS,
            HeaderValue::from_static("nosniff"),
        ))
        .layer(SetResponseHeaderLayer::overriding(
            header::X_FRAME_OPTIONS,
            HeaderValue::from_static("DENY"),
        ))
        .layer(SetResponseHeaderLayer::overriding(
            header::REFERRER_POLICY,
            HeaderValue::from_static("strict-origin-when-cross-origin"),
        ))
        .layer(SetResponseHeaderLayer::overriding(
            header::STRICT_TRANSPORT_SECURITY,
            HeaderValue::from_static("max-age=31536000; includeSubDomains"),
        ))
        .with_state(state)
}
