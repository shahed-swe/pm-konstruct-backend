// unwrap/expect are denied in production code by the workspace lints; tests may
// use them so a failure points at the assertion rather than an error path.
#![cfg_attr(test, allow(clippy::unwrap_used, clippy::expect_used))]

//! HTTP layer: axum routers, extractors, DTOs and the single error mapping.

pub mod app;
pub mod dto;
pub mod error;
pub mod extract;
pub mod routes;
pub mod state;

pub use app::build_router;
pub use state::AppState;
