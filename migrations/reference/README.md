# Frozen legacy schema reference

`legacy-schema-2026-09-21.sql` is a `pg_dump --schema-only` of the legacy PM
Konstruct development database, taken 2026-09-21 at git `644887237c`
(`_schema_version = 25`).

It is committed so the Phase 3 equivalence check is **self-contained and
reproducible in CI** rather than depending on a path outside the repo. It is
schema only: no rows, no credentials.

Used by:
- `tools/phase3/verify_baseline.sh` — default reference
- `tools/phase3/gen_baseline.py` — source for `0001_baseline.sql`
- `.github/workflows/ci.yml` — the `schema` job

Override with `PMK_LEGACY_SCHEMA=/path/to/schema.sql`.

**Refresh this whenever a production snapshot is taken** (every 8 weeks — see
`docs/migration/drift-register.md`). A refresh that changes this file means the
live schema moved, and `0001_baseline.sql` plus a new forward migration must
follow.
