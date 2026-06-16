ALTER TABLE playlist_items ADD COLUMN disabled_at INTEGER;

-- Backfill: start the reap clock at deploy time for rows already disabled, so the
-- first reaper pass never mass-deletes pre-existing disabled items.
UPDATE playlist_items SET disabled_at = strftime('%s','now') WHERE is_active = 0;
