-- Expose the seat limit through `auth_company_entitlement`.
--
-- Adding a user is capped by `companies.billing_seat_quantity`, but the API
-- role cannot read `companies` directly: the table is RLS-protected and the
-- entitlement is resolved while establishing the session, before the tenant
-- GUC is set. Everything the session needs therefore comes through this
-- SECURITY DEFINER function, and the seat count belongs with it.
--
-- `CREATE OR REPLACE` cannot change a function's return type, so the old one
-- is dropped first. Nothing else references it.

DROP FUNCTION IF EXISTS auth_company_entitlement(INTEGER);

CREATE FUNCTION auth_company_entitlement(p_company_id INTEGER)
RETURNS TABLE (
  billing_onboarding_completed BOOLEAN,
  billing_trial_ends_at        TIMESTAMPTZ,
  stripe_subscription_id       TEXT,
  billing_seat_quantity        INTEGER
)
LANGUAGE sql
SECURITY DEFINER
SET search_path = public, pg_catalog
STABLE
AS $$
  SELECT c.billing_onboarding_completed, c.billing_trial_ends_at,
         c.stripe_subscription_id, c.billing_seat_quantity
  FROM companies c
  WHERE c.id = p_company_id
$$;

REVOKE ALL ON FUNCTION auth_company_entitlement(INTEGER) FROM PUBLIC;
GRANT EXECUTE ON FUNCTION auth_company_entitlement(INTEGER) TO pmk_app;
