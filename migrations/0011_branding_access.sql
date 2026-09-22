-- Two corrections to how `company_branding` is constrained and reached.
--
-- 1. `job_display_mode` also allows 'job_name'.
--
--    The legacy schema had no CHECK on this column, and the settings route
--    accepts job_number, job_name and job_address (routes/settings.ts:66).
--    The constraint added in 0003 was built from the two values seen in the
--    data and left job_name out, so choosing "job name" in the UI would have
--    been accepted by the API and then rejected by the database as a 500.
--
-- 2. The login page can read branding without a session.
--
--    `company_branding` is RLS-protected on `current_company_id()`, which is
--    not set before anyone has authenticated -- but the login page needs the
--    company's name, colours and logo to render. The same problem the auth
--    lookups have, solved the same way: a SECURITY DEFINER function that
--    exposes exactly one row and nothing else.
--
--    It returns the *oldest* company's branding rather than a hardcoded id,
--    matching the legacy behaviour, so this stays correct as companies are
--    added or removed.

ALTER TABLE company_branding DROP CONSTRAINT IF EXISTS company_branding_job_display_mode_check;

ALTER TABLE company_branding ADD CONSTRAINT company_branding_job_display_mode_check
  CHECK (job_display_mode IN ('job_number', 'job_name', 'job_address'));

CREATE OR REPLACE FUNCTION public_default_branding()
RETURNS TABLE (
  company_name     TEXT,
  logo_url         TEXT,
  banner_url       TEXT,
  primary_color    TEXT,
  sidebar_color    TEXT,
  job_display_mode TEXT,
  email_send_mode  TEXT,
  updated_at       TIMESTAMPTZ
)
LANGUAGE sql
SECURITY DEFINER
SET search_path = public, pg_catalog
STABLE
AS $$
  SELECT b.company_name, b.logo_url, b.banner_url, b.primary_color,
         b.sidebar_color, b.job_display_mode, b.email_send_mode, b.updated_at
  FROM company_branding b
  WHERE b.company_id = (SELECT c.id FROM companies c ORDER BY c.id LIMIT 1)
$$;

REVOKE ALL ON FUNCTION public_default_branding() FROM PUBLIC;
GRANT EXECUTE ON FUNCTION public_default_branding() TO pmk_app;
