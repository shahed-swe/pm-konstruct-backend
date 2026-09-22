//! The one place application errors become HTTP responses.
//!
//! Error bodies are part of the contract: the legacy frontend reads
//! `{"error": "..."}` and keys the billing redirect off
//! `{"code":"BILLING_REQUIRED"}` (domain-rules R6). Both shapes are reproduced
//! byte for byte, which is why this mapping lives in exactly one function
//! rather than being scattered across handlers.

use axum::http::StatusCode;
use axum::response::{IntoResponse, Response};
use axum::Json;
use pmk_app::AppError;
use pmk_domain::DomainError;
use pmk_ports::PortError;
use serde::Serialize;

#[derive(Debug, Serialize)]
pub struct ErrorBody {
    /// Human-readable message. The legacy frontend displays this directly.
    pub error: String,
    /// Machine-readable discriminator. Only set where the legacy API set one,
    /// so no client sees a field it did not see before.
    #[serde(skip_serializing_if = "Option::is_none")]
    pub code: Option<&'static str>,
    /// Field-level validation detail. New; additive, so it cannot break a
    /// client that ignores it.
    #[serde(skip_serializing_if = "Option::is_none")]
    pub field: Option<String>,
}

#[derive(Debug)]
pub struct ApiError {
    pub status: StatusCode,
    pub body: ErrorBody,
    /// Retry-After, for rate limiting.
    pub retry_after_secs: Option<u64>,
}

impl ApiError {
    pub fn new(status: StatusCode, message: impl Into<String>) -> Self {
        Self {
            status,
            body: ErrorBody {
                error: message.into(),
                code: None,
                field: None,
            },
            retry_after_secs: None,
        }
    }

    #[must_use]
    pub fn with_code(mut self, code: &'static str) -> Self {
        self.body.code = Some(code);
        self
    }

    #[must_use]
    pub fn with_field(mut self, field: impl Into<String>) -> Self {
        self.body.field = Some(field.into());
        self
    }

    // -- the exact legacy strings, preserved ---------------------------------

    /// `requireSubscriptionAccess` -- domain-rules R6.
    #[must_use]
    pub fn billing_required() -> Self {
        Self::new(
            StatusCode::PAYMENT_REQUIRED,
            "Complete billing setup or reactivate your subscription to access the application.",
        )
        .with_code("BILLING_REQUIRED")
    }

    /// `requireSubscriptionConfigurationAccess` -- same code, different message.
    #[must_use]
    pub fn billing_configuration_required() -> Self {
        Self::new(
            StatusCode::PAYMENT_REQUIRED,
            "Reactivate your subscription in Billing before changing company settings.",
        )
        .with_code("BILLING_REQUIRED")
    }

    #[must_use]
    pub fn unauthenticated() -> Self {
        Self::new(StatusCode::UNAUTHORIZED, "Authentication required")
    }

    #[must_use]
    pub fn invalid_token() -> Self {
        Self::new(StatusCode::UNAUTHORIZED, "Invalid or expired token")
    }

    #[must_use]
    pub fn forbidden() -> Self {
        Self::new(StatusCode::FORBIDDEN, "Insufficient permissions")
    }
}

impl IntoResponse for ApiError {
    fn into_response(self) -> Response {
        let mut res = (self.status, Json(&self.body)).into_response();
        if let Some(secs) = self.retry_after_secs {
            if let Ok(v) = secs.to_string().parse() {
                res.headers_mut().insert(axum::http::header::RETRY_AFTER, v);
            }
        }
        res
    }
}

impl From<DomainError> for ApiError {
    fn from(e: DomainError) -> Self {
        match e {
            // 404 rather than 403 where revealing existence would leak across
            // tenants.
            DomainError::NotFound { entity } => {
                ApiError::new(StatusCode::NOT_FOUND, format!("{entity} not found"))
            }
            DomainError::Forbidden(_) => ApiError::forbidden(),
            DomainError::Invalid { field, ref reason } => {
                ApiError::new(StatusCode::BAD_REQUEST, reason.clone()).with_field(field)
            }
            DomainError::RuleViolation(ref m) => {
                ApiError::new(StatusCode::UNPROCESSABLE_ENTITY, m.clone())
            }
            DomainError::Conflict(ref m) => ApiError::new(StatusCode::CONFLICT, m.clone()),
            DomainError::BillingRequired(_) => ApiError::billing_required(),
        }
    }
}

impl From<PortError> for ApiError {
    fn from(e: PortError) -> Self {
        match e {
            PortError::NotFound => ApiError::new(StatusCode::NOT_FOUND, "Not found"),
            PortError::Conflict { ref constraint } => ApiError::new(
                StatusCode::CONFLICT,
                constraint
                    .as_deref()
                    .map_or_else(|| "Conflict".to_string(), constraint_message),
            ),
            PortError::CheckViolation { ref constraint } => {
                let (message, field) = constraint
                    .as_deref()
                    .map_or_else(|| ("Invalid value".to_string(), None), check_message);
                let mut err = ApiError::new(StatusCode::BAD_REQUEST, message);
                if let Some(f) = field {
                    err = err.with_field(f);
                }
                err
            }
            PortError::Unavailable { service, .. } => ApiError::new(
                StatusCode::SERVICE_UNAVAILABLE,
                format!("{service} is temporarily unavailable"),
            ),
            PortError::Rejected {
                service,
                ref detail,
            } => ApiError::new(
                StatusCode::BAD_GATEWAY,
                format!("{service} rejected the request: {detail}"),
            ),
            // Storage and Other are logged with detail but never echoed: an
            // internal error message can leak schema or file paths.
            PortError::Storage(ref detail) => {
                tracing::error!(error = %detail, "storage failure");
                ApiError::new(StatusCode::INTERNAL_SERVER_ERROR, "Internal server error")
            }
            PortError::Other(ref detail) => {
                tracing::error!(error = %detail, "unexpected failure");
                ApiError::new(StatusCode::INTERNAL_SERVER_ERROR, "Internal server error")
            }
        }
    }
}

impl From<AppError> for ApiError {
    fn from(e: AppError) -> Self {
        match e {
            AppError::Domain(d) => d.into(),
            AppError::Port(p) => p.into(),
            // One message for every credential failure: distinguishing them is
            // a user-enumeration oracle.
            AppError::InvalidCredentials => {
                ApiError::new(StatusCode::UNAUTHORIZED, "Invalid email or password")
            }
            AppError::Unauthenticated => ApiError::unauthenticated(),
            AppError::InvalidToken => ApiError::invalid_token(),
            AppError::RateLimited { retry_after_secs } => {
                let mut err = ApiError::new(
                    StatusCode::TOO_MANY_REQUESTS,
                    "Too many attempts. Please try again later.",
                );
                err.retry_after_secs = Some(retry_after_secs);
                err
            }
            AppError::Internal(ref detail) => {
                tracing::error!(error = %detail, "internal error");
                ApiError::new(StatusCode::INTERNAL_SERVER_ERROR, "Internal server error")
            }
        }
    }
}

/// Maps a CHECK constraint to a message and the request field it came from.
///
/// These back up the domain-layer validation: the domain should reject bad
/// input first with a better message, so reaching here means a gap in that
/// validation. It must still be a 400, never a 500.
fn check_message(constraint: &str) -> (String, Option<String>) {
    match constraint {
        "job_tasks_status_check" => (
            "Task status must be one of pending, in_progress, completed".into(),
            Some("status".into()),
        ),
        "jobs_status_check" => (
            "Job status must be one of active, completed, archived, on_hold".into(),
            Some("status".into()),
        ),
        "users_role_check" => (
            "Role must be one of MANAGER, SUPERVISOR, OFFICE".into(),
            Some("role".into()),
        ),
        "diary_notes_category_check" => (
            "That note category is not recognised".into(),
            Some("category".into()),
        ),
        "call_forward_item_type_check" => (
            "Item type must be one of HEADER, STAGE_CLAIM, TASK".into(),
            Some("itemType".into()),
        ),
        "call_forward_status_check" => (
            "Status must be one of not_started, in_progress, completed, on_hold".into(),
            Some("status".into()),
        ),
        "jobs_date_order_check" => (
            "The end date must be on or after the start date".into(),
            Some("endDate".into()),
        ),
        "progress_percent_range_check" => (
            "Percent complete must be between 0 and 100".into(),
            Some("percentComplete".into()),
        ),
        "scheduler_worker_absence_dates_check" => (
            "The absence end date must be on or after its start date".into(),
            Some("endDate".into()),
        ),
        // Unmapped: still a 400, but without echoing the constraint name.
        _ => ("Invalid value".into(), None),
    }
}

/// Turns a database constraint name into something a user can act on.
///
/// Names come from `backend/migrations/0003_constraints.sql`; anything
/// unmapped falls back to a generic conflict rather than exposing the raw
/// constraint identifier.
fn constraint_message(constraint: &str) -> String {
    match constraint {
        "jobs_company_job_number_unique" => {
            "A job with this number already exists for your company".into()
        }
        "users_email_unique" => "An account with this email already exists".into(),
        "uq_job_assignments_single_primary" => "This job already has a primary supervisor".into(),
        "uq_job_assignments_job_user" => "This user is already assigned to the job".into(),
        "uq_scheduler_company_worker_date" => {
            "That worker is already allocated on this date".into()
        }
        "uq_scheduler_worker_absence_period" => "An identical absence period already exists".into(),
        // Raised by the overlap trigger via RAISE, not by a named constraint,
        // so the repository supplies this name itself -- otherwise the client
        // would only ever see the bare "Conflict" fallback.
        "scheduler_worker_absence_overlap" => {
            "That worker already has an absence overlapping these dates".into()
        }
        "scheduler_allocation_target_check" => {
            "An allocation must target either a job or a maintenance job".into()
        }
        _ => "Conflict".into(),
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    fn body_of(e: ApiError) -> (StatusCode, String) {
        let status = e.status;
        (status, serde_json::to_string(&e.body).unwrap())
    }

    #[test]
    fn billing_body_matches_the_legacy_contract_exactly() {
        let (status, json) = body_of(ApiError::billing_required());
        assert_eq!(status, StatusCode::PAYMENT_REQUIRED);
        assert_eq!(
            json,
            r#"{"error":"Complete billing setup or reactivate your subscription to access the application.","code":"BILLING_REQUIRED"}"#
        );
    }

    #[test]
    fn billing_configuration_body_matches_too() {
        let (_, json) = body_of(ApiError::billing_configuration_required());
        assert_eq!(
            json,
            r#"{"error":"Reactivate your subscription in Billing before changing company settings.","code":"BILLING_REQUIRED"}"#
        );
    }

    #[test]
    fn auth_failures_use_the_legacy_wording() {
        assert_eq!(
            body_of(ApiError::unauthenticated()).1,
            r#"{"error":"Authentication required"}"#
        );
        assert_eq!(
            body_of(ApiError::invalid_token()).1,
            r#"{"error":"Invalid or expired token"}"#
        );
        assert_eq!(
            body_of(ApiError::forbidden()).1,
            r#"{"error":"Insufficient permissions"}"#
        );
    }

    #[test]
    fn code_and_field_are_omitted_when_absent() {
        let (_, json) = body_of(ApiError::new(StatusCode::BAD_REQUEST, "nope"));
        assert_eq!(json, r#"{"error":"nope"}"#, "no null code/field keys");
    }

    #[test]
    fn every_credential_failure_gives_one_indistinguishable_message() {
        let (status, json) = body_of(AppError::InvalidCredentials.into());
        assert_eq!(status, StatusCode::UNAUTHORIZED);
        assert_eq!(json, r#"{"error":"Invalid email or password"}"#);
    }

    #[test]
    fn not_found_names_the_entity_without_leaking_ids() {
        let (status, json) = body_of(DomainError::not_found("Job").into());
        assert_eq!(status, StatusCode::NOT_FOUND);
        assert_eq!(json, r#"{"error":"Job not found"}"#);
    }

    #[test]
    fn internal_errors_never_echo_their_detail() {
        let (status, json) = body_of(AppError::Internal("connection string leaked".into()).into());
        assert_eq!(status, StatusCode::INTERNAL_SERVER_ERROR);
        assert!(
            !json.contains("connection string"),
            "detail must stay in logs"
        );
        assert_eq!(json, r#"{"error":"Internal server error"}"#);
    }

    #[test]
    fn known_constraints_become_actionable_messages() {
        let e: ApiError = PortError::Conflict {
            constraint: Some("jobs_company_job_number_unique".into()),
        }
        .into();
        assert_eq!(e.status, StatusCode::CONFLICT);
        assert_eq!(
            e.body.error,
            "A job with this number already exists for your company"
        );
    }

    #[test]
    fn a_check_violation_is_a_400_not_a_500() {
        let e: ApiError = PortError::CheckViolation {
            constraint: Some("job_tasks_status_check".into()),
        }
        .into();
        assert_eq!(e.status, StatusCode::BAD_REQUEST);
        assert_eq!(e.body.field.as_deref(), Some("status"));
        assert!(e.body.error.contains("pending"));
    }

    #[test]
    fn an_unmapped_check_violation_is_still_a_400_and_leaks_nothing() {
        let e: ApiError = PortError::CheckViolation {
            constraint: Some("some_internal_check_7".into()),
        }
        .into();
        assert_eq!(e.status, StatusCode::BAD_REQUEST);
        assert_eq!(e.body.error, "Invalid value");
        assert!(!e.body.error.contains("some_internal_check_7"));
    }

    #[test]
    fn the_absence_overlap_trigger_gets_a_real_message() {
        let e: ApiError = PortError::Conflict {
            constraint: Some("scheduler_worker_absence_overlap".into()),
        }
        .into();
        assert_eq!(e.status, StatusCode::CONFLICT);
        assert!(e.body.error.contains("overlapping"), "{}", e.body.error);
    }

    #[test]
    fn double_booking_a_worker_explains_itself() {
        let e: ApiError = PortError::Conflict {
            constraint: Some("uq_scheduler_company_worker_date".into()),
        }
        .into();
        assert!(
            e.body.error.contains("already allocated"),
            "{}",
            e.body.error
        );
    }

    #[test]
    fn unknown_constraints_do_not_leak_their_name() {
        let e: ApiError = PortError::Conflict {
            constraint: Some("some_internal_idx_9".into()),
        }
        .into();
        assert_eq!(e.body.error, "Conflict");
    }

    #[test]
    fn validation_errors_carry_the_field_name() {
        let e: ApiError = DomainError::invalid("email", "must be an email address").into();
        assert_eq!(e.status, StatusCode::BAD_REQUEST);
        assert_eq!(e.body.field.as_deref(), Some("email"));
    }
}
