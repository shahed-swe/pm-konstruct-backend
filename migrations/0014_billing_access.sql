-- Billing reads and writes, for a caller with no tenant scope.
--
-- `companies` is RLS-protected on `current_company_id()`, and the billing
-- pages are reached while resolving a session -- the entitlement check runs
-- before the GUC is set. These follow the same pattern as the auth lookups in
-- 0006: one function per operation, each touching exactly the row it needs.
--
-- Every one takes the company id explicitly. That is the scope, and it comes
-- from the verified session rather than from the request body.

CREATE OR REPLACE FUNCTION auth_company_billing(p_company_id INTEGER)
RETURNS TABLE (
  name                         TEXT,
  stripe_customer_id           TEXT,
  stripe_subscription_id       TEXT,
  billing_plan_key             TEXT,
  billing_price_id             TEXT,
  billing_seat_quantity        INTEGER,
  billing_trial_ends_at        TIMESTAMPTZ,
  billing_onboarding_completed BOOLEAN
)
LANGUAGE sql
SECURITY DEFINER
SET search_path = public, pg_catalog
STABLE
AS $$
  SELECT c.name, c.stripe_customer_id, c.stripe_subscription_id,
         c.billing_plan_key, c.billing_price_id, c.billing_seat_quantity,
         c.billing_trial_ends_at, c.billing_onboarding_completed
  FROM companies c
  WHERE c.id = p_company_id
$$;

CREATE OR REPLACE FUNCTION auth_set_billing_customer(
  p_company_id  INTEGER,
  p_customer_id TEXT
)
RETURNS VOID
LANGUAGE sql
SECURITY DEFINER
SET search_path = public, pg_catalog
AS $$
  UPDATE companies SET stripe_customer_id = p_customer_id WHERE id = p_company_id
$$;

-- Records what a completed checkout bought.
--
-- Onboarding is marked complete here as well: a company that has just paid
-- has finished the walkthrough by definition, and leaving it incomplete would
-- send a paying customer back to the setup screen.
CREATE OR REPLACE FUNCTION auth_set_billing_subscription(
  p_company_id      INTEGER,
  p_subscription_id TEXT,
  p_plan_key        TEXT,
  p_price_id        TEXT,
  p_seats           INTEGER
)
RETURNS VOID
LANGUAGE sql
SECURITY DEFINER
SET search_path = public, pg_catalog
AS $$
  UPDATE companies
  SET stripe_subscription_id = p_subscription_id,
      billing_plan_key = p_plan_key,
      billing_price_id = p_price_id,
      billing_seat_quantity = p_seats,
      billing_onboarding_completed = TRUE
  WHERE id = p_company_id
$$;

CREATE OR REPLACE FUNCTION auth_complete_billing_onboarding(p_company_id INTEGER)
RETURNS VOID
LANGUAGE sql
SECURITY DEFINER
SET search_path = public, pg_catalog
AS $$
  UPDATE companies SET billing_onboarding_completed = TRUE WHERE id = p_company_id
$$;

-- Active users, for seat accounting on the billing page.
--
-- Counts one company's users and returns only the number, so it cannot be
-- used to enumerate them.
CREATE OR REPLACE FUNCTION auth_active_user_count(p_company_id INTEGER)
RETURNS BIGINT
LANGUAGE sql
SECURITY DEFINER
SET search_path = public, pg_catalog
STABLE
AS $$
  SELECT count(*) FROM users WHERE company_id = p_company_id AND active
$$;

REVOKE ALL ON FUNCTION auth_company_billing(INTEGER) FROM PUBLIC;
REVOKE ALL ON FUNCTION auth_set_billing_customer(INTEGER, TEXT) FROM PUBLIC;
REVOKE ALL ON FUNCTION auth_set_billing_subscription(INTEGER, TEXT, TEXT, TEXT, INTEGER) FROM PUBLIC;
REVOKE ALL ON FUNCTION auth_complete_billing_onboarding(INTEGER) FROM PUBLIC;
REVOKE ALL ON FUNCTION auth_active_user_count(INTEGER) FROM PUBLIC;

GRANT EXECUTE ON FUNCTION auth_company_billing(INTEGER) TO pmk_app;
GRANT EXECUTE ON FUNCTION auth_set_billing_customer(INTEGER, TEXT) TO pmk_app;
GRANT EXECUTE ON FUNCTION auth_set_billing_subscription(INTEGER, TEXT, TEXT, TEXT, INTEGER) TO pmk_app;
GRANT EXECUTE ON FUNCTION auth_complete_billing_onboarding(INTEGER) TO pmk_app;
GRANT EXECUTE ON FUNCTION auth_active_user_count(INTEGER) TO pmk_app;
