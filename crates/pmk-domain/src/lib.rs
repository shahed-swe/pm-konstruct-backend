// unwrap/expect are denied in production code by the workspace lints; tests may
// use them so a failure points at the assertion rather than an error path.
#![cfg_attr(test, allow(clippy::unwrap_used, clippy::expect_used, clippy::panic))]

//! Pure domain logic for PM Konstruct. No I/O, no async, no database.
//!
//! Everything here is a deterministic function of its inputs, which is what
//! makes the behaviour frozen in `docs/contract/domain-rules.md` testable in
//! isolation. Time enters through a caller-supplied value, never `now()`.

pub mod access;
pub mod call_forward;
pub mod error;
pub mod identity;
pub mod ids;
pub mod job;
pub mod tenant;

pub use error::{DomainError, DomainResult};
pub use identity::{HashKind, User};
pub use ids::*;
pub use tenant::CompanyId;
