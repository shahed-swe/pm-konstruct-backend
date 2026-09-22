-- One weather snapshot per diary entry.
--
-- The legacy service upserts with `onConflictDoUpdate({ target: diaryEntryId })`,
-- which Postgres can only satisfy against a unique constraint or index. There
-- is only a plain btree on the column, so that statement would have failed
-- with "no unique or exclusion constraint matching the ON CONFLICT
-- specification" the first time it ran.
--
-- It never ran: `weather_snapshots` holds 0 rows in production, like the rest
-- of the structured-weather feature. The constraint is what the code always
-- intended, and without it the upsert cannot be written at all.
--
-- The plain index becomes redundant once the unique one exists -- a unique
-- constraint is backed by an index that serves the same lookups -- so it is
-- dropped rather than left to be maintained on every write.

DELETE FROM weather_snapshots a
USING weather_snapshots b
WHERE a.diary_entry_id = b.diary_entry_id
  AND a.id < b.id;

ALTER TABLE weather_snapshots
  ADD CONSTRAINT weather_snapshots_entry_unique UNIQUE (diary_entry_id);

DROP INDEX IF EXISTS idx_weather_snapshots_entry;
