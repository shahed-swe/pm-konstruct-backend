//! The Stripe operations billing needs.
//!
//! A port rather than a direct dependency for the usual reason -- the tests
//! run against `stripe-mock` and the domain never sees an HTTP type -- and for
//! one specific to this rebuild: the legacy read Stripe's state out of a
//! `stripe.*` schema maintained by Replit's managed sync. That infrastructure
//! does not exist anywhere else, so the rebuild has to ask Stripe itself, and
//! keeping that behind a trait makes the switch explicit rather than
//! scattered.

use async_trait::async_trait;

use crate::PortResult;

/// A Stripe customer, as much of one as this needs.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct Customer {
    pub id: String,
}

/// A subscription's current state.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct Subscription {
    pub id: String,
    pub customer_id: String,
    /// Stripe's own spelling, parsed by the domain.
    pub status: String,
    /// Seats across all items.
    pub quantity: i64,
    pub current_period_end: Option<i64>,
    pub cancel_at_period_end: bool,
}

/// A hosted page the customer is redirected to.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct HostedSession {
    pub id: String,
    pub url: String,
}

/// What a completed checkout produced.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct CheckoutResult {
    pub customer_id: Option<String>,
    pub subscription_id: Option<String>,
    /// `complete`, `open` or `expired`.
    pub status: String,
}

/// What a new checkout is for.
#[derive(Debug, Clone)]
pub struct CheckoutRequest {
    pub customer_id: String,
    pub price_id: String,
    pub seats: i64,
    pub trial_days: Option<i64>,
    pub success_url: String,
    pub cancel_url: String,
    /// Carried through so the confirmation can tell which company it was for
    /// without trusting the client's claim.
    pub company_id: i32,
}

#[async_trait]
pub trait StripeGateway: Send + Sync {
    /// Whether the configured key talks to live Stripe or test mode.
    ///
    /// Checked at startup so a deployment cannot quietly charge real cards
    /// from a staging environment, or fail to charge from production.
    async fn is_livemode(&self) -> PortResult<bool>;

    /// The price id for a plan, creating the product and price if needed.
    ///
    /// Idempotent on the plan key, so restarting does not accumulate
    /// duplicate prices in the Stripe account.
    async fn price_for_plan(
        &self,
        plan_key: &str,
        label: &str,
        unit_amount: i64,
        currency: &str,
    ) -> PortResult<String>;

    async fn create_customer(&self, email: &str, company_name: &str) -> PortResult<Customer>;

    async fn create_checkout_session(&self, req: &CheckoutRequest) -> PortResult<HostedSession>;

    async fn retrieve_checkout_session(&self, session_id: &str) -> PortResult<CheckoutResult>;

    async fn retrieve_subscription(
        &self,
        subscription_id: &str,
    ) -> PortResult<Option<Subscription>>;

    /// The customer's most recent subscription, for recovering a link the
    /// database lost.
    async fn latest_subscription_for_customer(
        &self,
        customer_id: &str,
    ) -> PortResult<Option<Subscription>>;

    async fn create_portal_session(
        &self,
        customer_id: &str,
        return_url: &str,
    ) -> PortResult<HostedSession>;
}
