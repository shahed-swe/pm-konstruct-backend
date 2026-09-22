//! Billing routes.
//!
//! `/plans` is public -- it is the pricing page a prospect reads before
//! signing up. `/status` is any entitled user, because the banner that says
//! "your trial ends on the 3rd" appears for everyone. Everything that spends
//! money or changes the subscription is manager-only.

use axum::extract::State;
use axum::routing::{get, post};
use axum::{Json, Router};

use crate::dto::{
    BillingStatusDto, CheckoutRequestDto, ConfirmCheckoutRequest, HostedSessionDto, PlansDto,
    ReadinessDto, ReadinessRequest,
};
use crate::error::ApiError;
use crate::extract::{Entitled, ManagerOnly, RequireRole};
use crate::state::AppState;

pub fn router() -> Router<AppState> {
    Router::new()
        .route("/plans", get(plans))
        .route("/status", get(status))
        .route("/checkout", post(checkout))
        .route("/checkout/confirm", post(confirm_checkout))
        .route("/onboarding-complete", post(onboarding_complete))
        .route("/portal", post(portal))
        .route("/readiness", post(readiness))
}

/// The plan list. No authentication: it is a price list.
async fn plans() -> Json<PlansDto> {
    Json(PlansDto::current())
}

async fn status(
    State(state): State<AppState>,
    Entitled(session): Entitled,
) -> Result<Json<BillingStatusDto>, ApiError> {
    Ok(Json(state.billing.status(&session).await?.into()))
}

async fn checkout(
    State(state): State<AppState>,
    RequireRole(session): ManagerOnly,
    Json(req): Json<CheckoutRequestDto>,
) -> Result<Json<HostedSessionDto>, ApiError> {
    let out = state
        .billing
        .start_checkout(&session, &req.plan_key, req.seat_quantity)
        .await?;
    Ok(Json(out.into()))
}

/// Records a checkout the customer has completed.
///
/// The subscription is read from Stripe's copy of the session rather than
/// taken from the request, and the customer on it must match the one recorded
/// against this company.
async fn confirm_checkout(
    State(state): State<AppState>,
    RequireRole(session): ManagerOnly,
    Json(req): Json<ConfirmCheckoutRequest>,
) -> Result<Json<BillingStatusDto>, ApiError> {
    let status = state
        .billing
        .confirm_checkout(&session, &req.session_id)
        .await?;
    Ok(Json(status.into()))
}

async fn onboarding_complete(
    State(state): State<AppState>,
    RequireRole(session): ManagerOnly,
) -> Result<Json<BillingStatusDto>, ApiError> {
    Ok(Json(
        state.billing.complete_onboarding(&session).await?.into(),
    ))
}

async fn portal(
    State(state): State<AppState>,
    RequireRole(session): ManagerOnly,
) -> Result<Json<HostedSessionDto>, ApiError> {
    Ok(Json(state.billing.portal(&session).await?.into()))
}

/// Whether this deployment can take a payment.
///
/// Behind the deployment secret as well as the manager check: it reports
/// which Stripe account and mode the server is wired to.
async fn readiness(
    State(state): State<AppState>,
    RequireRole(_session): ManagerOnly,
    Json(req): Json<ReadinessRequest>,
) -> Result<Json<ReadinessDto>, ApiError> {
    Ok(Json(state.billing.readiness(&req.setup_key).await?.into()))
}
