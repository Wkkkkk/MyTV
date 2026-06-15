-- Extend the channel type CHECK constraint to include 'vod_on_demand'.
-- SQLite does not support ALTER COLUMN, so we recreate the table.
PRAGMA foreign_keys = OFF;

CREATE TABLE channels_new (
    id          INTEGER PRIMARY KEY AUTOINCREMENT,
    name        TEXT    NOT NULL,
    category    TEXT    NOT NULL,
    logo_url    TEXT,
    type        TEXT    NOT NULL CHECK(type IN ('live', 'vod_loop', 'vod_on_demand')),
    sort_order  INTEGER NOT NULL DEFAULT 0,
    loop_anchor DATETIME
);

INSERT INTO channels_new SELECT id, name, category, logo_url, type, sort_order, loop_anchor FROM channels;

DROP TABLE channels;

ALTER TABLE channels_new RENAME TO channels;

PRAGMA foreign_keys = ON;
