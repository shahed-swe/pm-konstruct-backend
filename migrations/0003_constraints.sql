-- PM Konstruct v2 -- integrity constraints
--
-- The legacy schema stores several closed value sets as bare `text`, so nothing
-- stops a typo becoming a permanent data state. These CHECKs close that.
--
-- Values are derived from BOTH the legacy code constants AND the actual distinct
-- values in the production + development exports (verified 2026-09-21), so no
-- existing row can be rejected. Verified by tools/phase3/verify_constraints.py.
--
-- Deliberately NOT constrained, because the set is open and a new value must not
-- require a migration:
--   notifications.type          code adds types freely (6 seen: call_forward_updated,
--                               new_action_note, note_status_changed, note_comment,
--                               eto_approval_required, eto_approved)
--   scheduler_workers.trade     free text ("Carpenter", "Apprentice", NULL)
--   job_dropbox_uploads.status  internal queue state
--   weather_snapshots.source    table empty in production; no verified value set
--   reports.type                table empty in production; no verified value set

-- ── roles and statuses ────────────────────────────────────────────────────
ALTER TABLE users            ADD CONSTRAINT users_role_check
  CHECK (role IN ('MANAGER', 'SUPERVISOR', 'OFFICE'));

ALTER TABLE jobs             ADD CONSTRAINT jobs_status_check
  CHECK (status IN ('active', 'completed', 'archived', 'on_hold'));

ALTER TABLE job_tasks        ADD CONSTRAINT job_tasks_status_check
  CHECK (status IN ('pending', 'in_progress', 'completed'));

ALTER TABLE scheduler_maintenance_jobs ADD CONSTRAINT scheduler_maintenance_jobs_status_check
  CHECK (status IN ('active', 'completed', 'archived', 'on_hold'));

-- ── call forward (domain-rules R1, R2) ────────────────────────────────────
ALTER TABLE call_forward     ADD CONSTRAINT call_forward_item_type_check
  CHECK (item_type IN ('HEADER', 'STAGE_CLAIM', 'TASK'));

ALTER TABLE call_forward     ADD CONSTRAINT call_forward_status_check
  CHECK (status IN ('not_started', 'in_progress', 'completed', 'on_hold'));

-- R2: parent_id had NO foreign key in the legacy schema, so an orphaned or
-- cross-job parent was possible. Self-reference plus a same-job guard is
-- enforced in the application (a CHECK cannot see another row).
ALTER TABLE call_forward     ADD CONSTRAINT call_forward_parent_fk
  FOREIGN KEY (parent_id) REFERENCES call_forward (id) ON DELETE CASCADE;

ALTER TABLE call_forward     ADD CONSTRAINT call_forward_no_self_parent
  CHECK (parent_id IS NULL OR parent_id <> id);

-- ── site diary ────────────────────────────────────────────────────────────
ALTER TABLE diary_notes      ADD CONSTRAINT diary_notes_category_check
  CHECK (category IN ('general', 'client', 'trades', 'site_conditions',
                      'issues', 'safety', 'materials', 'eto'));

ALTER TABLE diary_notes      ADD CONSTRAINT diary_notes_action_status_check
  CHECK (action_status IS NULL OR action_status IN ('action', 'processing', 'completed'));

ALTER TABLE site_diary       ADD CONSTRAINT site_diary_action_status_check
  CHECK (action_status IS NULL OR action_status IN ('action', 'processing', 'completed'));

-- ── media ─────────────────────────────────────────────────────────────────
ALTER TABLE diary_media      ADD CONSTRAINT diary_media_file_type_check
  CHECK (file_type IN ('photo', 'video', 'document'));

ALTER TABLE job_media        ADD CONSTRAINT job_media_file_type_check
  CHECK (file_type IN ('photo', 'video', 'document'));

ALTER TABLE diary_media      ADD CONSTRAINT diary_media_size_check
  CHECK (file_size >= 0);

ALTER TABLE job_media        ADD CONSTRAINT job_media_size_check
  CHECK (file_size >= 0);

-- ── branding (legacy values 'card' and 'manual' were retired and remapped
--    during the Supabase migration -- see docs/supabase-replit-data-migration.md)
ALTER TABLE company_branding ADD CONSTRAINT company_branding_job_display_mode_check
  CHECK (job_display_mode IN ('job_number', 'job_address'));

ALTER TABLE company_branding ADD CONSTRAINT company_branding_email_send_mode_check
  CHECK (email_send_mode IN ('device', 'smtp'));

-- ── scheduler (domain-rules R12) ──────────────────────────────────────────
-- NOTHING TO ADD. The legacy schema already enforces job XOR maintenance_job
-- via `scheduler_allocation_target_check`, and overlapping absences via the
-- `enforce_scheduler_worker_absence_no_overlap` trigger. Both are carried over
-- unchanged by 0001. An earlier draft of domain-rules.md R12 wrongly claimed
-- exclusivity was unenforced; it is.

-- ── progress ──────────────────────────────────────────────────────────────
ALTER TABLE progress         ADD CONSTRAINT progress_percent_range_check
  CHECK (percent_complete >= 0 AND percent_complete <= 100);

-- ── job assignments (domain-rules R4) ─────────────────────────────────────
-- Legacy `setPrimary()` performs three un-transactioned writes and can leave a
-- job with two primaries or none. This makes two primaries impossible.
CREATE UNIQUE INDEX IF NOT EXISTS uq_job_assignments_single_primary
  ON job_assignments (job_id) WHERE is_primary;

-- A user may be assigned to a job only once.
CREATE UNIQUE INDEX IF NOT EXISTS uq_job_assignments_job_user
  ON job_assignments (job_id, user_id);

-- ── date sanity ───────────────────────────────────────────────────────────
ALTER TABLE jobs             ADD CONSTRAINT jobs_date_order_check
  CHECK (start_date IS NULL OR end_date IS NULL OR start_date <= end_date);
