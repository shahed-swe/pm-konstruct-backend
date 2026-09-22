//! `BillingRepository` over Postgres.
//!
//! Until Phase 15 wires Stripe, entitlement is read from the `companies`
//! billing columns the legacy system already maintains, with the same
//! permissive default (`billing_onboarding_completed` defaults to true).

use async_trait::async_trait;
use pmk_domain::tenant::CompanyId;
use pmk_ports::repository::{BillingRepository, Entitlement};
use pmk_ports::PortResult;
use sqlx::PgPool;

use super::user::map_sqlx;

#[derive(Debug, Clone)]
pub struct PgBillingRepository {
    pool: PgPool,
}

impl PgBillingRepository {
    #[must_use]
    pub const fn new(pool: PgPool) -> Self {
        Self { pool }
    }
}

#[derive(Debug, sqlx::FromRow)]
struct Row {
    billing_onboarding_completed: bool,
    billing_trial_ends_at: Option<chrono::DateTime<chrono::Utc>>,
    stripe_subscription_id: Option<String>,
    billing_seat_quantity: Option<i32>,
}

#[async_trait]
impl BillingRepository for PgBillingRepository {
    async fn entitlement(&self, company: CompanyId) -> PortResult<Entitlement> {
        // `companies` is RLS-protected and this is read while resolving the
        // session, before the tenant GUC is set. Goes through the scoped
        // SECURITY DEFINER function from migration 0006.
        let row: Option<Row> = sqlx::query_as(
            "SELECT billing_onboarding_completed, billing_trial_ends_at, \
                    stripe_subscription_id, billing_seat_quantity \
             FROM auth_company_entitlement($1)",
        )
        .bind(company.get())
        .fetch_optional(&self.pool)
        .await
        .map_err(map_sqlx)?;

        let Some(row) = row else {
            // No company means no access, rather than defaulting open.
            return Ok(Entitlement {
                can_access_application: false,
                can_configure_account: false,
                onboarding_complete: false,
                seat_limit: Some(0),
            });
        };

        // Phase 15 replaces this with real subscription-status logic. Today it
        // mirrors the legacy behaviour: onboarding complete, or an unexpired
        // trial, or a subscription on file.
        let trial_active = row
            .billing_trial_ends_at
            .is_some_and(|t| t > chrono::Utc::now());
        let allowed = row.billing_onboarding_completed
            || trial_active
            || row.stripe_subscription_id.is_some();

        Ok(Entitlement {
            can_access_application: allowed,
            can_configure_account: allowed,
            onboarding_complete: row.billing_onboarding_completed,
            // A null quantity means the plan does not cap seats, which is
            // every company today: nothing has ever written this column.
            seat_limit: row.billing_seat_quantity.map(i64::from),
        })
    }
}
