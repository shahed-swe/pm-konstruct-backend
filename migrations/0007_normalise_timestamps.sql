-- PM Konstruct v2 -- normalise timestamps to TIMESTAMPTZ
--
-- The legacy schema mixes 40 `timestamptz` columns with 35 naive `timestamp`
-- columns, apparently by accident: `diary_media.created_at` is aware while
-- `jobs.created_at` is naive, with no principle separating them.
--
-- That matters. This codebase already carries one bug from mixing naive and
-- aware time -- the delay engine compared a local midnight against UTC-parsed
-- dates (domain-rules R1). Leaving 35 naive columns in place means every one
-- of the ~160 remaining endpoints has to remember which kind each column is.
--
-- Safety of the conversion
--   Replit runs its containers in UTC, so the naive values already *are* UTC;
--   `AT TIME ZONE 'UTC'` reinterprets without shifting them. Verified by
--   comparing row values before and after in tools/phase3/test_timestamps.sh.
--   If the legacy host were ever not UTC this would be wrong, which is exactly
--   why it is done once, here, under review -- rather than implicitly in 35
--   different Rust structs.
--
-- This is a deliberate divergence from the legacy schema. The Phase 3
-- verifier reports it as an intentional change via its allowlist rather than
-- silently accepting it.

DO $$
DECLARE
  r record;
  n int := 0;
BEGIN
  FOR r IN
    SELECT c.table_name, c.column_name
    FROM information_schema.columns c
    JOIN information_schema.tables t
      ON t.table_schema = c.table_schema AND t.table_name = c.table_name
    WHERE c.table_schema = 'public'
      AND t.table_type = 'BASE TABLE'
      AND c.data_type = 'timestamp without time zone'
    ORDER BY c.table_name, c.ordinal_position
  LOOP
    EXECUTE format(
      'ALTER TABLE %I ALTER COLUMN %I TYPE TIMESTAMPTZ USING %I AT TIME ZONE ''UTC''',
      r.table_name, r.column_name, r.column_name);
    n := n + 1;
  END LOOP;
  RAISE NOTICE 'converted % naive timestamp column(s) to timestamptz', n;
END
$$;

-- Defaults are re-stated because ALTER TYPE keeps the old expression's type.
DO $$
DECLARE r record;
BEGIN
  FOR r IN
    SELECT c.table_name, c.column_name
    FROM information_schema.columns c
    WHERE c.table_schema = 'public'
      AND c.data_type = 'timestamp with time zone'
      AND c.column_default LIKE '%now()%'
  LOOP
    EXECUTE format('ALTER TABLE %I ALTER COLUMN %I SET DEFAULT NOW()',
                   r.table_name, r.column_name);
  END LOOP;
END
$$;
