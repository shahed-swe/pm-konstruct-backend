//! `BillingRepository` over Postgres.
//!
//! Everything here runs without a tenant scope and goes through the SECURITY
//! DEFINER functions in migrations 0006, 0010 and 0014. `companies` is
//! RLS-protected, and the entitlement in particular is read while resolving a
//! session -- before the tenant GUC exists.

use async_trait::async_trait;
use pmk_domain::tenant::CompanyId;
use pmk_ports::repository::{BillingRecord, BillingRepository, Entitlement};
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

    async fn record(&self, company: CompanyId) -> PortResult<Option<BillingRecord>> {
        // Through the SECURITY DEFINER function, like the entitlement above:
        // `companies` is RLS-protected and this is read while resolving a
        // session, before the tenant GUC is set.
        let row = sqlx::query(
            "SELECT name, stripe_customer_id, stripe_subscription_id, billing_plan_key, \
                    billing_price_id, billing_seat_quantity, billing_trial_ends_at, \
                    billing_onboarding_completed \
             FROM auth_company_billing($1)",
        )
        .bind(company.get())
        .fetch_optional(&self.pool)
        .await
        .map_err(map_sqlx)?;

        Ok(row.map(|r| {
            use sqlx::Row as _;
            BillingRecord {
                company_name: r.get("name"),
                stripe_customer_id: r.get("stripe_customer_id"),
                stripe_subscription_id: r.get("stripe_subscription_id"),
                plan_key: r.get("billing_plan_key"),
                price_id: r.get("billing_price_id"),
                seat_quantity: r
                    .get::<Option<i32>, _>("billing_seat_quantity")
                    .map(i64::from),
                trial_ends_at: r.get("billing_trial_ends_at"),
                onboarding_complete: r.get("billing_onboarding_completed"),
            }
        }))
    }

    async fn set_customer(&self, company: CompanyId, customer_id: &str) -> PortResult<()> {
        sqlx::query("SELECT auth_set_billing_customer($1,$2)")
            .bind(company.get())
            .bind(customer_id)
            .execute(&self.pool)
            .await
            .map_err(map_sqlx)?;
        Ok(())
    }

    async fn set_subscription(
        &self,
        company: CompanyId,
        subscription_id: &str,
        plan_key: &str,
        price_id: &str,
        seats: i64,
    ) -> PortResult<()> {
        sqlx::query("SELECT auth_set_billing_subscription($1,$2,$3,$4,$5)")
            .bind(company.get())
            .bind(subscription_id)
            .bind(plan_key)
            .bind(price_id)
            .bind(i32::try_from(seats).unwrap_or(i32::MAX))
            .execute(&self.pool)
            .await
            .map_err(map_sqlx)?;
        Ok(())
    }

    async fn complete_onboarding(&self, company: CompanyId) -> PortResult<()> {
        sqlx::query("SELECT auth_complete_billing_onboarding($1)")
            .bind(company.get())
            .execute(&self.pool)
            .await
            .map_err(map_sqlx)?;
        Ok(())
    }

    async fn active_user_count(&self, company: CompanyId) -> PortResult<i64> {
        let n: i64 = sqlx::query_scalar("SELECT auth_active_user_count($1)")
            .bind(company.get())
            .fetch_one(&self.pool)
            .await
            .map_err(map_sqlx)?;
        Ok(n)
    }
}
