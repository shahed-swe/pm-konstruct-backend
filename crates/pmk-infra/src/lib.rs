// unwrap/expect are denied in production code by the workspace lints; tests may
// use them so a failure points at the assertion rather than an error path.
#![cfg_attr(test, allow(clippy::unwrap_used, clippy::expect_used))]

//! Infrastructure adapters. The only crate that talks to the outside world.

pub mod db;
