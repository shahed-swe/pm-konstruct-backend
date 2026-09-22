//! Billing shapes.

use pmk_app::billing::{CheckoutOutcome, Readiness};
use pmk_domain::billing::BillingStatus;
use serde::{Deserialize, Serialize};

#[derive(Debug, Serialize)]
#[serde(rename_all = "camelCase")]
pub struct PlanDto {
    pub key: &'static str,
    pub label: &'static str,
    pub min_seats: i64,
    pub max_seats: Option<i64>,
    /// Cents per seat per month.
    pub unit_amount: i64,
}

/// The pricing page. Public: it is what a prospect sees before signing up.
#[derive(Debug, Serialize)]
#[serde(rename_all = "camelCase")]
pub struct PlansDto {
    pub currency: &'static str,
    pub trial_days: i64,
    pub plans: Vec<PlanDto>,
}

impl PlansDto {
    #[must_use]
    pub fn current() -> Self {
        let (currency, trial_days, plans) = pmk_app::billing::BillingService::plans();
        Self {
            currency,
            trial_days,
            plans: plans
                .iter()
                .map(|p| PlanDto {
                    key: p.key,
                    label: p.label,
                    min_seats: p.min_seats,
                    max_seats: p.max_seats,
                    unit_amount: p.unit_amount,
                })
                .collect(),
        }
    }
}

#[derive(Debug, Serialize)]
#[serde(rename_all = "camelCase")]
pub struct BillingStatusDto {
    pub status: &'static str,
    pub has_access: bool,
    pub onboarding_complete: bool,
    pub active_users: i64,
    pub seat_limit: i64,
    pub current_period_end: Option<chrono::DateTime<chrono::Utc>>,
    pub cancel_at_period_end: bool,
    pub trial_ends_at: Option<chrono::DateTime<chrono::Utc>>,
    pub plan_key: Option<String>,
}

impl From<BillingStatus> for BillingStatusDto {
    fn from(b: BillingStatus) -> Self {
        Self {
            status: b.status.as_str(),
            has_access: b.has_access,
            onboarding_complete: b.onboarding_complete,
            active_users: b.active_users,
            seat_limit: b.seat_limit,
            current_period_end: b.current_period_end,
            cancel_at_period_end: b.cancel_at_period_end,
            trial_ends_at: b.trial_ends_at,
            plan_key: b.plan_key,
        }
    }
}

#[derive(Debug, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct CheckoutRequestDto {
    pub plan_key: String,
    pub seat_quantity: i64,
}

#[derive(Debug, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct ConfirmCheckoutRequest {
    pub session_id: String,
}

#[derive(Debug, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct ReadinessRequest {
    pub setup_key: String,
}

/// Where to send the customer next.
#[derive(Debug, Serialize)]
#[serde(rename_all = "camelCase")]
pub struct HostedSessionDto {
    pub session_id: String,
    pub url: String,
}

impl From<CheckoutOutcome> for HostedSessionDto {
    fn from(c: CheckoutOutcome) -> Self {
        Self {
            session_id: c.session_id,
            url: c.url,
        }
    }
}

#[derive(Debug, Serialize)]
#[serde(rename_all = "camelCase")]
pub struct ReadinessDto {
    pub configured: bool,
    pub livemode: Option<bool>,
    pub expected_livemode: bool,
    pub mode_matches: bool,
    pub ready: bool,
    pub prices: Vec<PriceStatusDto>,
    pub problems: Vec<String>,
}

#[derive(Debug, Serialize)]
#[serde(rename_all = "camelCase")]
pub struct PriceStatusDto {
    pub plan_key: String,
    pub price_id: Option<String>,
}

impl From<Readiness> for ReadinessDto {
    fn from(r: Readiness) -> Self {
        Self {
            ready: r.configured && r.mode_matches && r.problems.is_empty(),
            configured: r.configured,
            livemode: r.livemode,
            expected_livemode: r.expected_livemode,
            mode_matches: r.mode_matches,
            prices: r
                .prices
                .into_iter()
                .map(|(plan_key, price_id)| PriceStatusDto { plan_key, price_id })
                .collect(),
            problems: r.problems,
        }
    }
}
