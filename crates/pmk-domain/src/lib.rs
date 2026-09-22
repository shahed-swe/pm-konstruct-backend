//! Pure domain logic for PM Konstruct. No I/O, no async, no database.
//!
//! Everything here is a deterministic function of its inputs, which is what
//! makes the behaviour frozen in `docs/contract/domain-rules.md` testable in
//! isolation. Time enters through a caller-supplied value, never `now()`.

pub mod access;
pub mod call_forward;
pub mod error;
pub mod ids;
pub mod job;
pub mod tenant;

pub use error::{DomainError, DomainResult};
pub use ids::*;
pub use tenant::CompanyId;
