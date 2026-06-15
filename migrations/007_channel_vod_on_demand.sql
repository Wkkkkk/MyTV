-- Extend the channels.type CHECK constraint to include 'vod_on_demand'.
-- SQLite cannot ALTER a CHECK constraint, so the channels table must be recreated.
--
-- Caveat: sqlx 0.7 runs every migration inside a transaction, and
-- `PRAGMA foreign_keys` is a no-op while a transaction is open. With foreign-key
-- enforcement on (db::connect sets `.foreign_keys(true)`), `DROP TABLE channels`
-- performs an implicit cascade DELETE on every `sources` and `playlist_items`
-- row (both reference channels ON DELETE CASCADE) — silently wiping production
-- data. The empty-DB test harness cannot catch it (it seeds AFTER migrating).
--
-- So we snapshot the child tables, rebuild channels, then restore the children.
-- This is non-destructive regardless of foreign-key/transaction state.

CREATE TEMP TABLE _sources_backup AS SELECT * FROM sources;
CREATE TEMP TABLE _playlist_items_backup AS SELECT * FROM playlist_items;

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

-- Restore children (the DELETE guards the case where the cascade did NOT fire,
-- e.g. foreign keys off — avoids duplicate rows on re-insert).
DELETE FROM sources;
INSERT INTO sources SELECT * FROM _sources_backup;
DELETE FROM playlist_items;
INSERT INTO playlist_items SELECT * FROM _playlist_items_backup;

DROP TABLE _sources_backup;
DROP TABLE _playlist_items_backup;
