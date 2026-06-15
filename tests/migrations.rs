//! Migration safety regression tests.
//!
//! The integration harness seeds data *after* running migrations on an empty DB,
//! so it cannot catch a migration that destroys pre-existing data. These tests
//! reproduce a production upgrade: rows exist *before* the migration runs.

use std::str::FromStr;

use sqlx::sqlite::SqliteConnectOptions;
use sqlx::{ConnectOptions, Connection, Executor};

/// Migration 007 recreates the `channels` table to widen its `type` CHECK
/// constraint. Because sqlx 0.7 wraps each migration in a transaction (where
/// `PRAGMA foreign_keys` is a no-op) and `db::connect` enables foreign keys,
/// a naive `DROP TABLE channels` would cascade-delete every `sources` and
/// `playlist_items` row. This test fails against that naive version and passes
/// against the snapshot-and-restore migration.
#[tokio::test]
async fn migration_007_preserves_child_rows() {
    let mut conn = SqliteConnectOptions::from_str("sqlite::memory:")
        .unwrap()
        .foreign_keys(true)
        .connect()
        .await
        .unwrap();

    // Pre-007 schema (old CHECK constraint) with cascading child tables + data.
    conn.execute(
        "CREATE TABLE channels (
             id INTEGER PRIMARY KEY AUTOINCREMENT, name TEXT NOT NULL,
             category TEXT NOT NULL, logo_url TEXT,
             type TEXT NOT NULL CHECK(type IN ('live','vod_loop')),
             sort_order INTEGER NOT NULL DEFAULT 0, loop_anchor DATETIME);
         CREATE TABLE sources (
             id INTEGER PRIMARY KEY AUTOINCREMENT,
             channel_id INTEGER NOT NULL REFERENCES channels(id) ON DELETE CASCADE,
             url TEXT NOT NULL);
         CREATE TABLE playlist_items (
             id INTEGER PRIMARY KEY AUTOINCREMENT,
             channel_id INTEGER NOT NULL REFERENCES channels(id) ON DELETE CASCADE,
             title TEXT NOT NULL);
         INSERT INTO channels (id,name,category,type) VALUES (1,'C','cat','live');
         INSERT INTO sources (channel_id,url) VALUES (1,'u');
         INSERT INTO playlist_items (channel_id,title) VALUES (1,'t');",
    )
    .await
    .unwrap();

    // Run migration 007 inside a transaction, exactly as the sqlx 0.7 runner does.
    let sql = include_str!("../migrations/007_channel_vod_on_demand.sql");
    let mut tx = conn.begin().await.unwrap();
    tx.execute(sql).await.unwrap();
    tx.commit().await.unwrap();

    let (sources,): (i64,) = sqlx::query_as("SELECT COUNT(*) FROM sources")
        .fetch_one(&mut conn)
        .await
        .unwrap();
    let (items,): (i64,) = sqlx::query_as("SELECT COUNT(*) FROM playlist_items")
        .fetch_one(&mut conn)
        .await
        .unwrap();
    assert_eq!(sources, 1, "migration 007 must not delete sources rows");
    assert_eq!(
        items, 1,
        "migration 007 must not delete playlist_items rows"
    );

    // The widened constraint must accept the new channel type.
    sqlx::query("INSERT INTO channels (name,category,type) VALUES ('OD','cat','vod_on_demand')")
        .execute(&mut conn)
        .await
        .unwrap();
}
