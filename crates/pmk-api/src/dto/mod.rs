//! Request and response shapes.
//!
//! Field names use the legacy camelCase so the existing frontend and the
//! golden fixtures match without a translation layer.

pub mod auth;
pub mod call_forward;
pub mod diary;
pub mod job;

pub use auth::*;
pub use call_forward::*;
pub use diary::*;
pub use job::*;
