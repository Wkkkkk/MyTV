# Playlist Item Health Tracking Implementation Plan

> **For agentic workers:** REQUIRED SUB-SKILL: Use superpowers:subagent-driven-development (recommended) or superpowers:executing-plans to implement this plan task-by-task. Steps use checkbox (`- [ ]`) syntax for tracking.

**Goal:** Add health tracking and CORS/budget badges to VOD playlist items so they behave like live sources — auto-disabled after repeated failures, auto-re-enabled on recovery, and visible in admin with health + budget columns.

**Architecture:** New migration adds five health columns to `playlist_items` (mirrors `sources`). The model layer, health checker, admin display types, and player are extended in lockstep. The player switches to `list_active_for_channel` so disabled items are skipped from the time-based loop.

**Tech Stack:** Rust, Axum 0.7, SQLx 0.7 (runtime queries via `query_as`, no macro), Askama 0.12 templates, SQLite.

---

## File Map

| File | Change |
|------|--------|
| `migrations/005_playlist_item_health.sql` | new — adds 5 health columns + index to `playlist_items` |
| `src/model/playlist_item.rs` | add fields to `PlaylistItem`, add `list_active_for_channel` / `set_active` / `update_health` |
| `src/health.rs` | refactor `do_http_check` signature, refactor `process_result` signature, remove `probe_all_playlist_cors`, add `check_playlist_item` + `probe_playlist_item`, update `check_all` |
| `src/routes/player.rs` | `vod_items_and_index` uses `list_active_for_channel` instead of `list_for_channel` |
| `src/routes/admin/mod.rs` | `AdminPlaylistItemRow` gains health fields, update `From<PlaylistItem>`, add re-export for `playlist_item_toggle` |
| `src/routes/admin/playlist.rs` | add `playlist_item_toggle` handler, update `playlist_item_test` to call `probe_playlist_item` |
| `src/lib.rs` | wire `POST /playlist/:id/toggle` route |
| `templates/admin/partials/playlist_item_row.html` | add health badge + active badge + toggle button |
| `templates/admin/channel_detail.html` | add `Health` column header to playlist table `<thead>` |
| `tests/http.rs` | add integration test `test_tune_vod_skips_disabled_item` |

---

## Task 1: Database migration

**Files:**
- Create: `migrations/005_playlist_item_health.sql`

- [ ] **Step 1: Create the migration file**

```sql
ALTER TABLE playlist_items ADD COLUMN is_active           INTEGER NOT NULL DEFAULT 1;
ALTER TABLE playlist_items ADD COLUMN last_checked_at     INTEGER;
ALTER TABLE playlist_items ADD COLUMN last_status         TEXT CHECK(last_status IN ('ok', 'error'));
ALTER TABLE playlist_items ADD COLUMN consecutive_failures INTEGER NOT NULL DEFAULT 0;
ALTER TABLE playlist_items ADD COLUMN failure_reason      TEXT;

CREATE INDEX idx_playlist_items_is_active_channel_sort
    ON playlist_items(is_active, channel_id, sort_order);
```

- [ ] **Step 2: Verify existing tests still pass**

Run: `cargo test`
Expected: all tests pass (new columns default to `is_active=1`, others null/0, so existing seed data is unchanged)

- [ ] **Step 3: Commit**

```bash
git add migrations/005_playlist_item_health.sql
git commit -m "feat: migration — add health columns to playlist_items"
```

---

## Task 2: Model layer — struct + DB functions

**Files:**
- Modify: `src/model/playlist_item.rs`

- [ ] **Step 1: Write failing tests**

Add to the `#[cfg(test)]` block in `src/model/playlist_item.rs`, after the existing `make_channel` and `item` helpers. The `get` function already exists in this module.

```rust
#[tokio::test]
async fn test_list_active_excludes_inactive_items() {
    let pool = test_pool().await;
    let ch = make_channel(&pool).await;

    let first = create(&pool, item(ch.id, "ep1", 1800, 0)).await.unwrap();
    create(&pool, item(ch.id, "ep2", 2400, 1)).await.unwrap();

    set_active(&pool, first.id, false).await.unwrap();

    let active = list_active_for_channel(&pool, ch.id).await.unwrap();
    assert_eq!(active.len(), 1);
    assert_eq!(active[0].title, "ep2");
}

#[tokio::test]
async fn test_set_active_toggles_item() {
    let pool = test_pool().await;
    let ch = make_channel(&pool).await;

    let it = create(&pool, item(ch.id, "ep1", 1800, 0)).await.unwrap();
    assert!(it.is_active);

    set_active(&pool, it.id, false).await.unwrap();
    assert!(list_active_for_channel(&pool, ch.id).await.unwrap().is_empty());

    set_active(&pool, it.id, true).await.unwrap();
    assert_eq!(list_active_for_channel(&pool, ch.id).await.unwrap().len(), 1);
}

#[tokio::test]
async fn test_update_health_ok_resets_failures() {
    let pool = test_pool().await;
    let ch = make_channel(&pool).await;
    let it = create(&pool, item(ch.id, "ep1", 1800, 0)).await.unwrap();

    update_health(&pool, it.id, "error", Some("timeout"), 2, None).await.unwrap();
    update_health(&pool, it.id, "ok", None, 0, None).await.unwrap();

    let updated = get(&pool, it.id).await.unwrap().unwrap();
    assert_eq!(updated.last_status.as_deref(), Some("ok"));
    assert_eq!(updated.consecutive_failures, 0);
    assert!(updated.failure_reason.is_none());
    assert!(updated.is_active);
}

#[tokio::test]
async fn test_update_health_disables_after_threshold() {
    let pool = test_pool().await;
    let ch = make_channel(&pool).await;
    let it = create(&pool, item(ch.id, "ep1", 1800, 0)).await.unwrap();

    update_health(&pool, it.id, "error", Some("connection refused"), 3, Some(false))
        .await
        .unwrap();

    let updated = get(&pool, it.id).await.unwrap().unwrap();
    assert!(!updated.is_active);
    assert_eq!(updated.consecutive_failures, 3);
    assert_eq!(updated.last_status.as_deref(), Some("error"));
    assert_eq!(updated.failure_reason.as_deref(), Some("connection refused"));
}

#[tokio::test]
async fn test_update_health_reenables_disabled_item() {
    let pool = test_pool().await;
    let ch = make_channel(&pool).await;
    let it = create(&pool, item(ch.id, "ep1", 1800, 0)).await.unwrap();

    update_health(&pool, it.id, "error", Some("timeout"), 3, Some(false))
        .await
        .unwrap();
    assert!(!get(&pool, it.id).await.unwrap().unwrap().is_active);

    update_health(&pool, it.id, "ok", None, 0, Some(true))
        .await
        .unwrap();
    let reenabled = get(&pool, it.id).await.unwrap().unwrap();
    assert!(reenabled.is_active);
    assert_eq!(reenabled.consecutive_failures, 0);
    assert_eq!(reenabled.last_status.as_deref(), Some("ok"));
    assert!(reenabled.failure_reason.is_none());
}
```

- [ ] **Step 2: Run tests to verify they fail**

Run: `cargo test model::playlist_item`
Expected: compile error — `is_active`, `set_active`, `list_active_for_channel`, `update_health` not yet defined

- [ ] **Step 3: Update the `PlaylistItem` struct**

In `src/model/playlist_item.rs`, replace the struct definition:

```rust
#[derive(Debug, Clone, Serialize, Deserialize, FromRow)]
pub struct PlaylistItem {
    pub id: i64,
    pub channel_id: i64,
    pub title: String,
    pub url: String,
    pub duration_secs: i64,
    pub sort_order: i64,
    pub is_active: bool,
    pub last_checked_at: Option<i64>,
    pub last_status: Option<String>,
    pub consecutive_failures: i64,
    pub failure_reason: Option<String>,
}
```

- [ ] **Step 4: Add the three new DB functions**

Add after the existing `delete` function in `src/model/playlist_item.rs`:

```rust
/// List only active items for a channel ordered by sort_order.
pub async fn list_active_for_channel(pool: &SqlitePool, channel_id: i64) -> Result<Vec<PlaylistItem>> {
    sqlx::query_as::<_, PlaylistItem>(
        "SELECT * FROM playlist_items WHERE channel_id = ? AND is_active = 1 ORDER BY sort_order ASC",
    )
    .bind(channel_id)
    .fetch_all(pool)
    .await
    .map_err(Into::into)
}

/// Set the is_active flag on a playlist item; returns true if a row was updated.
pub async fn set_active(pool: &SqlitePool, id: i64, active: bool) -> Result<bool> {
    let rows = sqlx::query("UPDATE playlist_items SET is_active = ? WHERE id = ?")
        .bind(active)
        .bind(id)
        .execute(pool)
        .await?
        .rows_affected();
    Ok(rows > 0)
}

/// Update health check fields on a playlist item; optionally changes is_active.
pub async fn update_health(
    pool: &SqlitePool,
    id: i64,
    status: &str,
    reason: Option<&str>,
    consecutive_failures: i64,
    is_active: Option<bool>,
) -> Result<()> {
    if let Some(active) = is_active {
        sqlx::query(
            "UPDATE playlist_items
             SET last_checked_at = strftime('%s','now'),
                 last_status = ?,
                 failure_reason = ?,
                 consecutive_failures = ?,
                 is_active = ?
             WHERE id = ?",
        )
        .bind(status)
        .bind(reason)
        .bind(consecutive_failures)
        .bind(active)
        .bind(id)
        .execute(pool)
        .await?;
    } else {
        sqlx::query(
            "UPDATE playlist_items
             SET last_checked_at = strftime('%s','now'),
                 last_status = ?,
                 failure_reason = ?,
                 consecutive_failures = ?
             WHERE id = ?",
        )
        .bind(status)
        .bind(reason)
        .bind(consecutive_failures)
        .bind(id)
        .execute(pool)
        .await?;
    }
    Ok(())
}
```

- [ ] **Step 5: Update existing test fixtures**

In the existing tests in `src/model/playlist_item.rs`, the `PlaylistItem` structs created inline (e.g. in `test_current_position_*` tests) do not have the new fields. Add the new fields with their defaults to each inline-constructed `PlaylistItem`:

```rust
PlaylistItem {
    id: 1,
    channel_id: 1,
    title: "A".into(),
    url: "u".into(),
    duration_secs: 3600,
    sort_order: 0,
    // new fields:
    is_active: true,
    last_checked_at: None,
    last_status: None,
    consecutive_failures: 0,
    failure_reason: None,
}
```

Apply this to all four inline `PlaylistItem` constructions in the `test_current_position_*` tests.

- [ ] **Step 6: Run tests to verify they pass**

Run: `cargo test model::playlist_item`
Expected: all 11 tests pass

- [ ] **Step 7: Commit**

```bash
git add src/model/playlist_item.rs
git commit -m "feat: add health tracking fields and functions to PlaylistItem"
```

---

## Task 3: Player — skip disabled items

**Files:**
- Modify: `src/routes/player.rs`

- [ ] **Step 1: Write failing tests**

Add to the `#[cfg(test)]` block in `src/routes/player.rs`, after the existing `test_tune_vod_returns_503_when_no_loop_anchor` test:

```rust
#[tokio::test]
async fn test_tune_vod_skips_disabled_item() {
    let state = test_state().await;
    let ch = make_vod_channel(&state, 0).await;

    let first = playlist_item::create(
        &state.pool,
        playlist_item::NewPlaylistItem {
            channel_id: ch.id,
            title: "A".into(),
            url: "https://example.com/a.m3u8".into(),
            duration_secs: 3600,
            sort_order: 0,
        },
    )
    .await
    .unwrap();

    playlist_item::create(
        &state.pool,
        playlist_item::NewPlaylistItem {
            channel_id: ch.id,
            title: "B".into(),
            url: "https://example.com/b.m3u8".into(),
            duration_secs: 1800,
            sort_order: 1,
        },
    )
    .await
    .unwrap();

    playlist_item::set_active(&state.pool, first.id, false).await.unwrap();

    // Active set = [B] (1800s). Any offset within 1800s lands on B.
    let result = tune_vod_at(&state, &ch, 100).await.unwrap();
    assert_eq!(result.url, "https://example.com/b.m3u8");
}

#[tokio::test]
async fn test_tune_vod_returns_503_when_all_items_disabled() {
    let state = test_state().await;
    let ch = make_vod_channel(&state, 0).await;

    let it = playlist_item::create(
        &state.pool,
        playlist_item::NewPlaylistItem {
            channel_id: ch.id,
            title: "A".into(),
            url: "https://example.com/a.m3u8".into(),
            duration_secs: 3600,
            sort_order: 0,
        },
    )
    .await
    .unwrap();

    playlist_item::set_active(&state.pool, it.id, false).await.unwrap();

    let err = tune_vod_at(&state, &ch, 1000).await.unwrap_err();
    assert_eq!(err, StatusCode::SERVICE_UNAVAILABLE);
}
```

- [ ] **Step 2: Run tests to verify they fail**

Run: `cargo test routes::player::tests::test_tune_vod_skips_disabled_item`
Expected: FAIL — `list_for_channel` returns all items including disabled ones, so the disabled item is not skipped

- [ ] **Step 3: Switch `vod_items_and_index` to `list_active_for_channel`**

In `src/routes/player.rs`, in the `vod_items_and_index` function, change one line:

```rust
// Before:
let items = playlist_item::list_for_channel(&state.pool, ch.id)
// After:
let items = playlist_item::list_active_for_channel(&state.pool, ch.id)
```

- [ ] **Step 4: Run tests to verify they pass**

Run: `cargo test routes::player`
Expected: all player tests pass

- [ ] **Step 5: Commit**

```bash
git add src/routes/player.rs
git commit -m "feat: vod player skips disabled playlist items"
```

---

## Task 4: Health checker — refactor + playlist item checking

**Files:**
- Modify: `src/health.rs`

- [ ] **Step 1: Write the `probe_playlist_item_does_not_reenable_disabled_item` test**

Add to the `#[cfg(test)]` block in `src/health.rs`, after the existing `probe_source_does_not_reenable_disabled_source` test:

```rust
#[tokio::test]
async fn probe_playlist_item_does_not_reenable_disabled_item() {
    use tokio::io::{AsyncReadExt, AsyncWriteExt};

    let listener = tokio::net::TcpListener::bind("127.0.0.1:0").await.unwrap();
    let port = listener.local_addr().unwrap().port();
    tokio::spawn(async move {
        let (mut conn, _) = listener.accept().await.unwrap();
        let mut buf = [0u8; 512];
        let _ = conn.read(&mut buf).await;
        conn.write_all(b"HTTP/1.1 200 OK\r\nContent-Length: 5\r\n\r\nhello")
            .await
            .unwrap();
    });

    let pool = crate::db::connect("sqlite::memory:").await.unwrap();
    let ch = crate::model::channel::create(
        &pool,
        crate::model::channel::NewChannel {
            name: "test".to_string(),
            category: "test".to_string(),
            logo_url: None,
            channel_type: crate::model::channel::ChannelType::VodLoop,
            sort_order: 0,
            loop_anchor: None,
        },
    )
    .await
    .unwrap();

    let it = crate::model::playlist_item::create(
        &pool,
        crate::model::playlist_item::NewPlaylistItem {
            channel_id: ch.id,
            title: "ep1".to_string(),
            url: format!("http://127.0.0.1:{}/ep1.mp4", port),
            duration_secs: 3600,
            sort_order: 0,
        },
    )
    .await
    .unwrap();

    crate::model::playlist_item::set_active(&pool, it.id, false)
        .await
        .unwrap();
    let it = crate::model::playlist_item::get(&pool, it.id)
        .await
        .unwrap()
        .unwrap();
    assert!(!it.is_active, "item must start disabled");

    let client = reqwest::Client::builder()
        .timeout(std::time::Duration::from_secs(2))
        .build()
        .unwrap();
    let cors_cache: crate::CorsCache =
        std::sync::Arc::new(tokio::sync::RwLock::new(std::collections::HashMap::new()));

    probe_playlist_item(&pool, &client, &cors_cache, &it).await;

    let updated = crate::model::playlist_item::get(&pool, it.id)
        .await
        .unwrap()
        .unwrap();
    assert!(
        !updated.is_active,
        "probe_playlist_item must not re-enable a manually disabled item"
    );
}
```

- [ ] **Step 2: Run test to verify it fails**

Run: `cargo test health::tests::probe_playlist_item`
Expected: compile error — `probe_playlist_item` not yet defined

- [ ] **Step 3: Refactor `process_result` to take plain values**

Replace the current `process_result` function and **all six** of its existing tests:

```rust
fn process_result(is_active: bool, consecutive_failures: i64, ok: bool) -> (i64, HealthAction) {
    let new_failures = if ok { 0 } else { consecutive_failures + 1 };
    let action = if ok && !is_active {
        HealthAction::Reenable
    } else if !ok && new_failures >= FAILURE_THRESHOLD && is_active {
        HealthAction::Disable
    } else {
        HealthAction::None
    };
    (new_failures, action)
}
```

Replace the six `process_result` tests (they used `mock_source()` which is now unused):

```rust
#[test]
fn test_process_result_ok_resets_failures() {
    let (failures, action) = process_result(true, 2, true);
    assert_eq!(failures, 0);
    assert!(matches!(action, HealthAction::None));
}

#[test]
fn test_process_result_error_increments_failures() {
    let (failures, action) = process_result(true, 1, false);
    assert_eq!(failures, 2);
    assert!(matches!(action, HealthAction::None));
}

#[test]
fn test_process_result_triggers_disable_at_threshold() {
    let (failures, action) = process_result(true, 2, false);
    assert_eq!(failures, 3);
    assert!(matches!(action, HealthAction::Disable));
}

#[test]
fn test_process_result_already_inactive_not_disabled_again() {
    let (failures, action) = process_result(false, 2, false);
    assert_eq!(failures, 3);
    assert!(matches!(action, HealthAction::None));
}

#[test]
fn test_process_result_reenables_inactive_source_on_success() {
    let (failures, action) = process_result(false, 3, true);
    assert_eq!(failures, 0);
    assert!(matches!(action, HealthAction::Reenable));
}

#[test]
fn test_process_result_active_source_ok_no_action() {
    let (failures, action) = process_result(true, 0, true);
    assert_eq!(failures, 0);
    assert!(matches!(action, HealthAction::None));
}
```

Also delete the now-unused `mock_source` helper.

- [ ] **Step 4: Refactor `do_http_check` and update its callers**

Replace `do_http_check`:

```rust
async fn do_http_check(client: &reqwest::Client, url: &str, kind: &str) -> (bool, Option<String>) {
    let mut resp = match client.get(url).timeout(HTTP_TIMEOUT).send().await {
        Ok(r) => r,
        Err(e) => return (false, Some(format!("request failed: {e}"))),
    };

    let status = resp.status();
    if !status.is_success() && !status.is_redirection() {
        return (false, Some(format!("HTTP {}", status.as_u16())));
    }

    if kind == "youtube_live" {
        return (true, None);
    }

    match resp.chunk().await {
        Ok(Some(_)) => (true, None),
        Ok(None) => (false, Some("stream returned no data".to_string())),
        Err(e) => (false, Some(format!("read failed: {e}"))),
    }
}
```

Update `check_source` call site (one line change):

```rust
// Before:
let (ok, reason) = do_http_check(client, src).await;
// After:
let (ok, reason) = do_http_check(client, &src.url, &src.kind).await;
```

Also update `process_result` call in `check_source`:

```rust
// Before:
let (new_failures, action) = process_result(src, ok);
// After:
let (new_failures, action) = process_result(src.is_active, src.consecutive_failures, ok);
```

Update `probe_source` call site (one line change):

```rust
// Before:
let (ok, reason) = do_http_check(client, src).await;
// After:
let (ok, reason) = do_http_check(client, &src.url, &src.kind).await;
```

- [ ] **Step 5: Add `check_playlist_item` and `probe_playlist_item`**

Add these two functions after `probe_source` in `src/health.rs`. They use `crate::model::source::SourceKind::detect` (already public) to infer the kind from the URL.

```rust
pub async fn check_playlist_item(
    pool: &SqlitePool,
    client: &reqwest::Client,
    cors_cache: &CorsCache,
    item: &crate::model::playlist_item::PlaylistItem,
) {
    let kind = crate::model::source::SourceKind::detect(&item.url);
    let (ok, reason) = do_http_check(client, &item.url, kind.as_str()).await;
    let (new_failures, action) = process_result(item.is_active, item.consecutive_failures, ok);

    let is_active = match action {
        HealthAction::Disable => Some(false),
        HealthAction::Reenable => Some(true),
        HealthAction::None => None,
    };

    if let Err(e) = crate::model::playlist_item::update_health(
        pool,
        item.id,
        if ok { "ok" } else { "error" },
        reason.as_deref(),
        new_failures,
        is_active,
    )
    .await
    {
        tracing::error!("health: failed to update playlist item {}: {e}", item.id);
        return;
    }

    match action {
        HealthAction::Disable => tracing::warn!(
            "health: playlist item {} auto-disabled after {} consecutive failures",
            item.id,
            new_failures
        ),
        HealthAction::Reenable => tracing::info!(
            "health: playlist item {} auto-re-enabled after passing health check",
            item.id
        ),
        HealthAction::None => {}
    }

    if ok {
        probe_and_cache_cors(client, cors_cache, &item.url).await;
    }
}

/// Probes a playlist item's health without changing is_active.
/// Used by the admin Test button.
pub async fn probe_playlist_item(
    pool: &SqlitePool,
    client: &reqwest::Client,
    cors_cache: &CorsCache,
    item: &crate::model::playlist_item::PlaylistItem,
) {
    let kind = crate::model::source::SourceKind::detect(&item.url);
    let (ok, reason) = do_http_check(client, &item.url, kind.as_str()).await;
    let new_failures = if ok { 0 } else { item.consecutive_failures + 1 };

    if let Err(e) = crate::model::playlist_item::update_health(
        pool,
        item.id,
        if ok { "ok" } else { "error" },
        reason.as_deref(),
        new_failures,
        None,
    )
    .await
    {
        tracing::error!("health: failed to update playlist item {}: {e}", item.id);
        return;
    }

    if ok {
        probe_and_cache_cors(client, cors_cache, &item.url).await;
    }
}
```

- [ ] **Step 6: Replace `check_all` — remove `probe_all_playlist_cors`, add playlist item loop**

Delete the entire `probe_all_playlist_cors` function. Replace `check_all` with:

```rust
async fn check_all(pool: &SqlitePool, client: &reqwest::Client, cors_cache: &CorsCache) {
    let sources = match source::list_all(pool).await {
        Ok(s) => s,
        Err(e) => {
            tracing::error!("health: failed to fetch sources: {e}");
            return;
        }
    };
    for src in sources {
        check_source(pool, client, cors_cache, &src).await;
    }

    let items = match crate::model::playlist_item::list_all(pool).await {
        Ok(i) => i,
        Err(e) => {
            tracing::error!("health: failed to fetch playlist items: {e}");
            return;
        }
    };
    for item in items {
        check_playlist_item(pool, client, cors_cache, &item).await;
    }
}
```

- [ ] **Step 7: Run all tests to verify they pass**

Run: `cargo test`
Expected: all tests pass

- [ ] **Step 8: Commit**

```bash
git add src/health.rs
git commit -m "feat: extend health checker to cover playlist items"
```

---

## Task 5: Admin layer — display types + handlers

**Files:**
- Modify: `src/routes/admin/mod.rs`
- Modify: `src/routes/admin/playlist.rs`

- [ ] **Step 1: Update `AdminPlaylistItemRow` and its `From` impl in `mod.rs`**

Replace the `AdminPlaylistItemRow` struct:

```rust
pub struct AdminPlaylistItemRow {
    pub id: i64,
    pub title: String,
    pub url: String,
    pub duration_secs: i64,
    pub sort_order: i64,
    pub budget_badge_class: &'static str,
    pub budget_badge_char: &'static str,
    pub is_active: bool,
    pub last_status: Option<String>,
    pub consecutive_failures: i64,
    pub failure_reason: Option<String>,
}
```

Replace the `From<playlist_item::PlaylistItem>` impl:

```rust
impl From<playlist_item::PlaylistItem> for AdminPlaylistItemRow {
    fn from(i: playlist_item::PlaylistItem) -> Self {
        let (budget_badge_class, budget_badge_char) =
            crate::budget::budget_badge(crate::budget::BudgetStatus::Unknown);
        Self {
            id: i.id,
            title: i.title,
            url: i.url,
            duration_secs: i.duration_secs,
            sort_order: i.sort_order,
            budget_badge_class,
            budget_badge_char,
            is_active: i.is_active,
            last_status: i.last_status,
            consecutive_failures: i.consecutive_failures,
            failure_reason: i.failure_reason,
        }
    }
}
```

- [ ] **Step 2: Add `playlist_item_toggle` re-export in `mod.rs`**

Replace the existing re-export line:

```rust
// Before:
pub use playlist::{playlist_item_create, playlist_item_delete, playlist_item_test};
// After:
pub use playlist::{playlist_item_create, playlist_item_delete, playlist_item_test, playlist_item_toggle};
```

- [ ] **Step 3: Add `playlist_item_toggle` and update `playlist_item_test` in `playlist.rs`**

Add `playlist_item_toggle` handler after `playlist_item_delete`:

```rust
pub async fn playlist_item_toggle(
    State(state): State<AppState>,
    Path(item_id): Path<i64>,
) -> Result<impl IntoResponse, StatusCode> {
    let item = playlist_item::get(&state.pool, item_id)
        .await
        .map_err(internal_error)?
        .ok_or(StatusCode::NOT_FOUND)?;

    playlist_item::set_active(&state.pool, item_id, !item.is_active)
        .await
        .map_err(internal_error)?;
    Ok(Redirect::to(&format!("/admin/channels/{}", item.channel_id)))
}
```

Replace `playlist_item_test` to call `probe_playlist_item` (mirrors how `source_test` calls `probe_source`) and re-fetch the item to get updated health stats:

```rust
pub async fn playlist_item_test(
    State(state): State<AppState>,
    Path(item_id): Path<i64>,
) -> Result<Html<String>, StatusCode> {
    let item = playlist_item::get(&state.pool, item_id)
        .await
        .map_err(internal_error)?
        .ok_or(StatusCode::NOT_FOUND)?;

    crate::health::probe_playlist_item(
        &state.pool,
        &state.http_client,
        &state.cors_cache,
        &item,
    )
    .await;

    let updated = playlist_item::get(&state.pool, item_id)
        .await
        .map_err(internal_error)?
        .ok_or(StatusCode::NOT_FOUND)?;

    let cors = state.cors_cache.read().await.clone();
    let mut row: AdminPlaylistItemRow = updated.into();
    row.apply_budget(&cors);

    render(PlaylistItemRowTemplate { item: row })
}
```

- [ ] **Step 4: Run tests**

Run: `cargo test`
Expected: all tests pass

- [ ] **Step 5: Commit**

```bash
git add src/routes/admin/mod.rs src/routes/admin/playlist.rs
git commit -m "feat: admin playlist items gain health fields + toggle handler"
```

---

## Task 6: Route wiring

**Files:**
- Modify: `src/lib.rs`

- [ ] **Step 1: Add the toggle route**

In `src/lib.rs`, in `build_router`, add after the existing `/playlist/:id/test` route:

```rust
.route(
    "/playlist/:id/toggle",
    post(routes::admin::playlist_item_toggle),
)
```

- [ ] **Step 2: Run tests**

Run: `cargo test`
Expected: all tests pass

- [ ] **Step 3: Commit**

```bash
git add src/lib.rs
git commit -m "feat: wire POST /admin/playlist/:id/toggle route"
```

---

## Task 7: Templates

**Files:**
- Modify: `templates/admin/partials/playlist_item_row.html`
- Modify: `templates/admin/channel_detail.html`

- [ ] **Step 1: Update `playlist_item_row.html`**

Replace the entire file content:

```html
<tr id="pl-row-{{ item.id }}">
  <td style="color:#555">{{ item.sort_order }}</td>
  <td>{{ item.title }}</td>
  <td style="word-break:break-all;max-width:360px;font-size:0.78rem">{{ item.url }}</td>
  <td style="white-space:nowrap">{{ item.duration_secs }}s</td>
  <td>
    {% if item.is_active %}
    <span class="badge badge-on">on</span>
    {% else %}
    <span class="badge badge-off">off</span>
    {% endif %}
  </td>
  <td>
    {% match item.last_status %}
    {% when None %}
    <span style="color:#888" title="Never checked">○</span>
    {% when Some(status) %}
    {% if status == "ok" %}
    <span style="color:#4caf50" title="Healthy">●</span>
    {% else %}
    <span style="color:#e94560" title="Last check failed">●</span>
    {% if let Some(reason) = item.failure_reason.as_ref() %}
    <div style="font-size:0.7rem;color:#e94560;margin-top:2px">{{ reason }}</div>
    {% endif %}
    {% if !item.is_active && item.consecutive_failures >= 3 %}
    <div style="font-size:0.7rem;color:#888">[auto-disabled]</div>
    {% endif %}
    {% endif %}
    {% endmatch %}
  </td>
  <td>
    {% if item.budget_badge_char.is_empty() %}
    <span style="color:#888" title="Network budget not yet probed">·</span>
    {% else %}
    <span class="{{ item.budget_badge_class }}" title="Network budget">{{ item.budget_badge_char }}</span>
    {% endif %}
  </td>
  <td style="white-space:nowrap">
    <form action="/admin/playlist/{{ item.id }}/toggle" method="post" style="display:inline-block">
      <button class="btn btn-sm" type="submit">
        {% if item.is_active %}Disable{% else %}Enable{% endif %}
      </button>
    </form>
    <form action="/admin/playlist/{{ item.id }}/delete" method="post"
          style="display:inline-block;margin-left:4px">
      <button class="btn btn-sm btn-danger" type="submit"
              onclick="return confirm('Remove this item?')">Delete</button>
    </form>
    <button class="btn btn-sm" type="button"
            hx-post="/admin/playlist/{{ item.id }}/test"
            hx-target="#pl-row-{{ item.id }}"
            hx-swap="outerHTML"
            hx-disabled-elt="this"
            style="margin-left:4px">Test</button>
  </td>
</tr>
```

- [ ] **Step 2: Update `channel_detail.html` thead**

Find line 99:
```html
      <tr><th>#</th><th>Title</th><th>URL</th><th>Duration</th><th>Budget</th><th></th></tr>
```
Replace with:
```html
      <tr><th>#</th><th>Title</th><th>URL</th><th>Duration</th><th>Active</th><th>Health</th><th>Budget</th><th></th></tr>
```

- [ ] **Step 3: Verify the build compiles (Askama validates templates at compile time)**

Run: `cargo build`
Expected: success — no template errors

- [ ] **Step 4: Run all tests**

Run: `cargo test`
Expected: all tests pass

- [ ] **Step 5: Commit**

```bash
git add templates/admin/partials/playlist_item_row.html templates/admin/channel_detail.html
git commit -m "feat: playlist item rows show active/health/budget badges and toggle button"
```

---

## Task 8: Integration test

**Files:**
- Modify: `tests/http.rs`

- [ ] **Step 1: Add the integration test**

Add after the existing `test_tune_vod_empty_playlist_returns_503` test:

```rust
#[tokio::test]
async fn test_tune_vod_skips_disabled_item() {
    let app = app().await;

    // Seed has channel 4 with items id=1 (ep1) and id=2 (ep2).
    // Disable ep1 via the new toggle endpoint.
    let toggle = app
        .clone()
        .oneshot(authed_post("/admin/playlist/1/toggle"))
        .await
        .unwrap();
    assert_eq!(toggle.status(), StatusCode::SEE_OTHER);

    // Channel 4 now only has ep2 active — tune must return its URL.
    let tune = app.oneshot(req("/channel/4/tune")).await.unwrap();
    assert_eq!(tune.status(), StatusCode::OK);
    let json = body_json(tune).await;
    assert!(
        json["url"].as_str().unwrap().contains("ep2"),
        "disabled ep1 should be skipped; ep2 expected"
    );
}
```

Note: `app()` returns `axum::Router` which is `Clone`. Both clones share the same underlying `SqlitePool` (Arc-backed), so the toggle POST and tune GET operate on the same in-memory DB.

- [ ] **Step 2: Run the integration test**

Run: `cargo test test_tune_vod_skips_disabled_item`
Expected: PASS

- [ ] **Step 3: Run the full test suite**

Run: `cargo test`
Expected: all tests pass

- [ ] **Step 4: Run fmt and clippy**

```bash
cargo fmt
cargo clippy --all-targets -- -D warnings
```
Expected: no warnings, no formatting diffs

- [ ] **Step 5: Commit**

```bash
git add tests/http.rs
git commit -m "test: integration test for disabled VOD item skip"
```
