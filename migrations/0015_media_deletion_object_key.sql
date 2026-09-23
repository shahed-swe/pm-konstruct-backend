-- PM Konstruct v2 -- the deletion queue must hold the object key, not the filename
--
-- The bug this fixes: `media_deletion_queue.stored_name` held the bare
-- `stored_name` from `job_media`/`diary_media` -- a UUID plus extension --
-- while the object in the bucket lives at
--
--     {company_id}/{job|diary}/{entity_id}/{stored_name}
--
-- (`pmk_domain::media::object_key`). The worker therefore asked storage to
-- delete a key that had never existed. S3 and MinIO both answer a delete of a
-- missing key with success, so the worker logged `swept=1`, removed the queue
-- row, and the object stayed in the bucket.
--
-- The effect was that **no deleted photo was ever removed from storage** and
-- nothing reported a problem: the queue drained, the logs were clean, and the
-- bucket grew for ever with objects no row referenced. Found by deleting a
-- photo through the API and then listing the bucket.
--
-- Renaming the column rather than adding one, so the old name cannot be read
-- by mistake, and so any code still writing a bare filename fails to compile
-- or fails loudly at runtime instead of queueing another key that does not
-- exist.

ALTER TABLE media_deletion_queue RENAME COLUMN stored_name TO object_key;

ALTER INDEX uq_media_deletion_queue_stored_name
  RENAME TO uq_media_deletion_queue_object_key;

COMMENT ON COLUMN media_deletion_queue.object_key IS
  'Full key in the bucket, as pmk_domain::media::object_key builds it: {company_id}/{entity}/{entity_id}/{stored_name}. Never the bare stored_name -- see migration 0015.';

-- Anything queued before this migration holds a bare filename and names no
-- real object, so the worker would retry it for ever against a key that does
-- not exist. There is nothing to recover from those rows: the object key they
-- should have carried is not derivable from the row, because the media row
-- that knew the job or entry has already been deleted.
--
-- The objects themselves are orphaned in the bucket and have to be reconciled
-- against `job_media`/`diary_media` by hand -- `deploy/README.md` has the
-- `mc` command. Clearing the queue is the honest state: nothing pending,
-- rather than a queue that silently never drains.
DELETE FROM media_deletion_queue;
