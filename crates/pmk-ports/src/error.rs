use thiserror::Error;

pub type PortResult<T> = Result<T, PortError>;

/// Failures from the outside world. Adapters map provider-specific errors into
/// these so the application layer never matches on a vendor type.
#[derive(Debug, Error)]
pub enum PortError {
    #[error("not found")]
    NotFound,

    /// Uniqueness, overlap or exclusion violation. Carries the constraint name
    /// when the driver reports one, so the API can map it to a field.
    #[error("conflict{}", .constraint.as_ref().map(|c| format!(" ({c})")).unwrap_or_default())]
    Conflict { constraint: Option<String> },

    #[error("storage: {0}")]
    Storage(String),

    #[error("upstream {service} unavailable: {detail}")]
    Unavailable { service: &'static str, detail: String },

    #[error("upstream {service} rejected the request: {detail}")]
    Rejected { service: &'static str, detail: String },

    #[error(transparent)]
    Other(#[from] anyhow::Error),
}
