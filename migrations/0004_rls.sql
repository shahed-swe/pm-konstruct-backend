-- PM Konstruct v2 -- row-level security
--
-- Second line of defence for tenant isolation. The primary guard is the Rust
-- type system: `TenantScope` is unforgeable and every repository method demands
-- one, so a query without tenant scoping is a compile error (Architecture 2).
--
-- RLS catches anything that slips past -- a raw query, a new endpoint, a
-- refactor. Legacy tenancy was opt-in and applied by hand per service, with
-- several services carrying no company_id reference at all (Analysis 4.3).
--
-- Mechanics
--   The application connects as `pmk_app` and sets one GUC per transaction:
--       SET LOCAL app.company_id = '<id>';
--   Policies compare company_id against it. With the GUC unset, current_setting
--   returns NULL and every policy evaluates false, so an unscoped query returns
--   zero rows rather than everything -- fail closed, not open.
--
--   `pmk_migrator` and `pmk_admin` BYPASSRLS for migrations, the worker and the
--   CLI (billing webhooks and media GC are legitimately cross-tenant).

-- ── roles ─────────────────────────────────────────────────────────────────
-- Created without passwords; the deployment assigns them (see infra/).
DO $$
BEGIN
  IF NOT EXISTS (SELECT 1 FROM pg_roles WHERE rolname = 'pmk_app') THEN
    CREATE ROLE pmk_app NOLOGIN;
  END IF;
  IF NOT EXISTS (SELECT 1 FROM pg_roles WHERE rolname = 'pmk_admin') THEN
    CREATE ROLE pmk_admin NOLOGIN BYPASSRLS;
  END IF;
  IF NOT EXISTS (SELECT 1 FROM pg_roles WHERE rolname = 'pmk_migrator') THEN
    CREATE ROLE pmk_migrator NOLOGIN BYPASSRLS;
  END IF;
END
$$;

-- ── helper: the current tenant, or NULL when unset ────────────────────────
CREATE OR REPLACE FUNCTION current_company_id() RETURNS integer
  LANGUAGE sql STABLE
  -- SECURITY INVOKER; empty search_path so the body cannot be shadowed.
  SET search_path = pg_catalog
  AS $$
    SELECT NULLIF(current_setting('app.company_id', true), '')::integer
  $$;

COMMENT ON FUNCTION current_company_id() IS
  'Tenant for the current transaction, from the app.company_id GUC. NULL when unset, which makes every RLS policy fail closed.';

-- ── directly-scoped tables: they carry company_id ─────────────────────────
DO $$
DECLARE
  t text;
  direct text[] := ARRAY[
    'companies', 'users', 'jobs', 'company_branding', 'email_settings',
    'call_forward_templates', 'scheduler_workers', 'scheduler_worker_absences',
    'scheduler_allocations', 'scheduler_job_day_notes', 'scheduler_maintenance_jobs',
    'inspection_forms', 'job_dropbox_folders', 'job_dropbox_uploads',
    'job_dropbox_auto_uploads'
  ];
BEGIN
  FOREACH t IN ARRAY direct LOOP
    EXECUTE format('ALTER TABLE %I ENABLE ROW LEVEL SECURITY', t);
    EXECUTE format('ALTER TABLE %I FORCE ROW LEVEL SECURITY', t);
    EXECUTE format('DROP POLICY IF EXISTS tenant_isolation ON %I', t);
    IF t = 'companies' THEN
      EXECUTE format($f$
        CREATE POLICY tenant_isolation ON %I
          USING (id = current_company_id())
          WITH CHECK (id = current_company_id())$f$, t);
    ELSE
      EXECUTE format($f$
        CREATE POLICY tenant_isolation ON %I
          USING (company_id = current_company_id())
          WITH CHECK (company_id = current_company_id())$f$, t);
    END IF;
    EXECUTE format('GRANT SELECT, INSERT, UPDATE, DELETE ON %I TO pmk_app', t);
  END LOOP;
END
$$;

-- ── indirectly-scoped tables: reached through a parent ────────────────────
-- Each policy is an EXISTS against the owning row. The indexes added in 0002
-- keep these cheap; the FK column is always indexed.
DO $$
DECLARE
  r record;
  -- table, local fk column, parent table, parent column, parent's tenant path
  indirect text[][] := ARRAY[
    ARRAY['job_assignments',        'job_id',          'jobs',          'id'],
    ARRAY['job_tasks',              'job_id',          'jobs',          'id'],
    ARRAY['site_diary',             'job_id',          'jobs',          'id'],
    ARRAY['job_media',              'job_id',          'jobs',          'id'],
    ARRAY['call_forward',           'job_id',          'jobs',          'id'],
    ARRAY['progress',               'job_id',          'jobs',          'id'],
    ARRAY['reports',                'job_id',          'jobs',          'id'],
    ARRAY['eto_job_sequences',      'job_id',          'jobs',          'id'],
    ARRAY['diary_notes',            'diary_entry_id',  'site_diary',    'id'],
    ARRAY['diary_media',            'diary_entry_id',  'site_diary',    'id'],
    ARRAY['weather_snapshots',      'diary_entry_id',  'site_diary',    'id'],
    ARRAY['diary_note_comments',    'note_id',         'diary_notes',   'id'],
    ARRAY['inspection_form_items',  'form_id',         'inspection_forms', 'id'],
    ARRAY['inspection_form_media',  'form_id',         'inspection_forms', 'id'],
    ARRAY['user_permissions',       'user_id',         'users',         'id'],
    ARRAY['password_reset_tokens',  'user_id',         'users',         'id'],
    ARRAY['push_subscriptions',     'user_id',         'users',         'id'],
    ARRAY['user_notification_prefs','user_id',         'users',         'id'],
    ARRAY['notifications',          'user_id',         'users',         'id'],
    ARRAY['ai_usage_daily',         'user_id',         'users',         'id']
  ];
  tbl text; fk text; parent text; pcol text; pred text;
BEGIN
  FOR i IN 1 .. array_length(indirect, 1) LOOP
    tbl := indirect[i][1]; fk := indirect[i][2];
    parent := indirect[i][3]; pcol := indirect[i][4];

    -- Parents are themselves RLS-protected, so an EXISTS over the parent is
    -- already tenant-filtered. Nesting stays one level deep because jobs,
    -- users and inspection_forms all carry company_id directly; site_diary and
    -- diary_notes resolve through jobs.
    IF parent IN ('jobs', 'users', 'inspection_forms') THEN
      pred := format(
        'EXISTS (SELECT 1 FROM %I p WHERE p.%I = %I.%I AND p.company_id = current_company_id())',
        parent, pcol, tbl, fk);
    ELSIF parent = 'site_diary' THEN
      pred := format($p$EXISTS (
          SELECT 1 FROM site_diary sd JOIN jobs j ON j.id = sd.job_id
          WHERE sd.id = %I.%I AND j.company_id = current_company_id())$p$, tbl, fk);
    ELSE  -- diary_notes
      pred := format($p$EXISTS (
          SELECT 1 FROM diary_notes dn
          JOIN site_diary sd ON sd.id = dn.diary_entry_id
          JOIN jobs j ON j.id = sd.job_id
          WHERE dn.id = %I.%I AND j.company_id = current_company_id())$p$, tbl, fk);
    END IF;

    EXECUTE format('ALTER TABLE %I ENABLE ROW LEVEL SECURITY', tbl);
    EXECUTE format('ALTER TABLE %I FORCE ROW LEVEL SECURITY', tbl);
    EXECUTE format('DROP POLICY IF EXISTS tenant_isolation ON %I', tbl);
    EXECUTE format('CREATE POLICY tenant_isolation ON %I USING (%s) WITH CHECK (%s)',
                   tbl, pred, pred);
    EXECUTE format('GRANT SELECT, INSERT, UPDATE, DELETE ON %I TO pmk_app', tbl);
  END LOOP;
END
$$;

-- ── global tables: no tenant dimension, admin-only ────────────────────────
-- media_deletion_queue keys on stored_name alone and is processed by the worker.
-- billing_webhook_notification* is Stripe delivery state, inherently cross-tenant.
DO $$
DECLARE
  t text;
  global text[] := ARRAY['media_deletion_queue', 'billing_webhook_notifications',
                         'billing_webhook_notification_recipients'];
BEGIN
  FOREACH t IN ARRAY global LOOP
    EXECUTE format('ALTER TABLE %I ENABLE ROW LEVEL SECURITY', t);
    -- No policy: pmk_app sees nothing. Only BYPASSRLS roles may touch these.
    EXECUTE format('REVOKE ALL ON %I FROM pmk_app', t);
  END LOOP;
END
$$;

GRANT USAGE ON SCHEMA public TO pmk_app;
GRANT USAGE, SELECT ON ALL SEQUENCES IN SCHEMA public TO pmk_app;
GRANT EXECUTE ON FUNCTION current_company_id() TO pmk_app;
