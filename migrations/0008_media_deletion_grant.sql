-- PM Konstruct v2 -- let the app enqueue media deletions
--
-- Migration 0004 revoked all access to `media_deletion_queue` from `pmk_app`,
-- on the reasoning that the queue is cross-tenant: it keys on `stored_name`
-- alone and carries no company column, so the worker processes it under a
-- BYPASSRLS role.
--
-- That was too strict. Deleting a photo has to queue its object in the *same
-- transaction* as the row delete, otherwise a committed delete can leave the
-- object orphaned in storage. The API therefore needs INSERT here.
--
-- Caught by the Phase 7 integration test: DELETE /media/{id} returned 500 with
-- "permission denied for table media_deletion_queue".
--
-- INSERT only, deliberately. The application can enqueue but cannot read the
-- queue (so it cannot enumerate other tenants' object names), cannot mark
-- entries processed, and cannot clear them. Those remain the worker's.
--
-- Keeping it to INSERT constrains the query: `ON CONFLICT (stored_name)` makes
-- Postgres inspect the conflicting row and therefore needs SELECT, so the
-- repository uses the untargeted `ON CONFLICT DO NOTHING`, which does not.

GRANT INSERT ON media_deletion_queue TO pmk_app;
GRANT USAGE, SELECT ON SEQUENCE media_deletion_queue_id_seq TO pmk_app;

-- RLS stays enabled with no policy, which would normally deny the INSERT too.
-- A permissive INSERT-only policy keeps the deny on every other operation.
DROP POLICY IF EXISTS app_may_enqueue ON media_deletion_queue;
CREATE POLICY app_may_enqueue ON media_deletion_queue
  FOR INSERT TO pmk_app
  WITH CHECK (true);

COMMENT ON POLICY app_may_enqueue ON media_deletion_queue IS
  'The API enqueues an object when its row is deleted, in the same transaction. Reading and clearing the queue remain restricted to the worker.';
