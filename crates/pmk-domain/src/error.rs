//! One error taxonomy for the whole domain.
//!
//! `pmk-api` maps these to HTTP in exactly one place, so the byte-identical
//! error bodies the legacy frontend depends on (domain-rules R6) are produced
//! from a single table rather than scattered `res.status(...)` calls.

use thiserror::Error;

pub type DomainResult<T> = Result<T, DomainError>;

#[derive(Debug, Error, PartialEq, Eq)]
pub enum DomainError {
    /// The caller may not see this resource. Prefer `NotFound` when revealing
    /// existence would itself leak across tenants.
    #[error("forbidden: {0}")]
    Forbidden(&'static str),

    #[error("{entity} not found")]
    NotFound { entity: &'static str },

    /// A single field failed validation. `field` matches the API field name so
    /// the frontend can attach the message to an input.
    #[error("invalid {field}: {reason}")]
    Invalid { field: &'static str, reason: String },

    /// A business rule was violated -- not a field problem.
    #[error("{0}")]
    RuleViolation(String),

    /// Uniqueness or overlap conflict. Maps to HTTP 409.
    #[error("conflict: {0}")]
    Conflict(String),

    /// Subscription gate. Maps to HTTP 402 with code `BILLING_REQUIRED`
    /// (domain-rules R6) -- the message text is part of the contract.
    #[error("{0}")]
    BillingRequired(&'static str),
}

impl DomainError {
    pub fn invalid(field: &'static str, reason: impl Into<String>) -> Self {
        Self::Invalid {
            field,
            reason: reason.into(),
        }
    }
    #[must_use]
    pub const fn not_found(entity: &'static str) -> Self {
        Self::NotFound { entity }
    }
}
