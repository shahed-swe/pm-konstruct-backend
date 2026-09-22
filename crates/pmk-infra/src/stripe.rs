//! `StripeGateway` over Stripe's REST API.
//!
//! Hand-rolled against the HTTP API rather than through a client crate: this
//! uses eight endpoints, the request bodies are form-encoded, and the
//! responses are read for a handful of fields each. A crate that models the
//! whole API would be a large dependency for that, and would still need
//! wrapping to keep Stripe's types out of the domain.
//!
//! The base URL is configurable so the tests can point at `stripe-mock`,
//! which speaks the same API without an account or a key.

use async_trait::async_trait;
use pmk_ports::{
    CheckoutRequest, CheckoutResult, Customer, HostedSession, PortError, PortResult, StripeGateway,
    Subscription,
};
use serde::Deserialize;
use std::time::Duration;

/// Stripe's live API. Overridden in development to reach `stripe-mock`.
pub const DEFAULT_BASE_URL: &str = "https://api.stripe.com";

/// Pinned so an account's default version cannot change the shape of a
/// response under a running deployment.
const API_VERSION: &str = "2024-06-20";

pub struct StripeClient {
    base_url: String,
    secret_key: String,
    http: reqwest::Client,
}

impl std::fmt::Debug for StripeClient {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        // The key is deliberately absent.
        f.debug_struct("StripeClient")
            .field("base_url", &self.base_url)
            .finish_non_exhaustive()
    }
}

impl StripeClient {
    /// `None` when no secret key is configured, which leaves billing
    /// unavailable rather than broken.
    #[must_use]
    pub fn new(secret_key: &str, base_url: &str) -> Option<Self> {
        let key = secret_key.trim();
        if key.is_empty() {
            return None;
        }
        Some(Self {
            base_url: base_url.trim_end_matches('/').to_string(),
            secret_key: key.to_string(),
            http: reqwest::Client::builder()
                .timeout(Duration::from_secs(20))
                .build()
                .unwrap_or_default(),
        })
    }

    async fn post<T: for<'de> Deserialize<'de>>(
        &self,
        path: &str,
        form: &[(&str, String)],
        idempotency_key: Option<&str>,
    ) -> PortResult<T> {
        let mut req = self
            .http
            .post(format!("{}{path}", self.base_url))
            .basic_auth(&self.secret_key, None::<&str>)
            .header("Stripe-Version", API_VERSION)
            .form(form);

        // Stripe deduplicates on this, so a retried request cannot create a
        // second customer or a second subscription.
        if let Some(key) = idempotency_key {
            req = req.header("Idempotency-Key", key);
        }

        let response = req.send().await.map_err(unavailable)?;
        Self::read(response).await
    }

    async fn get<T: for<'de> Deserialize<'de>>(
        &self,
        path: &str,
        query: &[(&str, String)],
    ) -> PortResult<T> {
        let response = self
            .http
            .get(format!("{}{path}", self.base_url))
            .basic_auth(&self.secret_key, None::<&str>)
            .header("Stripe-Version", API_VERSION)
            .query(query)
            .send()
            .await
            .map_err(unavailable)?;
        Self::read(response).await
    }

    async fn read<T: for<'de> Deserialize<'de>>(response: reqwest::Response) -> PortResult<T> {
        let status = response.status();
        if status.is_success() {
            return response.json::<T>().await.map_err(unavailable);
        }

        let message = response
            .json::<ErrorEnvelope>()
            .await
            .ok()
            .and_then(|e| e.error.message)
            .unwrap_or_else(|| format!("Stripe returned {status}"));

        // An authentication failure is never surfaced: Stripe quotes the
        // offending `Authorization` header back in the message, so passing it
        // on would put the secret key in an API response and in whatever
        // captured it. Logged at the server, where the key already lives.
        if status == reqwest::StatusCode::UNAUTHORIZED || status == reqwest::StatusCode::FORBIDDEN {
            tracing::error!(%status, "Stripe rejected the API key");
            return Err(PortError::Rejected {
                service: "stripe",
                detail: "the configured Stripe key was rejected".to_string(),
            });
        }

        // Every other message describes the request rather than the account,
        // and is what makes a bad plan or price actionable.
        Err(PortError::Rejected {
            service: "stripe",
            detail: message,
        })
    }
}

fn unavailable(e: impl std::fmt::Display) -> PortError {
    PortError::Unavailable {
        service: "stripe",
        detail: e.to_string(),
    }
}

#[derive(Deserialize)]
struct ErrorEnvelope {
    #[serde(default)]
    error: StripeError,
}

#[derive(Deserialize, Default)]
struct StripeError {
    #[serde(default)]
    message: Option<String>,
}

#[derive(Deserialize)]
struct IdOnly {
    id: String,
}

#[derive(Deserialize)]
struct BalanceResponse {
    #[serde(default)]
    livemode: bool,
}

#[derive(Deserialize)]
struct SearchResponse<T> {
    #[serde(default = "Vec::new")]
    data: Vec<T>,
}

#[derive(Deserialize)]
struct PriceResponse {
    id: String,
}

#[derive(Deserialize)]
struct SessionResponse {
    id: String,
    #[serde(default)]
    url: Option<String>,
    #[serde(default)]
    status: Option<String>,
    #[serde(default)]
    customer: Option<String>,
    #[serde(default)]
    subscription: Option<String>,
}

#[derive(Deserialize)]
struct SubscriptionResponse {
    id: String,
    #[serde(default)]
    customer: Option<String>,
    #[serde(default)]
    status: Option<String>,
    #[serde(default)]
    current_period_end: Option<i64>,
    #[serde(default)]
    cancel_at_period_end: bool,
    #[serde(default)]
    items: SubscriptionItems,
}

#[derive(Deserialize, Default)]
struct SubscriptionItems {
    #[serde(default = "Vec::new")]
    data: Vec<SubscriptionItem>,
}

#[derive(Deserialize)]
struct SubscriptionItem {
    #[serde(default)]
    quantity: Option<i64>,
}

impl From<SubscriptionResponse> for Subscription {
    fn from(s: SubscriptionResponse) -> Self {
        Self {
            id: s.id,
            customer_id: s.customer.unwrap_or_default(),
            status: s.status.unwrap_or_else(|| "unknown".to_string()),
            // Summed across items: a subscription can carry more than one,
            // and the seat count is all of them.
            quantity: s.items.data.iter().filter_map(|i| i.quantity).sum(),
            current_period_end: s.current_period_end,
            cancel_at_period_end: s.cancel_at_period_end,
        }
    }
}

#[async_trait]
impl StripeGateway for StripeClient {
    async fn is_livemode(&self) -> PortResult<bool> {
        let balance: BalanceResponse = self.get("/v1/balance", &[]).await?;
        Ok(balance.livemode)
    }

    async fn price_for_plan(
        &self,
        plan_key: &str,
        label: &str,
        unit_amount: i64,
        currency: &str,
    ) -> PortResult<String> {
        // Looked up by the lookup key first, so a restart reuses the existing
        // price rather than creating another one beside it. Stripe prices are
        // immutable, and duplicates would make the catalogue unreadable.
        let lookup_key = format!("pmk_{plan_key}");
        let found: SearchResponse<PriceResponse> = self
            .get(
                "/v1/prices",
                &[
                    ("lookup_keys[]", lookup_key.clone()),
                    ("active", "true".to_string()),
                    ("limit", "1".to_string()),
                ],
            )
            .await?;
        if let Some(price) = found.data.into_iter().next() {
            return Ok(price.id);
        }

        let product: IdOnly = self
            .post(
                "/v1/products",
                &[
                    ("name", format!("PM Konstruct — {label}")),
                    ("metadata[plan_key]", plan_key.to_string()),
                ],
                Some(&format!("pmk-product-{plan_key}")),
            )
            .await?;

        let price: PriceResponse = self
            .post(
                "/v1/prices",
                &[
                    ("product", product.id),
                    ("unit_amount", unit_amount.to_string()),
                    ("currency", currency.to_string()),
                    ("recurring[interval]", "month".to_string()),
                    // Charged per seat, so the quantity on the subscription
                    // item is what the customer pays for.
                    ("recurring[usage_type]", "licensed".to_string()),
                    ("lookup_key", lookup_key),
                    ("metadata[plan_key]", plan_key.to_string()),
                ],
                Some(&format!("pmk-price-{plan_key}-{unit_amount}-{currency}")),
            )
            .await?;
        Ok(price.id)
    }

    async fn create_customer(&self, email: &str, company_name: &str) -> PortResult<Customer> {
        let customer: IdOnly = self
            .post(
                "/v1/customers",
                &[
                    ("email", email.to_string()),
                    ("name", company_name.to_string()),
                ],
                None,
            )
            .await?;
        Ok(Customer { id: customer.id })
    }

    async fn create_checkout_session(&self, req: &CheckoutRequest) -> PortResult<HostedSession> {
        let mut form = vec![
            ("mode", "subscription".to_string()),
            ("customer", req.customer_id.clone()),
            ("line_items[0][price]", req.price_id.clone()),
            ("line_items[0][quantity]", req.seats.to_string()),
            ("success_url", req.success_url.clone()),
            ("cancel_url", req.cancel_url.clone()),
            // Read back on confirmation, so the company a session belongs to
            // comes from Stripe rather than from the client.
            ("metadata[company_id]", req.company_id.to_string()),
            (
                "subscription_data[metadata][company_id]",
                req.company_id.to_string(),
            ),
        ];
        if let Some(days) = req.trial_days {
            form.push(("subscription_data[trial_period_days]", days.to_string()));
        }

        let session: SessionResponse = self.post("/v1/checkout/sessions", &form, None).await?;
        Ok(HostedSession {
            id: session.id,
            url: session.url.unwrap_or_default(),
        })
    }

    async fn retrieve_checkout_session(&self, session_id: &str) -> PortResult<CheckoutResult> {
        let session: SessionResponse = self
            .get(&format!("/v1/checkout/sessions/{session_id}"), &[])
            .await?;
        Ok(CheckoutResult {
            customer_id: session.customer,
            subscription_id: session.subscription,
            status: session.status.unwrap_or_else(|| "open".to_string()),
        })
    }

    async fn retrieve_subscription(
        &self,
        subscription_id: &str,
    ) -> PortResult<Option<Subscription>> {
        match self
            .get::<SubscriptionResponse>(&format!("/v1/subscriptions/{subscription_id}"), &[])
            .await
        {
            Ok(s) => Ok(Some(s.into())),
            // A subscription Stripe has never heard of is an answer, not a
            // failure: the database can hold an id from a deleted test mode.
            Err(PortError::Rejected { .. }) => Ok(None),
            Err(e) => Err(e),
        }
    }

    async fn latest_subscription_for_customer(
        &self,
        customer_id: &str,
    ) -> PortResult<Option<Subscription>> {
        let found: SearchResponse<SubscriptionResponse> = self
            .get(
                "/v1/subscriptions",
                &[
                    ("customer", customer_id.to_string()),
                    ("status", "all".to_string()),
                    ("limit", "1".to_string()),
                ],
            )
            .await?;
        Ok(found.data.into_iter().next().map(Into::into))
    }

    async fn create_portal_session(
        &self,
        customer_id: &str,
        return_url: &str,
    ) -> PortResult<HostedSession> {
        let session: SessionResponse = self
            .post(
                "/v1/billing_portal/sessions",
                &[
                    ("customer", customer_id.to_string()),
                    ("return_url", return_url.to_string()),
                ],
                None,
            )
            .await?;
        Ok(HostedSession {
            id: session.id,
            url: session.url.unwrap_or_default(),
        })
    }
}
