// unwrap/expect are denied in production code by the workspace lints; tests may
// use them so a failure points at the assertion rather than an error path.
#![cfg_attr(test, allow(clippy::unwrap_used, clippy::expect_used, clippy::panic))]

//! Ports: the traits the domain and application layers depend on.
//!
//! A port exists only where there is a real external boundary or a genuine
//! second implementation (a test double counts). No `UserServiceInterface` for
//! its own sake — see the Phase 4 risk note about hexagonal ceremony.

pub mod clock;
pub mod error;
pub mod events;
pub mod repository;
pub mod storage;

pub use clock::{Clock, FixedClock, SystemClock};
pub use error::{PortError, PortResult};
pub use events::{Audience, BroadcastEvent, EventBus, MAX_EVENT_BYTES};
pub use storage::{ObjectHead, ObjectStore, PresignedUrl};
