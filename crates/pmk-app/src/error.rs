//! Application error type.
//!
//! `pmk-api` maps this to HTTP in exactly one place, so the byte-identical
//! error bodies the legacy frontend depends on (domain-rules R6) come from a
//! single table rather than scattered status calls.

use pmk_domain::DomainError;
use pmk_ports::PortError;
use thiserror::Error;

pub type AppResult<T> = Result<T, AppError>;

#[derive(Debug, Error)]
pub enum AppError {
    #[error(transparent)]
    Domain(#[from] DomainError),

    /// Credentials did not match, or the account is inactive. Deliberately one
    /// variant with one message: distinguishing "no such user" from "wrong
    /// password" is a user-enumeration oracle.
    #[error("Invalid email or password")]
    InvalidCredentials,

    #[error("Authentication required")]
    Unauthenticated,

    #[error("Invalid or expired token")]
    InvalidToken,

    #[error("Too many attempts. Please try again later.")]
    RateLimited { retry_after_secs: u64 },

    /// The caller exceeded a per-feature quota. Distinct from `RateLimited`,
    /// which is the login throttle and carries its own fixed message.
    #[error("{0}")]
    TooManyRequests(String),

    /// A feature this deployment has not been configured for. Not an error in
    /// the code: the AI assistant and the weather lookup both ship without
    /// credentials and the UI simply does not offer them.
    #[error("{0}")]
    FeatureUnavailable(String),

    #[error(transparent)]
    Port(#[from] PortError),

    #[error("internal error: {0}")]
    Internal(String),
}

impl AppError {
    pub fn internal(e: impl std::fmt::Display) -> Self {
        Self::Internal(e.to_string())
    }
}
