-- SQLite cannot ALTER a CHECK constraint, so we recreate the sources table
-- with 'dash' added to the kind CHECK. All columns from 001+002 and indexes
-- from 003+004 are preserved.
CREATE TABLE sources_new (
    id                   INTEGER PRIMARY KEY AUTOINCREMENT,
    channel_id           INTEGER NOT NULL REFERENCES channels(id) ON DELETE CASCADE,
    kind                 TEXT    NOT NULL CHECK(kind IN ('youtube_live', 'hls', 'iptv', 'dash')),
    url                  TEXT    NOT NULL,
    priority             INTEGER NOT NULL DEFAULT 1,
    is_active            INTEGER NOT NULL DEFAULT 1,
    last_checked_at      INTEGER,
    last_status          TEXT    CHECK(last_status IN ('ok', 'error')),
    consecutive_failures INTEGER NOT NULL DEFAULT 0,
    failure_reason       TEXT
);

INSERT INTO sources_new SELECT * FROM sources;
DROP TABLE sources;
ALTER TABLE sources_new RENAME TO sources;

CREATE INDEX idx_sources_channel_priority ON sources(channel_id, priority);
CREATE INDEX idx_sources_is_active_channel_priority ON sources(is_active, channel_id, priority);
