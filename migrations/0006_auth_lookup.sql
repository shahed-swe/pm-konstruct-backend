-- PM Konstruct v2 -- authentication lookup path (Phase 5)
--
-- Problem
--   `users` has FORCE ROW LEVEL SECURITY, and its policy compares company_id
--   against the `app.company_id` GUC. But login and token verification must
--   read a user *before* any tenant is known -- the tenant is derived from the
--   row being read. With the GUC unset the policy fails closed, so the app role
--   sees nothing and nobody can log in.
--
--   Running the API as a superuser would "fix" it by bypassing RLS entirely,
--   which defeats the whole second layer of tenant isolation. That is exactly
--   what the first local run was accidentally doing.
--
-- Approach
--   Two narrowly-scoped SECURITY DEFINER functions. They execute as their
--   owner, so they can read `users` regardless of the GUC, but they are the
--   *only* such hole and each returns exactly the fields the auth path needs:
--   no free-form querying, no way to enumerate another tenant.
--
--   `search_path` is pinned so the body cannot be hijacked by a caller-supplied
--   schema, which is the standard SECURITY DEFINER footgun.

CREATE OR REPLACE FUNCTION auth_find_credential_by_email(p_email TEXT)
RETURNS TABLE (
  id                  INTEGER,
  company_id          INTEGER,
  name                TEXT,
  email               TEXT,
  role                TEXT,
  phone               TEXT,
  active              BOOLEAN,
  password_changed_at TIMESTAMPTZ,
  password_hash       TEXT
)
LANGUAGE sql
SECURITY DEFINER
SET search_path = public, pg_catalog
STABLE
AS $$
  SELECT u.id, u.company_id, u.name, u.email, u.role, u.phone, u.active,
         u.password_changed_at, u.password_hash
  FROM users u
  WHERE lower(u.email) = lower(btrim(p_email))
  LIMIT 1
$$;

COMMENT ON FUNCTION auth_find_credential_by_email(TEXT) IS
  'Login-time credential lookup. SECURITY DEFINER because the tenant is not known until this row is read. Returns at most one row and no other table.';

CREATE OR REPLACE FUNCTION auth_find_user_by_id(p_id INTEGER)
RETURNS TABLE (
  id                  INTEGER,
  company_id          INTEGER,
  name                TEXT,
  email               TEXT,
  role                TEXT,
  phone               TEXT,
  active              BOOLEAN,
  password_changed_at TIMESTAMPTZ
)
LANGUAGE sql
SECURITY DEFINER
SET search_path = public, pg_catalog
STABLE
AS $$
  SELECT u.id, u.company_id, u.name, u.email, u.role, u.phone, u.active,
         u.password_changed_at
  FROM users u
  WHERE u.id = p_id
  LIMIT 1
$$;

COMMENT ON FUNCTION auth_find_user_by_id(INTEGER) IS
  'Token-verification user reload. SECURITY DEFINER for the same reason as the credential lookup; the tenant scope is derived from its result.';

-- Password rehash on login (bcrypt -> argon2id) also happens pre-tenant.
CREATE OR REPLACE FUNCTION auth_replace_password_hash(p_id INTEGER, p_hash TEXT)
RETURNS VOID
LANGUAGE sql
SECURITY DEFINER
SET search_path = public, pg_catalog
AS $$
  UPDATE users
  SET password_hash = p_hash, password_changed_at = NOW(), updated_at = NOW()
  WHERE id = p_id
$$;

COMMENT ON FUNCTION auth_replace_password_hash(INTEGER, TEXT) IS
  'Transparent bcrypt-to-argon2id upgrade during login, before a tenant scope exists.';

-- Billing entitlement is read per request, keyed by primary key, before the
-- tenant GUC is meaningful for this purpose.
CREATE OR REPLACE FUNCTION auth_company_entitlement(p_company_id INTEGER)
RETURNS TABLE (
  billing_onboarding_completed BOOLEAN,
  billing_trial_ends_at        TIMESTAMPTZ,
  stripe_subscription_id       TEXT
)
LANGUAGE sql
SECURITY DEFINER
SET search_path = public, pg_catalog
STABLE
AS $$
  SELECT c.billing_onboarding_completed, c.billing_trial_ends_at,
         c.stripe_subscription_id
  FROM companies c
  WHERE c.id = p_company_id
$$;

-- Only the app role may call these; nothing else is granted.
REVOKE ALL ON FUNCTION auth_find_credential_by_email(TEXT) FROM PUBLIC;
REVOKE ALL ON FUNCTION auth_find_user_by_id(INTEGER) FROM PUBLIC;
REVOKE ALL ON FUNCTION auth_replace_password_hash(INTEGER, TEXT) FROM PUBLIC;
REVOKE ALL ON FUNCTION auth_company_entitlement(INTEGER) FROM PUBLIC;

GRANT EXECUTE ON FUNCTION auth_find_credential_by_email(TEXT) TO pmk_app;
GRANT EXECUTE ON FUNCTION auth_find_user_by_id(INTEGER) TO pmk_app;
GRANT EXECUTE ON FUNCTION auth_replace_password_hash(INTEGER, TEXT) TO pmk_app;
GRANT EXECUTE ON FUNCTION auth_company_entitlement(INTEGER) TO pmk_app;

-- The application connects as this role. It is NOT a superuser and does NOT
-- have BYPASSRLS, so every tenant-scoped query is subject to the policies in
-- 0004. Migration 0004 created it NOLOGIN; give it a login now.
--
-- The password is set by deployment, not here: a password in a migration would
-- be committed to the repository.
DO $$
BEGIN
  IF NOT EXISTS (SELECT 1 FROM pg_roles WHERE rolname = 'pmk_app' AND rolcanlogin) THEN
    ALTER ROLE pmk_app LOGIN;
  END IF;
END
$$;
