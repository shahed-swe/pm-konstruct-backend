// unwrap/expect are denied in production code by the workspace lints; tests may
// use them so a failure points at the assertion rather than an error path.
#![cfg_attr(test, allow(clippy::unwrap_used, clippy::expect_used, clippy::panic))]

//! Application services: orchestration, transactions, and the use cases the
//! HTTP layer calls. Business rules live in `pmk-domain`; I/O lives behind
//! `pmk-ports`.

pub mod call_forward;
pub mod diary;
pub mod error;
pub mod forms;
pub mod identity;
pub mod jobs;
pub mod media;
pub mod scheduler;

pub use error::{AppError, AppResult};
