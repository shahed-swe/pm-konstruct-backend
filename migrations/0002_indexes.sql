-- PM Konstruct v2 -- performance indexes
--
-- The legacy schema has 8 explicit indexes across 38 tables and 60 foreign keys
-- (Analysis 4.4). None of the hot paths are covered. At 32 jobs that is
-- invisible; at 300 companies every dashboard load is a sequential scan.
--
-- Rationale per index is recorded in docs/db/index-rationale.md, which is
-- generated from EXPLAIN ANALYZE output by tools/phase3/explain_report.py.
--
-- Postgres does NOT auto-index the referencing side of a foreign key, only the
-- referenced side. Every FK used in a join or cascade delete therefore needs an
-- explicit index.

-- ── tenancy: every list query filters on company_id ────────────────────────
CREATE INDEX IF NOT EXISTS idx_jobs_company_status      ON jobs (company_id, status);
CREATE INDEX IF NOT EXISTS idx_jobs_company_created     ON jobs (company_id, created_at DESC);
CREATE INDEX IF NOT EXISTS idx_users_company            ON users (company_id) WHERE active;
CREATE INDEX IF NOT EXISTS idx_jobs_manager             ON jobs (manager_id) WHERE manager_id IS NOT NULL;
CREATE INDEX IF NOT EXISTS idx_jobs_supervisor          ON jobs (supervisor_id) WHERE supervisor_id IS NOT NULL;

-- ── job visibility (domain-rules R3): assignments UNION primary supervisor ──
CREATE INDEX IF NOT EXISTS idx_job_assignments_user     ON job_assignments (user_id);
CREATE INDEX IF NOT EXISTS idx_job_assignments_job      ON job_assignments (job_id);

-- ── site diary: listed per job, newest first; notes/comments/media cascade ──
CREATE INDEX IF NOT EXISTS idx_site_diary_job_date      ON site_diary (job_id, date DESC);
CREATE INDEX IF NOT EXISTS idx_site_diary_author        ON site_diary (author_id) WHERE author_id IS NOT NULL;
CREATE INDEX IF NOT EXISTS idx_diary_notes_entry        ON diary_notes (diary_entry_id, sort_order)
  WHERE NOT archived;
CREATE INDEX IF NOT EXISTS idx_diary_notes_category     ON diary_notes (category, diary_entry_id);
CREATE INDEX IF NOT EXISTS idx_diary_note_comments_note ON diary_note_comments (note_id, created_at);
CREATE INDEX IF NOT EXISTS idx_diary_media_entry        ON diary_media (diary_entry_id);
CREATE INDEX IF NOT EXISTS idx_diary_media_note         ON diary_media (note_id) WHERE note_id IS NOT NULL;
CREATE INDEX IF NOT EXISTS idx_weather_snapshots_entry  ON weather_snapshots (diary_entry_id);

-- ── call forward: the heaviest table (1,687 rows), tree + delay queries ────
CREATE INDEX IF NOT EXISTS idx_call_forward_job_sort    ON call_forward (job_id, sort_order);
CREATE INDEX IF NOT EXISTS idx_call_forward_parent      ON call_forward (parent_id) WHERE parent_id IS NOT NULL;
-- Drives the delay engine and the dashboard overdue/upcoming widgets: only
-- incomplete items can be delayed, so the partial index stays small.
CREATE INDEX IF NOT EXISTS idx_call_forward_open_finish ON call_forward (job_id, est_finish)
  WHERE status <> 'completed' AND actual_finish IS NULL;
CREATE INDEX IF NOT EXISTS idx_call_forward_templates_company
  ON call_forward_templates (company_id);

-- ── notifications: 1,724 rows; the bell polls unread per user ──────────────
CREATE INDEX IF NOT EXISTS idx_notifications_user_unread
  ON notifications (user_id, created_at DESC) WHERE read_at IS NULL;
CREATE INDEX IF NOT EXISTS idx_notifications_user_all
  ON notifications (user_id, created_at DESC);

-- ── permissions: resolved on every authenticated request ──────────────────
CREATE INDEX IF NOT EXISTS idx_user_permissions_user    ON user_permissions (user_id);

-- ── media and tasks ───────────────────────────────────────────────────────
CREATE INDEX IF NOT EXISTS idx_job_tasks_job_sort       ON job_tasks (job_id, sort_order);
CREATE INDEX IF NOT EXISTS idx_progress_job_date        ON progress (job_id, date DESC);
CREATE INDEX IF NOT EXISTS idx_reports_job              ON reports (job_id, generated_at DESC);

-- ── scheduler: board query is workers x date-range ────────────────────────
CREATE INDEX IF NOT EXISTS idx_scheduler_workers_company
  ON scheduler_workers (company_id) WHERE active;
CREATE INDEX IF NOT EXISTS idx_scheduler_allocations_worker_date
  ON scheduler_allocations (worker_id, assigned_date);
CREATE INDEX IF NOT EXISTS idx_scheduler_allocations_job
  ON scheduler_allocations (job_id) WHERE job_id IS NOT NULL;
CREATE INDEX IF NOT EXISTS idx_scheduler_day_notes_company_date
  ON scheduler_job_day_notes (company_id, note_date);

-- ── forms ─────────────────────────────────────────────────────────────────
CREATE INDEX IF NOT EXISTS idx_inspection_forms_company_job
  ON inspection_forms (company_id, job_id);
CREATE INDEX IF NOT EXISTS idx_inspection_forms_diary_note
  ON inspection_forms (diary_note_id);

-- ── dropbox ───────────────────────────────────────────────────────────────
CREATE INDEX IF NOT EXISTS idx_job_dropbox_folders_job  ON job_dropbox_folders (job_id, sort_order);
CREATE INDEX IF NOT EXISTS idx_job_dropbox_uploads_job  ON job_dropbox_uploads (job_id, status);

-- ── auth ──────────────────────────────────────────────────────────────────
-- Reset-token lookup is by token (already unique). This covers cleanup of
-- expired rows and the "active token for user" check.
CREATE INDEX IF NOT EXISTS idx_password_reset_user
  ON password_reset_tokens (user_id, expires_at DESC) WHERE used_at IS NULL;
CREATE INDEX IF NOT EXISTS idx_push_subscriptions_user  ON push_subscriptions (user_id);
