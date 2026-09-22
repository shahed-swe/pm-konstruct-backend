//! Request and response shapes.
//!
//! Field names use the legacy camelCase so the existing frontend and the
//! golden fixtures match without a translation layer.

pub mod auth;
pub mod call_forward;
pub mod dashboard;
pub mod diary;
pub mod forms;
pub mod job;
pub mod media;
pub mod progress;
pub mod reports;
pub mod scheduler;

pub use auth::*;
pub use call_forward::*;
pub use dashboard::*;
pub use diary::*;
pub use forms::*;
pub use job::*;
pub use media::*;
pub use progress::*;
pub use reports::*;
pub use scheduler::*;
