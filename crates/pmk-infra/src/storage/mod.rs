//! Object storage adapters.

pub mod s3;

pub use s3::{object_key, S3ObjectStore};
