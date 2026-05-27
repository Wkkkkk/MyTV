CREATE TABLE IF NOT EXISTS channels (
    id          INTEGER PRIMARY KEY AUTOINCREMENT,
    name        TEXT    NOT NULL,
    category    TEXT    NOT NULL,
    logo_url    TEXT,
    type        TEXT    NOT NULL CHECK(type IN ('live', 'vod_loop')),
    sort_order  INTEGER NOT NULL DEFAULT 0,
    loop_anchor DATETIME
);

CREATE TABLE IF NOT EXISTS sources (
    id          INTEGER PRIMARY KEY AUTOINCREMENT,
    channel_id  INTEGER NOT NULL REFERENCES channels(id) ON DELETE CASCADE,
    kind        TEXT    NOT NULL CHECK(kind IN ('youtube_live', 'hls', 'iptv')),
    url         TEXT    NOT NULL,
    priority    INTEGER NOT NULL DEFAULT 1,
    is_active   INTEGER NOT NULL DEFAULT 1
);

CREATE TABLE IF NOT EXISTS playlist_items (
    id            INTEGER PRIMARY KEY AUTOINCREMENT,
    channel_id    INTEGER NOT NULL REFERENCES channels(id) ON DELETE CASCADE,
    title         TEXT    NOT NULL,
    url           TEXT    NOT NULL,
    duration_secs INTEGER NOT NULL,
    sort_order    INTEGER NOT NULL DEFAULT 0
);
