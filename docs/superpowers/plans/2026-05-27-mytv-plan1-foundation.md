# MyTV Plan 1: Foundation Implementation Plan

> **For agentic workers:** REQUIRED SUB-SKILL: Use superpowers:subagent-driven-development (recommended) or superpowers:executing-plans to implement this plan task-by-task. Steps use checkbox (`- [ ]`) syntax for tracking.

**Goal:** Set up the Axum/SQLite foundation — project structure, database schema, data models, CRUD queries, and a running server with a health check endpoint.

**Architecture:** Single Rust binary using Axum for HTTP and sqlx for async SQLite queries. Migrations run automatically on startup. Each entity (channel, source, playlist_item) lives in its own focused module containing both its model struct and its database query functions. The VOD loop position calculator also lives in `playlist_item` since it is pure logic over that data.

**Tech Stack:** Rust, Axum 0.7, sqlx 0.7 (SQLite + migrate + chrono), tokio 1 (full), dotenvy, tracing + tracing-subscriber, anyhow, serde + serde_json

---

## File Structure

```
MyTV/
├── Cargo.toml
├── .env.example
├── migrations/
│   └── 001_initial.sql          - all three table definitions
└── src/
    ├── main.rs                  - server startup, AppState, route wiring
    ├── config.rs                - Config struct loaded from env vars
    ├── db.rs                    - pool creation + migration runner
    ├── channel.rs               - Channel struct + CRUD queries + distinct_categories
    ├── source.rs                - Source struct + CRUD queries
    ├── playlist_item.rs         - PlaylistItem struct + CRUD queries + current_position
    └── routes/
        ├── mod.rs               - declares sub-modules
        └── health.rs            - GET /health → {"status":"ok"}
```

---

## Task 1: Initialize Rust Project with Dependencies

**Files:**
- Create: `Cargo.toml`
- Create: `.env.example`

- [ ] **Step 1: Scaffold a new Rust binary project**

```bash
cargo new MyTV --name mytv
cd MyTV
```

Expected output: `Created binary (application) 'MyTV' package`

- [ ] **Step 2: Replace Cargo.toml with project dependencies**

```toml
[package]
name = "mytv"
version = "0.1.0"
edition = "2021"

[dependencies]
axum = "0.7"
tokio = { version = "1", features = ["full"] }
sqlx = { version = "0.7", features = ["runtime-tokio-rustls", "sqlite", "migrate", "chrono"] }
tower-http = { version = "0.5", features = ["trace"] }
serde = { version = "1", features = ["derive"] }
serde_json = "1"
chrono = { version = "0.4", features = ["serde"] }
dotenvy = "0.15"
tracing = "0.1"
tracing-subscriber = { version = "0.3", features = ["env-filter"] }
anyhow = "1"
```

- [ ] **Step 3: Create .env.example**

```
DATABASE_URL=sqlite:mytv.db
ADMIN_PASSWORD=changeme
YOUTUBE_API_KEY=
PORT=3000
RUST_LOG=info
```

- [ ] **Step 4: Verify the project compiles**

```bash
cargo build
```

Expected: compiles without errors (first run downloads crates — may take a minute)

- [ ] **Step 5: Commit**

```bash
git init
git add Cargo.toml Cargo.lock .env.example
git commit -m "feat: initialize mytv rust project with dependencies"
```

---

## Task 2: Config Module

**Files:**
- Create: `src/config.rs`
- Modify: `src/main.rs`

- [ ] **Step 1: Write the failing tests in src/config.rs**

```rust
pub struct Config {
    pub database_url: String,
    pub admin_password: String,
    pub youtube_api_key: Option<String>,
    pub port: u16,
}

impl Config {
    pub fn from_env() -> anyhow::Result<Self> {
        Ok(Config {
            database_url: std::env::var("DATABASE_URL")
                .unwrap_or_else(|_| "sqlite:mytv.db".to_string()),
            admin_password: std::env::var("ADMIN_PASSWORD")
                .unwrap_or_else(|_| "admin".to_string()),
            youtube_api_key: std::env::var("YOUTUBE_API_KEY").ok(),
            port: std::env::var("PORT")
                .unwrap_or_else(|_| "3000".to_string())
                .parse()?,
        })
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn test_config_defaults() {
        std::env::remove_var("DATABASE_URL");
        std::env::remove_var("PORT");
        std::env::remove_var("YOUTUBE_API_KEY");

        let config = Config::from_env().unwrap();

        assert_eq!(config.database_url, "sqlite:mytv.db");
        assert_eq!(config.port, 3000);
        assert!(config.youtube_api_key.is_none());
    }

    #[test]
    fn test_config_reads_env_vars() {
        std::env::set_var("DATABASE_URL", "sqlite:test.db");
        std::env::set_var("PORT", "8080");
        std::env::set_var("YOUTUBE_API_KEY", "abc123");

        let config = Config::from_env().unwrap();

        assert_eq!(config.database_url, "sqlite:test.db");
        assert_eq!(config.port, 8080);
        assert_eq!(config.youtube_api_key, Some("abc123".to_string()));

        std::env::remove_var("DATABASE_URL");
        std::env::remove_var("PORT");
        std::env::remove_var("YOUTUBE_API_KEY");
    }
}
```

- [ ] **Step 2: Run tests to confirm they fail with a compile error**

```bash
cargo test config
```

Expected: `error[E0433]: failed to resolve: use of undeclared crate or module 'config'`

- [ ] **Step 3: Declare the module in src/main.rs**

Replace `src/main.rs` with:

```rust
mod config;

fn main() {
    println!("Hello, MyTV!");
}
```

- [ ] **Step 4: Run tests to confirm they pass**

```bash
cargo test config
```

Expected:
```
test config::tests::test_config_defaults ... ok
test config::tests::test_config_reads_env_vars ... ok
```

- [ ] **Step 5: Commit**

```bash
git add src/config.rs src/main.rs
git commit -m "feat: add config module with env var loading"
```

---

## Task 3: Database Setup

**Files:**
- Create: `migrations/001_initial.sql`
- Create: `src/db.rs`
- Modify: `src/main.rs`

- [ ] **Step 1: Create the migration file**

Create `migrations/001_initial.sql`:

```sql
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
```

- [ ] **Step 2: Write the failing test in src/db.rs**

```rust
use anyhow::Result;
use sqlx::sqlite::{SqliteConnectOptions, SqlitePoolOptions};
use sqlx::SqlitePool;
use std::str::FromStr;

pub async fn connect(database_url: &str) -> Result<SqlitePool> {
    let opts = SqliteConnectOptions::from_str(database_url)?
        .foreign_keys(true)
        .create_if_missing(true);

    let pool = SqlitePoolOptions::new()
        .connect_with(opts)
        .await?;

    sqlx::migrate!("./migrations").run(&pool).await?;
    Ok(pool)
}

#[cfg(test)]
mod tests {
    use super::*;

    #[tokio::test]
    async fn test_migrations_create_all_tables() {
        let pool = connect("sqlite::memory:").await.unwrap();

        for table in &["channels", "sources", "playlist_items"] {
            let row: Option<(String,)> = sqlx::query_as(
                "SELECT name FROM sqlite_master WHERE type='table' AND name=?",
            )
            .bind(table)
            .fetch_optional(&pool)
            .await
            .unwrap();

            assert!(row.is_some(), "table '{}' should exist after migration", table);
        }
    }
}
```

- [ ] **Step 3: Run test to confirm it fails with a compile error**

```bash
cargo test db
```

Expected: `error[E0433]: failed to resolve: use of undeclared crate or module 'db'`

- [ ] **Step 4: Declare the module in src/main.rs**

```rust
mod config;
mod db;

fn main() {
    println!("Hello, MyTV!");
}
```

- [ ] **Step 5: Run tests to confirm they pass**

```bash
cargo test db
```

Expected: `test db::tests::test_migrations_create_all_tables ... ok`

- [ ] **Step 6: Commit**

```bash
git add migrations/001_initial.sql src/db.rs src/main.rs
git commit -m "feat: add sqlite database setup with migrations"
```

---

## Task 4: Channel Model and Queries

**Files:**
- Create: `src/channel.rs`
- Modify: `src/main.rs`

- [ ] **Step 1: Write the failing tests in src/channel.rs**

```rust
use anyhow::Result;
use chrono::{DateTime, Utc};
use serde::{Deserialize, Serialize};
use sqlx::{FromRow, SqlitePool};

#[derive(Debug, Clone, Serialize, Deserialize, FromRow)]
pub struct Channel {
    pub id: i64,
    pub name: String,
    pub category: String,
    pub logo_url: Option<String>,
    pub r#type: String,
    pub sort_order: i64,
    pub loop_anchor: Option<DateTime<Utc>>,
}

#[derive(Debug, Clone, PartialEq)]
pub enum ChannelType {
    Live,
    VodLoop,
}

impl Channel {
    pub fn channel_type(&self) -> ChannelType {
        match self.r#type.as_str() {
            "vod_loop" => ChannelType::VodLoop,
            _ => ChannelType::Live,
        }
    }
}

pub struct NewChannel {
    pub name: String,
    pub category: String,
    pub logo_url: Option<String>,
    pub channel_type: String,
    pub sort_order: i64,
    pub loop_anchor: Option<DateTime<Utc>>,
}

pub async fn create(pool: &SqlitePool, input: NewChannel) -> Result<Channel> {
    let id = sqlx::query(
        "INSERT INTO channels (name, category, logo_url, type, sort_order, loop_anchor)
         VALUES (?, ?, ?, ?, ?, ?)",
    )
    .bind(&input.name)
    .bind(&input.category)
    .bind(&input.logo_url)
    .bind(&input.channel_type)
    .bind(input.sort_order)
    .bind(input.loop_anchor)
    .execute(pool)
    .await?
    .last_insert_rowid();

    get(pool, id)
        .await?
        .ok_or_else(|| anyhow::anyhow!("channel not found after insert"))
}

pub async fn get(pool: &SqlitePool, id: i64) -> Result<Option<Channel>> {
    sqlx::query_as::<_, Channel>("SELECT * FROM channels WHERE id = ?")
        .bind(id)
        .fetch_optional(pool)
        .await
        .map_err(Into::into)
}

pub async fn list(pool: &SqlitePool) -> Result<Vec<Channel>> {
    sqlx::query_as::<_, Channel>(
        "SELECT * FROM channels ORDER BY sort_order ASC, name ASC",
    )
    .fetch_all(pool)
    .await
    .map_err(Into::into)
}

pub async fn list_by_category(pool: &SqlitePool, category: &str) -> Result<Vec<Channel>> {
    sqlx::query_as::<_, Channel>(
        "SELECT * FROM channels WHERE category = ? ORDER BY sort_order ASC, name ASC",
    )
    .bind(category)
    .fetch_all(pool)
    .await
    .map_err(Into::into)
}

pub async fn delete(pool: &SqlitePool, id: i64) -> Result<bool> {
    let rows = sqlx::query("DELETE FROM channels WHERE id = ?")
        .bind(id)
        .execute(pool)
        .await?
        .rows_affected();
    Ok(rows > 0)
}

/// Returns a sorted, deduplicated list of category names from a channel slice.
pub fn distinct_categories(channels: &[Channel]) -> Vec<String> {
    let mut cats: Vec<String> = channels
        .iter()
        .map(|c| c.category.clone())
        .collect::<std::collections::HashSet<_>>()
        .into_iter()
        .collect();
    cats.sort();
    cats
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::db;

    async fn test_pool() -> SqlitePool {
        db::connect("sqlite::memory:").await.unwrap()
    }

    fn live(name: &str, category: &str) -> NewChannel {
        NewChannel {
            name: name.to_string(),
            category: category.to_string(),
            logo_url: None,
            channel_type: "live".to_string(),
            sort_order: 0,
            loop_anchor: None,
        }
    }

    #[tokio::test]
    async fn test_create_and_get_channel() {
        let pool = test_pool().await;

        let ch = create(&pool, live("CNN International", "news")).await.unwrap();

        assert_eq!(ch.name, "CNN International");
        assert_eq!(ch.category, "news");
        assert_eq!(ch.channel_type(), ChannelType::Live);

        let fetched = get(&pool, ch.id).await.unwrap().unwrap();
        assert_eq!(fetched.id, ch.id);
    }

    #[tokio::test]
    async fn test_list_returns_all_channels() {
        let pool = test_pool().await;
        create(&pool, live("CNN", "news")).await.unwrap();
        create(&pool, live("ESPN", "sports")).await.unwrap();
        create(&pool, live("BBC", "news")).await.unwrap();

        let all = list(&pool).await.unwrap();
        assert_eq!(all.len(), 3);
    }

    #[tokio::test]
    async fn test_list_by_category_filters_correctly() {
        let pool = test_pool().await;
        create(&pool, live("CNN", "news")).await.unwrap();
        create(&pool, live("ESPN", "sports")).await.unwrap();
        create(&pool, live("BBC", "news")).await.unwrap();

        let news = list_by_category(&pool, "news").await.unwrap();
        assert_eq!(news.len(), 2);
        assert!(news.iter().all(|c| c.category == "news"));
    }

    #[tokio::test]
    async fn test_delete_channel() {
        let pool = test_pool().await;
        let ch = create(&pool, live("TMP", "test")).await.unwrap();

        assert!(delete(&pool, ch.id).await.unwrap());
        assert!(get(&pool, ch.id).await.unwrap().is_none());
    }

    #[tokio::test]
    async fn test_distinct_categories_sorted_deduped() {
        let pool = test_pool().await;
        create(&pool, live("CNN", "news")).await.unwrap();
        create(&pool, live("ESPN", "sports")).await.unwrap();
        create(&pool, live("BBC", "news")).await.unwrap();

        let all = list(&pool).await.unwrap();
        let cats = distinct_categories(&all);
        assert_eq!(cats, vec!["news", "sports"]);
    }
}
```

- [ ] **Step 2: Run tests to confirm they fail with a compile error**

```bash
cargo test channel
```

Expected: `error[E0433]: failed to resolve: use of undeclared crate or module 'channel'`

- [ ] **Step 3: Declare the module in src/main.rs**

```rust
mod channel;
mod config;
mod db;

fn main() {
    println!("Hello, MyTV!");
}
```

- [ ] **Step 4: Run tests to confirm they pass**

```bash
cargo test channel
```

Expected: all 5 channel tests pass

- [ ] **Step 5: Commit**

```bash
git add src/channel.rs src/main.rs
git commit -m "feat: add channel model and crud queries"
```

---

## Task 5: Source Model and Queries

**Files:**
- Create: `src/source.rs`
- Modify: `src/main.rs`

- [ ] **Step 1: Write the failing tests in src/source.rs**

```rust
use anyhow::Result;
use serde::{Deserialize, Serialize};
use sqlx::{FromRow, SqlitePool};

#[derive(Debug, Clone, Serialize, Deserialize, FromRow)]
pub struct Source {
    pub id: i64,
    pub channel_id: i64,
    pub kind: String,
    pub url: String,
    pub priority: i64,
    pub is_active: bool,
}

pub struct NewSource {
    pub channel_id: i64,
    pub kind: String,
    pub url: String,
    pub priority: i64,
}

pub async fn create(pool: &SqlitePool, input: NewSource) -> Result<Source> {
    let id = sqlx::query(
        "INSERT INTO sources (channel_id, kind, url, priority, is_active) VALUES (?, ?, ?, ?, 1)",
    )
    .bind(input.channel_id)
    .bind(&input.kind)
    .bind(&input.url)
    .bind(input.priority)
    .execute(pool)
    .await?
    .last_insert_rowid();

    sqlx::query_as::<_, Source>("SELECT * FROM sources WHERE id = ?")
        .bind(id)
        .fetch_one(pool)
        .await
        .map_err(Into::into)
}

pub async fn list_for_channel(pool: &SqlitePool, channel_id: i64) -> Result<Vec<Source>> {
    sqlx::query_as::<_, Source>(
        "SELECT * FROM sources WHERE channel_id = ? ORDER BY priority ASC",
    )
    .bind(channel_id)
    .fetch_all(pool)
    .await
    .map_err(Into::into)
}

pub async fn list_active_for_channel(pool: &SqlitePool, channel_id: i64) -> Result<Vec<Source>> {
    sqlx::query_as::<_, Source>(
        "SELECT * FROM sources WHERE channel_id = ? AND is_active = 1 ORDER BY priority ASC",
    )
    .bind(channel_id)
    .fetch_all(pool)
    .await
    .map_err(Into::into)
}

pub async fn delete(pool: &SqlitePool, id: i64) -> Result<bool> {
    let rows = sqlx::query("DELETE FROM sources WHERE id = ?")
        .bind(id)
        .execute(pool)
        .await?
        .rows_affected();
    Ok(rows > 0)
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::{channel, db};

    async fn test_pool() -> SqlitePool {
        db::connect("sqlite::memory:").await.unwrap()
    }

    async fn make_channel(pool: &SqlitePool) -> channel::Channel {
        channel::create(
            pool,
            channel::NewChannel {
                name: "Test".to_string(),
                category: "test".to_string(),
                logo_url: None,
                channel_type: "live".to_string(),
                sort_order: 0,
                loop_anchor: None,
            },
        )
        .await
        .unwrap()
    }

    fn hls(channel_id: i64, url: &str, priority: i64) -> NewSource {
        NewSource {
            channel_id,
            kind: "hls".to_string(),
            url: url.to_string(),
            priority,
        }
    }

    #[tokio::test]
    async fn test_create_and_list_sources_ordered_by_priority() {
        let pool = test_pool().await;
        let ch = make_channel(&pool).await;

        create(&pool, hls(ch.id, "https://primary.example.com/stream.m3u8", 1)).await.unwrap();
        create(&pool, hls(ch.id, "https://backup.example.com/stream.m3u8", 2)).await.unwrap();

        let sources = list_for_channel(&pool, ch.id).await.unwrap();
        assert_eq!(sources.len(), 2);
        assert_eq!(sources[0].priority, 1);
        assert_eq!(sources[1].priority, 2);
    }

    #[tokio::test]
    async fn test_list_active_excludes_inactive_sources() {
        let pool = test_pool().await;
        let ch = make_channel(&pool).await;

        let primary = create(&pool, hls(ch.id, "https://primary.example.com/stream.m3u8", 1)).await.unwrap();
        create(&pool, hls(ch.id, "https://backup.example.com/stream.m3u8", 2)).await.unwrap();

        sqlx::query("UPDATE sources SET is_active = 0 WHERE id = ?")
            .bind(primary.id)
            .execute(&pool)
            .await
            .unwrap();

        let active = list_active_for_channel(&pool, ch.id).await.unwrap();
        assert_eq!(active.len(), 1);
        assert_eq!(active[0].url, "https://backup.example.com/stream.m3u8");
    }

    #[tokio::test]
    async fn test_sources_deleted_when_channel_is_deleted() {
        let pool = test_pool().await;
        let ch = make_channel(&pool).await;

        create(&pool, hls(ch.id, "https://primary.example.com/stream.m3u8", 1)).await.unwrap();

        channel::delete(&pool, ch.id).await.unwrap();

        let sources = list_for_channel(&pool, ch.id).await.unwrap();
        assert!(sources.is_empty(), "ON DELETE CASCADE should remove sources");
    }
}
```

- [ ] **Step 2: Run tests to confirm they fail with a compile error**

```bash
cargo test source
```

Expected: `error[E0433]: failed to resolve: use of undeclared crate or module 'source'`

- [ ] **Step 3: Declare the module in src/main.rs**

```rust
mod channel;
mod config;
mod db;
mod source;

fn main() {
    println!("Hello, MyTV!");
}
```

- [ ] **Step 4: Run tests to confirm they pass**

```bash
cargo test source
```

Expected: all 3 source tests pass

- [ ] **Step 5: Commit**

```bash
git add src/source.rs src/main.rs
git commit -m "feat: add source model and crud queries with failover ordering"
```

---

## Task 6: PlaylistItem Model, Queries, and Loop Position Calculator

**Files:**
- Create: `src/playlist_item.rs`
- Modify: `src/main.rs`

- [ ] **Step 1: Write the failing tests in src/playlist_item.rs**

```rust
use anyhow::Result;
use serde::{Deserialize, Serialize};
use sqlx::{FromRow, SqlitePool};

#[derive(Debug, Clone, Serialize, Deserialize, FromRow)]
pub struct PlaylistItem {
    pub id: i64,
    pub channel_id: i64,
    pub title: String,
    pub url: String,
    pub duration_secs: i64,
    pub sort_order: i64,
}

pub struct NewPlaylistItem {
    pub channel_id: i64,
    pub title: String,
    pub url: String,
    pub duration_secs: i64,
    pub sort_order: i64,
}

pub async fn create(pool: &SqlitePool, input: NewPlaylistItem) -> Result<PlaylistItem> {
    let id = sqlx::query(
        "INSERT INTO playlist_items (channel_id, title, url, duration_secs, sort_order)
         VALUES (?, ?, ?, ?, ?)",
    )
    .bind(input.channel_id)
    .bind(&input.title)
    .bind(&input.url)
    .bind(input.duration_secs)
    .bind(input.sort_order)
    .execute(pool)
    .await?
    .last_insert_rowid();

    sqlx::query_as::<_, PlaylistItem>("SELECT * FROM playlist_items WHERE id = ?")
        .bind(id)
        .fetch_one(pool)
        .await
        .map_err(Into::into)
}

pub async fn list_for_channel(pool: &SqlitePool, channel_id: i64) -> Result<Vec<PlaylistItem>> {
    sqlx::query_as::<_, PlaylistItem>(
        "SELECT * FROM playlist_items WHERE channel_id = ? ORDER BY sort_order ASC",
    )
    .bind(channel_id)
    .fetch_all(pool)
    .await
    .map_err(Into::into)
}

pub async fn delete(pool: &SqlitePool, id: i64) -> Result<bool> {
    let rows = sqlx::query("DELETE FROM playlist_items WHERE id = ?")
        .bind(id)
        .execute(pool)
        .await?
        .rows_affected();
    Ok(rows > 0)
}

pub fn total_duration_secs(items: &[PlaylistItem]) -> i64 {
    items.iter().map(|i| i.duration_secs).sum()
}

/// Given a playlist and unix timestamps (seconds), returns the index of the
/// currently playing item and the playback offset in seconds within that item.
/// Returns None if the playlist is empty.
pub fn current_position(
    items: &[PlaylistItem],
    now_secs: i64,
    anchor_secs: i64,
) -> Option<(usize, i64)> {
    if items.is_empty() {
        return None;
    }
    let total = total_duration_secs(items);
    let elapsed = (now_secs - anchor_secs).rem_euclid(total);
    let mut acc = 0i64;
    for (i, item) in items.iter().enumerate() {
        acc += item.duration_secs;
        if elapsed < acc {
            let offset = elapsed - (acc - item.duration_secs);
            return Some((i, offset));
        }
    }
    Some((items.len() - 1, items.last().unwrap().duration_secs))
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::{channel, db};

    async fn test_pool() -> SqlitePool {
        db::connect("sqlite::memory:").await.unwrap()
    }

    async fn make_channel(pool: &SqlitePool) -> channel::Channel {
        channel::create(
            pool,
            channel::NewChannel {
                name: "VOD Loop".to_string(),
                category: "test".to_string(),
                logo_url: None,
                channel_type: "vod_loop".to_string(),
                sort_order: 0,
                loop_anchor: None,
            },
        )
        .await
        .unwrap()
    }

    fn item(channel_id: i64, title: &str, duration_secs: i64, sort_order: i64) -> NewPlaylistItem {
        NewPlaylistItem {
            channel_id,
            title: title.to_string(),
            url: format!("https://example.com/{}.mp4", title),
            duration_secs,
            sort_order,
        }
    }

    #[tokio::test]
    async fn test_create_and_list_playlist_items_in_order() {
        let pool = test_pool().await;
        let ch = make_channel(&pool).await;

        create(&pool, item(ch.id, "ep1", 1800, 0)).await.unwrap();
        create(&pool, item(ch.id, "ep2", 2400, 1)).await.unwrap();

        let items = list_for_channel(&pool, ch.id).await.unwrap();
        assert_eq!(items.len(), 2);
        assert_eq!(items[0].title, "ep1");
        assert_eq!(items[1].title, "ep2");
        assert_eq!(total_duration_secs(&items), 4200);
    }

    #[tokio::test]
    async fn test_current_position_within_first_item() {
        let items = vec![
            PlaylistItem { id: 1, channel_id: 1, title: "A".into(), url: "u".into(), duration_secs: 3600, sort_order: 0 },
            PlaylistItem { id: 2, channel_id: 1, title: "B".into(), url: "u".into(), duration_secs: 1800, sort_order: 1 },
        ];
        // 500 seconds into the loop — still in item A
        let (idx, offset) = current_position(&items, 1500, 1000).unwrap();
        assert_eq!(idx, 0);
        assert_eq!(offset, 500);
    }

    #[tokio::test]
    async fn test_current_position_within_second_item() {
        let items = vec![
            PlaylistItem { id: 1, channel_id: 1, title: "A".into(), url: "u".into(), duration_secs: 3600, sort_order: 0 },
            PlaylistItem { id: 2, channel_id: 1, title: "B".into(), url: "u".into(), duration_secs: 1800, sort_order: 1 },
        ];
        // 4000 seconds in — 400 seconds into item B (after A's 3600)
        let (idx, offset) = current_position(&items, 4000, 0).unwrap();
        assert_eq!(idx, 1);
        assert_eq!(offset, 400);
    }

    #[tokio::test]
    async fn test_current_position_wraps_around_to_start() {
        let items = vec![
            PlaylistItem { id: 1, channel_id: 1, title: "A".into(), url: "u".into(), duration_secs: 3600, sort_order: 0 },
            PlaylistItem { id: 2, channel_id: 1, title: "B".into(), url: "u".into(), duration_secs: 1800, sort_order: 1 },
        ];
        // total = 5400; 5500 seconds in wraps to 100 seconds into item A
        let (idx, offset) = current_position(&items, 5500, 0).unwrap();
        assert_eq!(idx, 0);
        assert_eq!(offset, 100);
    }

    #[tokio::test]
    async fn test_current_position_empty_playlist_returns_none() {
        let result = current_position(&[], 1000, 0);
        assert!(result.is_none());
    }

    #[tokio::test]
    async fn test_playlist_items_deleted_when_channel_is_deleted() {
        let pool = test_pool().await;
        let ch = make_channel(&pool).await;

        create(&pool, item(ch.id, "ep1", 1800, 0)).await.unwrap();

        channel::delete(&pool, ch.id).await.unwrap();

        let items = list_for_channel(&pool, ch.id).await.unwrap();
        assert!(items.is_empty(), "ON DELETE CASCADE should remove playlist items");
    }
}
```

- [ ] **Step 2: Run tests to confirm they fail with a compile error**

```bash
cargo test playlist
```

Expected: `error[E0433]: failed to resolve: use of undeclared crate or module 'playlist_item'`

- [ ] **Step 3: Declare the module in src/main.rs**

```rust
mod channel;
mod config;
mod db;
mod playlist_item;
mod source;

fn main() {
    println!("Hello, MyTV!");
}
```

- [ ] **Step 4: Run tests to confirm they pass**

```bash
cargo test playlist
```

Expected: all 6 playlist_item tests pass

- [ ] **Step 5: Commit**

```bash
git add src/playlist_item.rs src/main.rs
git commit -m "feat: add playlist_item model, crud, and vod loop position calculator"
```

---

## Task 7: Axum Server with AppState and Health Endpoint

**Files:**
- Create: `src/routes/mod.rs`
- Create: `src/routes/health.rs`
- Modify: `src/main.rs`

- [ ] **Step 1: Create the health handler**

Create `src/routes/mod.rs`:

```rust
pub mod health;
```

Create `src/routes/health.rs`:

```rust
use axum::http::StatusCode;
use axum::response::Json;
use serde_json::{json, Value};

pub async fn health_check() -> (StatusCode, Json<Value>) {
    (StatusCode::OK, Json(json!({ "status": "ok" })))
}
```

- [ ] **Step 2: Wire up the full server in src/main.rs**

```rust
mod channel;
mod config;
mod db;
mod playlist_item;
mod routes;
mod source;

use anyhow::Result;
use axum::{routing::get, Router};
use sqlx::SqlitePool;
use std::sync::Arc;
use tracing_subscriber::EnvFilter;

#[derive(Clone)]
pub struct AppState {
    pub pool: SqlitePool,
    pub config: Arc<config::Config>,
}

#[tokio::main]
async fn main() -> Result<()> {
    dotenvy::dotenv().ok();
    tracing_subscriber::fmt()
        .with_env_filter(EnvFilter::from_default_env())
        .init();

    let config = Arc::new(config::Config::from_env()?);
    let pool = db::connect(&config.database_url).await?;

    let state = AppState {
        pool,
        config: config.clone(),
    };

    let app = Router::new()
        .route("/health", get(routes::health::health_check))
        .with_state(state);

    let addr = format!("0.0.0.0:{}", config.port);
    tracing::info!("listening on {addr}");
    let listener = tokio::net::TcpListener::bind(&addr).await?;
    axum::serve(listener, app).await?;

    Ok(())
}
```

- [ ] **Step 3: Create your local .env file**

```bash
cp .env.example .env
```

- [ ] **Step 4: Run the server**

```bash
cargo run
```

Expected output: `INFO mytv: listening on 0.0.0.0:3000`

- [ ] **Step 5: Verify the health endpoint in a second terminal**

```bash
curl http://localhost:3000/health
```

Expected: `{"status":"ok"}`

- [ ] **Step 6: Run the full test suite**

```bash
cargo test
```

Expected: all tests pass across all modules (channel, config, db, playlist_item, source)

- [ ] **Step 7: Commit**

```bash
git add src/routes/ src/main.rs .env.example
git commit -m "feat: add axum server with appstate and health check endpoint"
```

---

## Self-Review

**Spec coverage check:**

| Spec requirement | Covered in this plan |
|---|---|
| Rust + Axum tech stack | ✅ Task 1, 7 |
| SQLite via sqlx | ✅ Task 3 |
| `channels` table with all spec fields | ✅ Task 3, 4 |
| `sources` table with priority ordering | ✅ Task 3, 5 |
| `playlist_items` table with duration | ✅ Task 3, 6 |
| Foreign key cascade deletes | ✅ Task 5, 6 (tested) |
| VOD loop position calculator | ✅ Task 6 (`current_position`) |
| Config from env vars (all four vars) | ✅ Task 2 |
| foreign_keys = ON for SQLite | ✅ Task 3 (`SqliteConnectOptions`) |
| yt-dlp integration | ⬜ Plan 2 |
| Tune / next player endpoints | ⬜ Plan 2 |
| EPG schedule generation | ⬜ Plan 2 |
| Askama templates + HTMX guide grid | ⬜ Plan 3 |
| Admin CRUD UI | ⬜ Plan 4 |
| YouTube API + iptv-org discovery | ⬜ Plan 4 |

**Placeholder scan:** No TBDs, no vague steps, every code block is complete and runnable.

**Type consistency:**
- `NewChannel` defined in Task 4, referenced correctly in Task 5 and 6 test helpers
- `current_position` returns `Option<(usize, i64)>` — Plan 2 will call this with `chrono::Utc::now().timestamp()` and `channel.loop_anchor.timestamp()`
- `AppState` defined in Task 7 with `pool: SqlitePool` and `config: Arc<Config>` — all future route handlers will receive `State(AppState)` via Axum's extractor

---

## Next Plans

- **Plan 2:** EPG Engine + Player API — yt-dlp subprocess wrapper, stream URL resolution, `/channel/:id/tune` and `/channel/:id/next` endpoints, live channel failover logic, EPG schedule generation for 24h window
- **Plan 3:** EPG Grid UI + Player — Askama templates, HTMX category tabs and time navigation, hls.js player panel, mobile layout
- **Plan 4:** Admin UI + Discovery — Admin CRUD pages for channels/sources/playlists, YouTube Data API search, iptv-org M3U import, manual URL entry
