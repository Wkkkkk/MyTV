ALTER TABLE playlist_items ADD COLUMN is_active           INTEGER NOT NULL DEFAULT 1;
ALTER TABLE playlist_items ADD COLUMN last_checked_at     INTEGER;
ALTER TABLE playlist_items ADD COLUMN last_status         TEXT CHECK(last_status IN ('ok', 'error'));
ALTER TABLE playlist_items ADD COLUMN consecutive_failures INTEGER NOT NULL DEFAULT 0;
ALTER TABLE playlist_items ADD COLUMN failure_reason      TEXT;

CREATE INDEX idx_playlist_items_is_active_channel_sort
    ON playlist_items(is_active, channel_id, sort_order);
