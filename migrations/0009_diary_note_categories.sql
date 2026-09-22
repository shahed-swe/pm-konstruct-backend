-- Widen `diary_notes_category_check` to cover 'property-inspection'.
--
-- The legacy schema had no CHECK on this column at all; 0003 added one built
-- from the categories the diary UI offers plus 'eto'. That list missed
-- 'property-inspection', which the inspection form writes -- so creating a
-- draft failed on the constraint.
--
-- Nothing in production hit it: `inspection_forms` holds 0 rows and no note
-- carries the category, because the feature has never been used (see
-- docs/audit/feature-usage.md). The constraint was still wrong, and would
-- have made the feature dead on arrival the first time anyone opened it.
--
-- Recreated rather than widened in place: Postgres has no ALTER CONSTRAINT
-- for a CHECK expression.

ALTER TABLE diary_notes DROP CONSTRAINT IF EXISTS diary_notes_category_check;

ALTER TABLE diary_notes ADD CONSTRAINT diary_notes_category_check
  CHECK (category IN ('general', 'client', 'trades', 'site_conditions',
                      'issues', 'safety', 'materials', 'eto',
                      'property-inspection'));
