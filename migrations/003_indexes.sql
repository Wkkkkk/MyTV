CREATE INDEX idx_sources_channel_priority ON sources(channel_id, priority);
CREATE INDEX idx_playlist_items_channel_sort ON playlist_items(channel_id, sort_order);
