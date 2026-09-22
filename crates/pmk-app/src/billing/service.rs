use std::sync::Arc;

use pmk_domain::billing::{
    validate_checkout, BillingStatus, SubscriptionStatus, CURRENCY, PLANS, TRIAL_DAYS,
};
use pmk_domain::tenant::CompanyId;
use pmk_domain::DomainError;
use pmk_ports::repository::{BillingRecord, BillingRepository};
use pmk_ports::{CheckoutRequest, StripeGateway, Subscription};

use crate::identity::SessionUser;
use crate::{AppError, AppResult};

/// Where the hosted pages send the customer back to.
#[derive(Debug, Clone)]
pub struct ReturnUrls {
    pub success: String,
    pub cancel: String,
    pub portal_return: String,
}

/// A checkout that is ready to be paid.
#[derive(Debug, Clone)]
pub struct CheckoutOutcome {
    pub session_id: String,
    pub url: String,
}

/// Whether this deployment can actually take a payment.
///
/// Checked before anyone is sent to a checkout page, so a misconfiguration
/// surfaces to an administrator rather than to a customer mid-purchase.
#[derive(Debug, Clone)]
pub struct Readiness {
    pub configured: bool,
    pub livemode: Option<bool>,
    pub expected_livemode: bool,
    pub mode_matches: bool,
    pub prices: Vec<(String, Option<String>)>,
    pub problems: Vec<String>,
}

pub struct BillingService {
    billing: Arc<dyn BillingRepository>,
    /// `None` when no Stripe key is configured. Billing is then unavailable
    /// rather than broken -- which is how production runs today.
    stripe: Option<Arc<dyn StripeGateway>>,
    urls: ReturnUrls,
    /// What this deployment expects Stripe to be. A mismatch means a staging
    /// environment is pointed at live keys, or production at test ones.
    expect_livemode: bool,
    setup_secret: String,
}

impl std::fmt::Debug for BillingService {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        f.debug_struct("BillingService")
            .field("configured", &self.stripe.is_some())
            .field("expect_livemode", &self.expect_livemode)
            .finish_non_exhaustive()
    }
}

impl BillingService {
    #[must_use]
    pub fn new(
        billing: Arc<dyn BillingRepository>,
        stripe: Option<Arc<dyn StripeGateway>>,
        urls: ReturnUrls,
        expect_livemode: bool,
        setup_secret: String,
    ) -> Self {
        Self {
            billing,
            stripe,
            urls,
            expect_livemode,
            setup_secret,
        }
    }

    fn gateway(&self) -> AppResult<&Arc<dyn StripeGateway>> {
        self.stripe.as_ref().ok_or_else(|| {
            AppError::FeatureUnavailable("Billing is not configured on this server.".into())
        })
    }

    async fn record(&self, company: CompanyId) -> AppResult<BillingRecord> {
        self.billing
            .record(company)
            .await?
            .ok_or_else(|| AppError::Domain(DomainError::not_found("Company")))
    }

    /// The company's current billing position.
    ///
    /// Stripe is the source of truth for the subscription's state; the
    /// database only holds the linkage. The legacy read both out of a
    /// `stripe.*` schema that Replit's managed sync maintained, which does
    /// not exist outside Replit.
    pub async fn status(&self, s: &SessionUser) -> AppResult<BillingStatus> {
        let company = s.principal.company_id();
        let record = self.record(company).await?;
        let active_users = self.billing.active_user_count(company).await?;

        let subscription = self.fetch_subscription(&record).await;

        let (status, seat_limit, current_period_end, cancel_at_period_end) = match &subscription {
            Some(sub) => (
                SubscriptionStatus::parse(&sub.status),
                sub.quantity,
                sub.current_period_end
                    .and_then(|t| chrono::DateTime::from_timestamp(t, 0)),
                sub.cancel_at_period_end,
            ),
            // No subscription at all: a company that predates billing, which
            // keeps its access. Locking those out would be the rollout
            // breaking every existing customer.
            None => (SubscriptionStatus::Legacy, 0, None, false),
        };

        Ok(BillingStatus {
            has_access: status.grants_access(),
            status,
            onboarding_complete: record.onboarding_complete,
            active_users,
            seat_limit,
            current_period_end,
            cancel_at_period_end,
            trial_ends_at: record.trial_ends_at,
            plan_key: record.plan_key,
        })
    }

    /// Reads the subscription from Stripe, tolerating every way it can fail.
    ///
    /// A billing page that 500s because Stripe is slow is worse than one that
    /// shows a company as legacy for a minute, so every error here becomes
    /// "no subscription" with a warning in the log.
    async fn fetch_subscription(&self, record: &BillingRecord) -> Option<Subscription> {
        let stripe = self.stripe.as_ref()?;

        if let Some(id) = record.stripe_subscription_id.as_deref() {
            match stripe.retrieve_subscription(id).await {
                Ok(Some(sub)) => return Some(sub),
                Ok(None) => {}
                Err(e) => {
                    tracing::warn!(error = %e, "could not read the subscription from Stripe");
                    return None;
                }
            }
        }

        // The stored id can be missing or stale -- a confirmation that did not
        // finish writing, or a subscription replaced through the portal. The
        // customer's latest is the recovery path.
        let customer = record.stripe_customer_id.as_deref()?;
        match stripe.latest_subscription_for_customer(customer).await {
            Ok(sub) => sub,
            Err(e) => {
                tracing::warn!(error = %e, "could not list subscriptions from Stripe");
                None
            }
        }
    }

    /// Starts a checkout for a plan and seat count.
    pub async fn start_checkout(
        &self,
        s: &SessionUser,
        plan_key: &str,
        seats: i64,
    ) -> AppResult<CheckoutOutcome> {
        let stripe = self.gateway()?;
        let company = s.principal.company_id();

        let current = self.status(s).await?;
        if !current.status.may_start_checkout() {
            return Err(AppError::Domain(DomainError::Conflict(
                "Manage your existing subscription through the billing portal.".into(),
            )));
        }

        let plan =
            validate_checkout(plan_key, seats, current.active_users).map_err(AppError::Domain)?;

        let record = self.record(company).await?;
        let price_id = stripe
            .price_for_plan(plan.key, plan.label, plan.unit_amount, CURRENCY)
            .await?;

        // Created once and remembered: a second customer for the same company
        // would split its invoice history.
        let customer_id = match record.stripe_customer_id {
            Some(id) => id,
            None => {
                let customer = stripe
                    .create_customer(&s.user.email, &record.company_name)
                    .await?;
                self.billing.set_customer(company, &customer.id).await?;
                customer.id
            }
        };

        // The trial is offered only to a company that has never subscribed,
        // so cancelling and re-subscribing cannot earn another free month.
        let trial_days = record
            .stripe_subscription_id
            .is_none()
            .then_some(TRIAL_DAYS);

        let session = stripe
            .create_checkout_session(&CheckoutRequest {
                customer_id,
                price_id,
                seats,
                trial_days,
                success_url: self.urls.success.clone(),
                cancel_url: self.urls.cancel.clone(),
                company_id: company.get(),
            })
            .await?;

        Ok(CheckoutOutcome {
            session_id: session.id,
            url: session.url,
        })
    }

    /// Records a checkout the customer has completed.
    ///
    /// The subscription id comes from Stripe's copy of the session, not from
    /// the client: a caller could otherwise claim any session id and have it
    /// written against their company.
    pub async fn confirm_checkout(
        &self,
        s: &SessionUser,
        session_id: &str,
    ) -> AppResult<BillingStatus> {
        if session_id.trim().is_empty() {
            return Err(AppError::Domain(DomainError::invalid(
                "sessionId",
                "Checkout session ID is required.",
            )));
        }
        let stripe = self.gateway()?;
        let company = s.principal.company_id();

        let result = stripe.retrieve_checkout_session(session_id).await?;
        if result.status != "complete" {
            return Err(AppError::Domain(DomainError::Conflict(
                "That checkout has not been completed.".into(),
            )));
        }
        let Some(subscription_id) = result.subscription_id else {
            return Err(AppError::Domain(DomainError::Conflict(
                "That checkout did not create a subscription.".into(),
            )));
        };

        let subscription = stripe
            .retrieve_subscription(&subscription_id)
            .await?
            .ok_or_else(|| AppError::Domain(DomainError::not_found("Subscription")))?;

        // The customer on the subscription must be the one recorded against
        // this company. Without this a manager could confirm somebody else's
        // session and attach their subscription to their own company.
        let record = self.record(company).await?;
        if record.stripe_customer_id.as_deref() != Some(subscription.customer_id.as_str()) {
            return Err(AppError::Domain(DomainError::Forbidden(
                "That checkout belongs to another account",
            )));
        }

        let plan = pmk_domain::billing::plan_for_seats(subscription.quantity);
        let price_id = record.price_id.unwrap_or_default();
        self.billing
            .set_subscription(
                company,
                &subscription.id,
                plan.key,
                &price_id,
                subscription.quantity,
            )
            .await?;

        self.status(s).await
    }

    /// Marks the billing walkthrough finished.
    pub async fn complete_onboarding(&self, s: &SessionUser) -> AppResult<BillingStatus> {
        self.billing
            .complete_onboarding(s.principal.company_id())
            .await?;
        self.status(s).await
    }

    /// A link to Stripe's customer portal.
    pub async fn portal(&self, s: &SessionUser) -> AppResult<CheckoutOutcome> {
        let stripe = self.gateway()?;
        let record = self.record(s.principal.company_id()).await?;
        let Some(customer) = record.stripe_customer_id else {
            return Err(AppError::Domain(DomainError::Conflict(
                "Start a subscription before opening the billing portal.".into(),
            )));
        };
        let session = stripe
            .create_portal_session(&customer, &self.urls.portal_return)
            .await?;
        Ok(CheckoutOutcome {
            session_id: session.id,
            url: session.url,
        })
    }

    /// Whether this deployment can take a payment at all.
    ///
    /// Behind the deployment secret, because it reports which Stripe account
    /// and mode the server is wired to.
    pub async fn readiness(&self, setup_key: &str) -> AppResult<Readiness> {
        if self.setup_secret.is_empty() || setup_key != self.setup_secret {
            return Err(AppError::Domain(DomainError::Forbidden(
                "Invalid setup key.",
            )));
        }

        let Some(stripe) = &self.stripe else {
            return Ok(Readiness {
                configured: false,
                livemode: None,
                expected_livemode: self.expect_livemode,
                mode_matches: false,
                prices: Vec::new(),
                problems: vec!["No Stripe secret key is configured.".to_string()],
            });
        };

        let mut problems = Vec::new();
        let livemode = match stripe.is_livemode().await {
            Ok(v) => Some(v),
            Err(e) => {
                problems.push(format!("Stripe did not answer: {e}"));
                None
            }
        };
        let mode_matches = livemode == Some(self.expect_livemode);
        if livemode.is_some() && !mode_matches {
            // Getting this wrong means either charging real cards from a
            // staging environment or failing to charge from production.
            problems.push(format!(
                "Stripe is in {} mode but this deployment expects {} mode.",
                if livemode == Some(true) {
                    "live"
                } else {
                    "test"
                },
                if self.expect_livemode { "live" } else { "test" },
            ));
        }

        let mut prices = Vec::new();
        for plan in PLANS {
            match stripe
                .price_for_plan(plan.key, plan.label, plan.unit_amount, CURRENCY)
                .await
            {
                Ok(id) => prices.push((plan.key.to_string(), Some(id))),
                Err(e) => {
                    problems.push(format!("No price for the {} plan: {e}", plan.key));
                    prices.push((plan.key.to_string(), None));
                }
            }
        }

        Ok(Readiness {
            configured: true,
            livemode,
            expected_livemode: self.expect_livemode,
            mode_matches,
            prices,
            problems,
        })
    }

    /// The public plan list, for the pricing page.
    #[must_use]
    pub fn plans() -> (&'static str, i64, &'static [pmk_domain::billing::Plan]) {
        (CURRENCY, TRIAL_DAYS, &PLANS)
    }
}
