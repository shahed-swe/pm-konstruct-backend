//! Subscription plans and what a subscription entitles a company to.
//!
//! Every figure here is money the client charges, so the bands and prices are
//! pinned by tests. The legacy read Stripe's state out of a `stripe.*` schema
//! that Replit's managed sync maintained; that infrastructure does not exist
//! outside Replit, so the rebuild asks Stripe directly and this module holds
//! the parts that do not need to.

use crate::error::{DomainError, DomainResult};

/// Prices are in Australian cents, per seat, per month.
pub const CURRENCY: &str = "aud";

/// How long a new company may use the application before subscribing.
pub const TRIAL_DAYS: i64 = 30;

/// A subscription package.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct Plan {
    pub key: &'static str,
    pub label: &'static str,
    pub min_seats: i64,
    /// `None` on the top band, which is open-ended.
    pub max_seats: Option<i64>,
    /// Cents per seat per month.
    pub unit_amount: i64,
}

/// The three bands, cheapest per seat at the top end.
pub const PLANS: [Plan; 3] = [
    Plan {
        key: "starter",
        label: "1–5 seats",
        min_seats: 1,
        max_seats: Some(5),
        unit_amount: 1000,
    },
    Plan {
        key: "team",
        label: "6–10 seats",
        min_seats: 6,
        max_seats: Some(10),
        unit_amount: 700,
    },
    Plan {
        key: "scale",
        label: "11+ seats",
        min_seats: 11,
        max_seats: None,
        unit_amount: 500,
    },
];

#[must_use]
pub fn plan_by_key(key: &str) -> Option<Plan> {
    PLANS.iter().copied().find(|p| p.key == key)
}

/// The band a seat count falls in.
///
/// Exactly one band matches any positive count, so a company cannot be sold
/// the wrong tier for its size.
#[must_use]
pub fn plan_for_seats(seats: i64) -> Plan {
    PLANS
        .iter()
        .copied()
        .find(|p| seats <= p.max_seats.unwrap_or(i64::MAX))
        .unwrap_or(PLANS[PLANS.len() - 1])
}

/// Where a subscription stands.
///
/// `Legacy` is a company that predates billing: it has no subscription and
/// keeps its access. Removing that would lock out every existing customer the
/// day billing is switched on.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum SubscriptionStatus {
    Legacy,
    Trialing,
    Active,
    PastDue,
    Unpaid,
    Canceled,
    Incomplete,
    IncompleteExpired,
    Expired,
    Unknown,
}

impl SubscriptionStatus {
    #[must_use]
    pub const fn as_str(self) -> &'static str {
        match self {
            Self::Legacy => "legacy",
            Self::Trialing => "trialing",
            Self::Active => "active",
            Self::PastDue => "past_due",
            Self::Unpaid => "unpaid",
            Self::Canceled => "canceled",
            Self::Incomplete => "incomplete",
            Self::IncompleteExpired => "incomplete_expired",
            Self::Expired => "expired",
            Self::Unknown => "unknown",
        }
    }

    /// Parses the status Stripe reports.
    ///
    /// Anything unrecognised becomes `Unknown`, which does **not** grant
    /// access: a status this code has never heard of is not a reason to let
    /// someone in.
    #[must_use]
    pub fn parse(raw: &str) -> Self {
        match raw.trim() {
            "trialing" => Self::Trialing,
            "active" => Self::Active,
            "past_due" => Self::PastDue,
            "unpaid" => Self::Unpaid,
            "canceled" => Self::Canceled,
            "incomplete" => Self::Incomplete,
            "incomplete_expired" => Self::IncompleteExpired,
            _ => Self::Unknown,
        }
    }

    /// May a company in this state use the application?
    ///
    /// Past due still can: a failed card should prompt a fix, not lock a site
    /// team out of the diary they are writing that morning. Stripe's own
    /// dunning moves it to `unpaid` or `canceled` when it gives up, and those
    /// do not.
    #[must_use]
    pub const fn grants_access(self) -> bool {
        matches!(self, Self::Legacy | Self::Trialing | Self::Active)
    }

    /// May a company in this state change its own subscription?
    ///
    /// An active subscription is managed through the portal rather than by
    /// starting a second checkout.
    #[must_use]
    pub const fn may_start_checkout(self) -> bool {
        !matches!(self, Self::Active)
    }
}

/// Checks a requested plan and seat count against the company's size.
pub fn validate_checkout(plan_key: &str, seats: i64, active_users: i64) -> DomainResult<Plan> {
    let Some(plan) = plan_by_key(plan_key) else {
        return Err(DomainError::invalid(
            "planKey",
            "Choose a valid subscription package.",
        ));
    };
    if seats < 1 {
        return Err(DomainError::invalid(
            "seatQuantity",
            "Seat quantity must be a positive whole number.",
        ));
    }
    if plan_for_seats(seats) != plan {
        return Err(DomainError::invalid(
            "planKey",
            "The selected package does not match the requested seat quantity.",
        ));
    }
    // Buying fewer seats than the company already uses would put it
    // immediately over its limit and block the next user it adds.
    if seats < active_users {
        return Err(DomainError::invalid(
            "seatQuantity",
            format!("Your plan needs at least {active_users} seats for current active users."),
        ));
    }
    Ok(plan)
}

/// What a company's billing page shows.
#[derive(Debug, Clone)]
pub struct BillingStatus {
    pub status: SubscriptionStatus,
    pub has_access: bool,
    pub onboarding_complete: bool,
    pub active_users: i64,
    /// Seats paid for. Zero when there is no subscription.
    pub seat_limit: i64,
    pub current_period_end: Option<chrono::DateTime<chrono::Utc>>,
    pub cancel_at_period_end: bool,
    pub trial_ends_at: Option<chrono::DateTime<chrono::Utc>>,
    pub plan_key: Option<String>,
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn the_bands_cover_every_positive_seat_count_exactly_once() {
        for seats in 1..=200 {
            let plan = plan_for_seats(seats);
            assert!(seats >= plan.min_seats, "{seats} landed below {}", plan.key);
            if let Some(max) = plan.max_seats {
                assert!(seats <= max, "{seats} landed above {}", plan.key);
            }
        }
    }

    #[test]
    fn the_band_boundaries_are_where_the_labels_say() {
        assert_eq!(plan_for_seats(1).key, "starter");
        assert_eq!(plan_for_seats(5).key, "starter");
        assert_eq!(plan_for_seats(6).key, "team");
        assert_eq!(plan_for_seats(10).key, "team");
        assert_eq!(plan_for_seats(11).key, "scale");
        assert_eq!(plan_for_seats(10_000).key, "scale");
    }

    #[test]
    fn the_prices_are_the_ones_the_client_charges() {
        // In cents, per seat, per month. Changing one of these changes an
        // invoice, so they are pinned rather than assumed.
        assert_eq!(plan_by_key("starter").unwrap().unit_amount, 1000);
        assert_eq!(plan_by_key("team").unwrap().unit_amount, 700);
        assert_eq!(plan_by_key("scale").unwrap().unit_amount, 500);
        assert_eq!(CURRENCY, "aud");
        assert_eq!(TRIAL_DAYS, 30);
    }

    #[test]
    fn a_larger_band_costs_less_per_seat() {
        let mut previous = i64::MAX;
        for plan in PLANS {
            assert!(plan.unit_amount < previous, "{} is not cheaper", plan.key);
            previous = plan.unit_amount;
        }
    }

    #[test]
    fn the_bands_are_contiguous() {
        // No seat count falls between two bands.
        for pair in PLANS.windows(2) {
            let (lower, upper) = (pair[0], pair[1]);
            assert_eq!(
                lower.max_seats.unwrap() + 1,
                upper.min_seats,
                "gap between {} and {}",
                lower.key,
                upper.key
            );
        }
    }

    #[test]
    fn an_unknown_plan_key_is_refused() {
        assert!(plan_by_key("enterprise").is_none());
        assert!(validate_checkout("enterprise", 3, 1).is_err());
    }

    #[test]
    fn the_plan_must_match_the_seat_count() {
        // Buying the cheap band for twenty seats would underpay.
        assert!(validate_checkout("starter", 20, 1).is_err());
        assert!(validate_checkout("scale", 3, 1).is_err());
        assert!(validate_checkout("scale", 20, 1).is_ok());
    }

    #[test]
    fn a_company_cannot_buy_fewer_seats_than_it_uses() {
        // It would be over its limit the moment the subscription started.
        let e = validate_checkout("starter", 3, 5).unwrap_err().to_string();
        assert!(e.contains("at least 5 seats"), "{e}");
        assert!(validate_checkout("starter", 5, 5).is_ok());
    }

    #[test]
    fn zero_or_negative_seats_are_refused() {
        assert!(validate_checkout("starter", 0, 0).is_err());
        assert!(validate_checkout("starter", -1, 0).is_err());
    }

    #[test]
    fn only_the_three_working_states_grant_access() {
        for s in [
            SubscriptionStatus::Legacy,
            SubscriptionStatus::Trialing,
            SubscriptionStatus::Active,
        ] {
            assert!(s.grants_access(), "{s:?}");
        }
        for s in [
            SubscriptionStatus::PastDue,
            SubscriptionStatus::Unpaid,
            SubscriptionStatus::Canceled,
            SubscriptionStatus::Incomplete,
            SubscriptionStatus::IncompleteExpired,
            SubscriptionStatus::Expired,
            SubscriptionStatus::Unknown,
        ] {
            assert!(!s.grants_access(), "{s:?}");
        }
    }

    #[test]
    fn a_company_predating_billing_keeps_its_access() {
        // Otherwise switching billing on locks out every existing customer.
        assert!(SubscriptionStatus::Legacy.grants_access());
    }

    #[test]
    fn an_unrecognised_status_does_not_grant_access() {
        // A status this code has never heard of is not a reason to let
        // someone in.
        let s = SubscriptionStatus::parse("something_new");
        assert_eq!(s, SubscriptionStatus::Unknown);
        assert!(!s.grants_access());
    }

    #[test]
    fn the_stripe_statuses_round_trip() {
        for s in [
            SubscriptionStatus::Trialing,
            SubscriptionStatus::Active,
            SubscriptionStatus::PastDue,
            SubscriptionStatus::Unpaid,
            SubscriptionStatus::Canceled,
            SubscriptionStatus::Incomplete,
            SubscriptionStatus::IncompleteExpired,
        ] {
            assert_eq!(SubscriptionStatus::parse(s.as_str()), s);
        }
    }

    #[test]
    fn an_active_subscription_is_changed_through_the_portal() {
        assert!(!SubscriptionStatus::Active.may_start_checkout());
        assert!(SubscriptionStatus::Trialing.may_start_checkout());
        assert!(SubscriptionStatus::Canceled.may_start_checkout());
        assert!(SubscriptionStatus::Legacy.may_start_checkout());
    }
}
