// unwrap/expect are denied in production code by the workspace lints; tests may
// use them so a failure points at the assertion rather than an error path.
#![cfg_attr(test, allow(clippy::unwrap_used, clippy::expect_used, clippy::panic))]

//! Infrastructure adapters. The only crate that talks to the outside world.

pub mod config;
pub mod db;
pub mod repo;
pub mod telemetry;

pub use config::Config;
