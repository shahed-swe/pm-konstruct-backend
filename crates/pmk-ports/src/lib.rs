//! Ports: the traits the domain and application layers depend on.
//!
//! A port exists only where there is a real external boundary or a genuine
//! second implementation (a test double counts). No `UserServiceInterface` for
//! its own sake — see the Phase 4 risk note about hexagonal ceremony.

pub mod clock;
pub mod error;

pub use clock::{Clock, FixedClock, SystemClock};
pub use error::{PortError, PortResult};
