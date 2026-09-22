-- Functions for the three paths that run without a session.
--
-- First-run setup, self-service registration and password recovery all happen
-- before the caller has a tenant -- registration *creates* the tenant, and
-- recovery is used by someone who cannot sign in. The application role has no
-- RLS context in any of them, so each goes through a SECURITY DEFINER function
-- that touches exactly the rows it needs and returns nothing else.
--
-- This is the same shape as the auth lookups in 0006, for the same reason.

-- Is this a fresh installation?
--
-- Only ever answers yes or no. It cannot be used to count or enumerate users.
CREATE OR REPLACE FUNCTION auth_any_users_exist()
RETURNS BOOLEAN
LANGUAGE sql
SECURITY DEFINER
SET search_path = public, pg_catalog
STABLE
AS $$
  SELECT EXISTS (SELECT 1 FROM users)
$$;

-- Creates a company with its first manager, atomically.
--
-- The caller runs this inside a SERIALIZABLE transaction, so two concurrent
-- registrations cannot both observe an empty database and the duplicate checks
-- below cannot both pass before either commits. The alternative -- checking
-- then inserting -- has a window in which two companies of the same name are
-- created.
--
-- The isolation level is set by the caller rather than here: by the time a
-- function body runs, a query is already in flight and SET TRANSACTION is no
-- longer allowed.
--
-- `p_bootstrap` is first-run setup, which additionally refuses once any user
-- exists. Ordinary registration passes false and may run at any time.
--
-- Errors are raised with explicit SQLSTATEs so the API can tell them apart
-- without matching on message text:
--   23505 -- the email or company name is taken
--   P0001 -- setup has already been completed
CREATE OR REPLACE FUNCTION auth_register_company(
  p_company_name TEXT,
  p_name         TEXT,
  p_email        TEXT,
  p_password_hash TEXT,
  p_phone        TEXT,
  p_bootstrap    BOOLEAN
)
RETURNS TABLE (
  company_id INTEGER,
  user_id    INTEGER,
  name       TEXT,
  email      TEXT,
  role       TEXT,
  phone      TEXT,
  active     BOOLEAN
)
LANGUAGE plpgsql
SECURITY DEFINER
SET search_path = public, pg_catalog
AS $$
-- The RETURNS TABLE names are variables in scope here and would shadow the
-- identically named columns, so a bare `company_id` inside an INSERT is
-- ambiguous. This says an ambiguous name means the column; the locals below
-- are prefixed and never ambiguous.
#variable_conflict use_column
DECLARE
  v_company_id INTEGER;
  v_user_id    INTEGER;
BEGIN
  IF p_bootstrap AND EXISTS (SELECT 1 FROM users) THEN
    RAISE EXCEPTION 'Setup has already been completed' USING ERRCODE = 'P0001';
  END IF;

  IF EXISTS (SELECT 1 FROM users u WHERE u.email = p_email) THEN
    RAISE EXCEPTION 'An account with this email already exists.'
      USING ERRCODE = '23505';
  END IF;

  IF EXISTS (
    SELECT 1 FROM companies c WHERE lower(trim(c.name)) = lower(trim(p_company_name))
  ) THEN
    RAISE EXCEPTION 'A company with this name already exists. Sign in or use a more specific company name.'
      USING ERRCODE = '23505';
  END IF;

  -- A new company starts with a 30-day trial and onboarding incomplete, so
  -- the billing gate sends it to the setup flow rather than straight in.
  INSERT INTO companies (name, billing_onboarding_completed, billing_trial_ends_at)
  VALUES (trim(p_company_name), FALSE, now() + INTERVAL '30 days')
  RETURNING id INTO v_company_id;

  INSERT INTO users (company_id, name, email, role, phone, active, password_hash)
  VALUES (v_company_id, trim(p_name), p_email, 'MANAGER',
          NULLIF(trim(COALESCE(p_phone, '')), ''), TRUE, p_password_hash)
  RETURNING id INTO v_user_id;

  -- Settings rows so the first visit to either page has something to edit.
  INSERT INTO email_settings (company_id) VALUES (v_company_id)
  ON CONFLICT (company_id) DO NOTHING;

  INSERT INTO company_branding (company_id, company_name)
  VALUES (v_company_id, trim(p_company_name))
  ON CONFLICT (company_id) DO NOTHING;

  RETURN QUERY
  SELECT v_company_id, u.id, u.name, u.email, u.role, u.phone, u.active
  FROM users u WHERE u.id = v_user_id;
END
$$;

-- Who owns this address, and how many active users their company has.
--
-- Returns no row for an unknown or inactive address. The caller answers
-- identically either way, so this cannot be used to test which addresses
-- exist.
CREATE OR REPLACE FUNCTION auth_recovery_context(p_email TEXT)
RETURNS TABLE (user_id INTEGER, active_users BIGINT)
LANGUAGE sql
SECURITY DEFINER
SET search_path = public, pg_catalog
STABLE
AS $$
  SELECT u.id,
         (SELECT count(*) FROM users p WHERE p.company_id = u.company_id AND p.active)
  FROM users u
  WHERE u.email = p_email AND u.active
  LIMIT 1
$$;

-- Stores an emailed reset token, spending any outstanding one first.
--
-- Two valid tokens at once would mean an older email still worked after a
-- newer one was requested.
CREATE OR REPLACE FUNCTION auth_store_reset_token(
  p_user_id INTEGER,
  p_token   TEXT,
  p_expires TIMESTAMPTZ
)
RETURNS VOID
LANGUAGE sql
SECURITY DEFINER
SET search_path = public, pg_catalog
AS $$
  WITH spent AS (
    UPDATE password_reset_tokens SET used_at = now()
    WHERE user_id = p_user_id AND used_at IS NULL
    RETURNING 1
  )
  INSERT INTO password_reset_tokens (user_id, token, expires_at)
  VALUES (p_user_id, p_token, p_expires)
$$;

-- Finds an unexpired, unused token matching either shape.
--
-- An emailed link is stored verbatim; a manager-issued code is stored only as
-- a prefixed hash. The caller cannot know which it holds, so both are tried.
-- An empty hash never matches, which is what an input with no usable
-- characters produces.
CREATE OR REPLACE FUNCTION auth_find_reset_token(p_raw TEXT, p_hashed TEXT)
RETURNS TABLE (id INTEGER, user_id INTEGER)
LANGUAGE sql
SECURITY DEFINER
SET search_path = public, pg_catalog
STABLE
AS $$
  SELECT t.id, t.user_id
  FROM password_reset_tokens t
  WHERE (t.token = p_raw OR (p_hashed <> '' AND t.token = p_hashed))
    AND t.used_at IS NULL
    AND t.expires_at > now()
  ORDER BY t.id DESC
  LIMIT 1
$$;

-- Spends a token and sets the password, in one statement.
--
-- The UPDATE is conditional on the token still being unspent, so a concurrent
-- reset -- or a newly issued code that superseded this one -- makes it match
-- nothing and the password is left alone. Returns whether it took.
CREATE OR REPLACE FUNCTION auth_claim_reset(
  p_token_id      INTEGER,
  p_user_id       INTEGER,
  p_password_hash TEXT
)
RETURNS BOOLEAN
LANGUAGE plpgsql
SECURITY DEFINER
SET search_path = public, pg_catalog
AS $$
DECLARE
  v_claimed INTEGER;
BEGIN
  UPDATE password_reset_tokens
  SET used_at = now()
  WHERE id = p_token_id AND used_at IS NULL AND expires_at > now()
  RETURNING id INTO v_claimed;

  IF v_claimed IS NULL THEN
    RETURN FALSE;
  END IF;

  UPDATE users
  SET password_hash = p_password_hash, password_changed_at = now(), updated_at = now()
  WHERE id = p_user_id;

  -- Every other outstanding token for this user is spent too: having reset
  -- the password, an older link must not still work.
  UPDATE password_reset_tokens
  SET used_at = now()
  WHERE user_id = p_user_id AND used_at IS NULL;

  RETURN TRUE;
END
$$;

REVOKE ALL ON FUNCTION auth_any_users_exist() FROM PUBLIC;
REVOKE ALL ON FUNCTION auth_register_company(TEXT, TEXT, TEXT, TEXT, TEXT, BOOLEAN) FROM PUBLIC;
REVOKE ALL ON FUNCTION auth_recovery_context(TEXT) FROM PUBLIC;
REVOKE ALL ON FUNCTION auth_store_reset_token(INTEGER, TEXT, TIMESTAMPTZ) FROM PUBLIC;
REVOKE ALL ON FUNCTION auth_find_reset_token(TEXT, TEXT) FROM PUBLIC;
REVOKE ALL ON FUNCTION auth_claim_reset(INTEGER, INTEGER, TEXT) FROM PUBLIC;

GRANT EXECUTE ON FUNCTION auth_any_users_exist() TO pmk_app;
GRANT EXECUTE ON FUNCTION auth_register_company(TEXT, TEXT, TEXT, TEXT, TEXT, BOOLEAN) TO pmk_app;
GRANT EXECUTE ON FUNCTION auth_recovery_context(TEXT) TO pmk_app;
GRANT EXECUTE ON FUNCTION auth_store_reset_token(INTEGER, TEXT, TIMESTAMPTZ) TO pmk_app;
GRANT EXECUTE ON FUNCTION auth_find_reset_token(TEXT, TEXT) TO pmk_app;
GRANT EXECUTE ON FUNCTION auth_claim_reset(INTEGER, INTEGER, TEXT) TO pmk_app;
