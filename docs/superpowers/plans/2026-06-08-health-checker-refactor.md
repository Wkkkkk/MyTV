# Health Checker Refactor Implementation Plan

> **For agentic workers:** REQUIRED SUB-SKILL: Use superpowers:subagent-driven-development (recommended) or superpowers:executing-plans to implement this plan task-by-task. Steps use checkbox (`- [ ]`) syntax for tracking.

**Goal:** Eliminate the four near-identical health-check functions in `src/health.rs` by extracting a shared `run_check` helper, and restore per-cycle CORS host deduplication so N items on the same CDN produce 1 probe instead of N.

**Architecture:** A private `run_check` generic function handles the shared lifecycle (HTTP check → DB update → optional lifecycle logging); the four public/private wrappers become thin delegators. A cycle-local `HashSet<String>` in `check_all` tracks which CDN hosts have been probed and skips duplicates.

**Tech Stack:** Rust 1.96, `src/health.rs` only — no schema changes, no new files, no public API signature changes.

---

## File Map

- **Modify:** `src/health.rs` — all changes live here

---

### Task 1: Write the failing test for `run_check` probe mode

**Files:**
- Modify: `src/health.rs` (test module at bottom of file)

The test verifies that when `manage_lifecycle: false`, the update closure always receives `is_active_change = None` — even when the source is inactive and the health check passes (which would normally trigger a Reenable action in background-check mode).

- [ ] **Step 1: Add the test to the `#[cfg(test)]` module at the bottom of `src/health.rs`**

Append this test inside the existing `mod tests { ... }` block (after `probe_playlist_item_does_not_reenable_disabled_item`):

```rust
#[tokio::test]
async fn test_run_check_probe_mode_never_changes_is_active() {
    use tokio::io::{AsyncReadExt, AsyncWriteExt};

    // Server returns 200 with body — simulates a healthy source
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

    let client = reqwest::Client::builder()
        .timeout(std::time::Duration::from_secs(2))
        .build()
        .unwrap();

    // Source is inactive with 3 failures — in manage_lifecycle=true mode this would Reenable.
    // In probe mode (manage_lifecycle=false) it must never touch is_active.
    let ok = run_check(
        &client,
        &format!("http://127.0.0.1:{}/stream.m3u8", port),
        "hls",
        false, // is_active: currently disabled
        3,     // consecutive_failures
        false, // manage_lifecycle: probe mode
        |_status, _reason, _failures, is_active_change| async move {
            assert!(
                is_active_change.is_none(),
                "probe mode must never pass is_active_change = Some(…)"
            );
            Ok::<(), sqlx::Error>(())
        },
    )
    .await;

    assert!(ok, "server returned 200 — run_check must return true");
}
```

- [ ] **Step 2: Run the test to verify it fails to compile (function doesn't exist yet)**

```bash
cargo test test_run_check_probe_mode_never_changes_is_active 2>&1 | head -20
```

Expected: compile error — `run_check` not found. This is the correct failing state.

- [ ] **Step 3: Commit the failing test**

```bash
git add src/health.rs
git commit -m "test: add failing test for run_check probe mode (idea 23)"
```

---

### Task 2: Extract `run_check` and refactor the four functions

**Files:**
- Modify: `src/health.rs`

Replace the duplicated bodies of `check_source`, `probe_source`, `check_playlist_item`, and `probe_playlist_item` with a shared `run_check` helper. The four functions become thin wrappers.

Key design decisions:
- `run_check` takes `manage_lifecycle: bool`: `true` for background-checker paths, `false` for admin Test-button paths.
- The `update` closure takes `(status: &'static str, reason: Option<String>, failures: i64, is_active_change: Option<bool>)` and returns a `Future<Output = sqlx::Result<()>>`. Use `async move {}` inside closures so the future owns `reason`.
- `check_source` and `check_playlist_item` now return `bool` (the `ok` result). They no longer accept `cors_cache` — CORS probing moves to `check_all` in Task 4. For now, after extracting `run_check`, keep the `cors_cache` parameter and the `probe_and_cache_cors` call in these two private functions (dedup comes in Task 4).
- All tracing messages in `run_check` use `url` instead of numeric ID — more actionable in logs.

- [ ] **Step 1: Add `run_check` above `check_source` in `src/health.rs`**

Insert this function between the `HealthClients` struct block and the existing `probe_source` function:

```rust
async fn run_check<F, Fut>(
    client: &reqwest::Client,
    url: &str,
    kind: &str,
    is_active: bool,
    consecutive_failures: i64,
    manage_lifecycle: bool,
    update: F,
) -> bool
where
    F: FnOnce(&'static str, Option<String>, i64, Option<bool>) -> Fut,
    Fut: std::future::Future<Output = sqlx::Result<()>>,
{
    let (ok, reason) = do_http_check(client, url, kind).await;
    let (new_failures, action) = process_result(is_active, consecutive_failures, ok);
    let is_active_change = if manage_lifecycle {
        match action {
            HealthAction::Disable => Some(false),
            HealthAction::Reenable => Some(true),
            HealthAction::None => None,
        }
    } else {
        None
    };

    let status: &'static str = if ok { "ok" } else { "error" };
    if let Err(e) = update(status, reason, new_failures, is_active_change).await {
        tracing::error!("health: failed to update {url}: {e}");
        return false;
    }

    if manage_lifecycle {
        match action {
            HealthAction::Disable => tracing::warn!(
                "health: {url} auto-disabled after {new_failures} consecutive failures"
            ),
            HealthAction::Reenable => {
                tracing::info!("health: {url} auto-re-enabled after passing health check")
            }
            HealthAction::None => {}
        }
    }

    ok
}
```

- [ ] **Step 2: Replace `probe_source` with a thin wrapper**

Replace the existing `probe_source` function (lines 63–89 in the original) with:

```rust
pub async fn probe_source(
    pool: &SqlitePool,
    client: &reqwest::Client,
    cors_cache: &CorsCache,
    src: &Source,
) {
    let ok = run_check(
        client,
        &src.url,
        &src.kind,
        src.is_active,
        src.consecutive_failures,
        false,
        |status, reason, failures, is_active_change| async move {
            source::update_health(pool, src.id, status, reason.as_deref(), failures, is_active_change).await
        },
    )
    .await;

    if ok {
        probe_and_cache_cors(client, cors_cache, &src.url).await;
    }
}
```

- [ ] **Step 3: Replace `check_source` with a thin wrapper**

Replace the existing `check_source` function (lines 91–138 in the original) with:

```rust
async fn check_source(
    pool: &SqlitePool,
    client: &reqwest::Client,
    cors_cache: &CorsCache,
    src: &Source,
) -> bool {
    let ok = run_check(
        client,
        &src.url,
        &src.kind,
        src.is_active,
        src.consecutive_failures,
        true,
        |status, reason, failures, is_active_change| async move {
            source::update_health(pool, src.id, status, reason.as_deref(), failures, is_active_change).await
        },
    )
    .await;

    if ok {
        probe_and_cache_cors(client, cors_cache, &src.url).await;
    }

    ok
}
```

- [ ] **Step 4: Replace `check_playlist_item` with a thin wrapper**

Replace the existing `check_playlist_item` function (lines 140–186 in the original) with:

```rust
async fn check_playlist_item(
    pool: &SqlitePool,
    client: &reqwest::Client,
    cors_cache: &CorsCache,
    item: &crate::model::playlist_item::PlaylistItem,
) -> bool {
    let kind = crate::model::source::SourceKind::detect(&item.url);
    let ok = run_check(
        client,
        &item.url,
        kind.as_str(),
        item.is_active,
        item.consecutive_failures,
        true,
        |status, reason, failures, is_active_change| async move {
            crate::model::playlist_item::update_health(pool, item.id, status, reason.as_deref(), failures, is_active_change).await
        },
    )
    .await;

    if ok {
        probe_and_cache_cors(client, cors_cache, &item.url).await;
    }

    ok
}
```

- [ ] **Step 5: Replace `probe_playlist_item` with a thin wrapper**

Replace the existing `probe_playlist_item` function (lines 188–215 in the original) with:

```rust
pub async fn probe_playlist_item(
    pool: &SqlitePool,
    client: &reqwest::Client,
    cors_cache: &CorsCache,
    item: &crate::model::playlist_item::PlaylistItem,
) {
    let kind = crate::model::source::SourceKind::detect(&item.url);
    let ok = run_check(
        client,
        &item.url,
        kind.as_str(),
        item.is_active,
        item.consecutive_failures,
        false,
        |status, reason, failures, is_active_change| async move {
            crate::model::playlist_item::update_health(pool, item.id, status, reason.as_deref(), failures, is_active_change).await
        },
    )
    .await;

    if ok {
        probe_and_cache_cors(client, cors_cache, &item.url).await;
    }
}
```

- [ ] **Step 6: Update `check_all` callers to capture the returned `bool`**

`check_source` and `check_playlist_item` now return `bool`. `check_all` currently calls them and ignores the return. Update `check_all` to capture the return value (even if it's `_ok` for now — dedup uses it in Task 4):

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
        let _ = check_source(pool, client, cors_cache, &src).await;
    }

    let items = match crate::model::playlist_item::list_all(pool).await {
        Ok(i) => i,
        Err(e) => {
            tracing::error!("health: failed to fetch playlist items: {e}");
            return;
        }
    };
    for item in items {
        let _ = check_playlist_item(pool, client, cors_cache, &item).await;
    }
}
```

- [ ] **Step 7: Run the target test and all tests**

```bash
cargo test test_run_check_probe_mode_never_changes_is_active 2>&1 | tail -5
```

Expected: `test test_run_check_probe_mode_never_changes_is_active ... ok`

```bash
cargo test 2>&1 | tail -10
```

Expected: all tests pass (same count as before, no failures).

- [ ] **Step 8: Run clippy and fmt**

```bash
cargo clippy -- -D warnings 2>&1 | tail -20
cargo fmt
```

Expected: no warnings, no diff.

- [ ] **Step 9: Commit**

```bash
git add src/health.rs
git commit -m "refactor: extract run_check helper, collapse 4 health-check functions (idea 23)"
```

---

### Task 3: Write the failing test for CORS dedup

**Files:**
- Modify: `src/health.rs` (test module)

Two tests: a unit test for the dedup mechanism (same-host URLs → HashSet deduplicates), and a structural integration test verifying that a VOD channel's items are each health-checked independently even with dedup.

Note: `probe_and_cache_cors` skips non-HTTPS URLs (returns `None` immediately), so counting CORS HEAD probes requires HTTPS test infrastructure that is out of scope here. The unit test below verifies the dedup mechanism directly. The integration test covers the health-check-per-item correctness.

- [ ] **Step 1: Add the dedup mechanism unit test inside `mod tests`**

```rust
#[test]
fn test_probed_hosts_dedup_same_cdn() {
    // Two episodes on the same CDN produce the same manifest host.
    // The HashSet used in check_all must deduplicate them so only
    // the first triggers a CORS probe.
    let mut probed_hosts = std::collections::HashSet::new();

    let ep1 = "https://cdn.example.com/vod/season1/ep1.m3u8";
    let ep2 = "https://cdn.example.com/vod/season1/ep2.m3u8";
    let ep3 = "https://other-cdn.example.com/vod/ep3.m3u8";

    let h1 = crate::media::hls::extract_manifest_host(ep1);
    let h2 = crate::media::hls::extract_manifest_host(ep2);
    let h3 = crate::media::hls::extract_manifest_host(ep3);

    assert!(probed_hosts.insert(h1), "ep1: first insert for this CDN host must succeed");
    assert!(!probed_hosts.insert(h2), "ep2: same CDN host must be deduplicated");
    assert!(probed_hosts.insert(h3), "ep3: different CDN host must not be deduplicated");
    assert_eq!(probed_hosts.len(), 2);
}
```

- [ ] **Step 2: Add an integration test verifying per-item health checking with shared CDN**

This test creates two playlist items on the same host, runs `check_all`, and asserts that both items have their health independently updated in the DB (each gets its own HTTP health check).

```rust
#[tokio::test]
async fn test_check_all_health_checks_each_item_independently() {
    use tokio::io::{AsyncReadExt, AsyncWriteExt};

    // Server returns 200 for the first two connections (one per item health check)
    let listener = tokio::net::TcpListener::bind("127.0.0.1:0").await.unwrap();
    let port = listener.local_addr().unwrap().port();
    tokio::spawn(async move {
        for _ in 0..2u8 {
            let (mut conn, _) = listener.accept().await.unwrap();
            let mut buf = [0u8; 512];
            let _ = conn.read(&mut buf).await;
            conn.write_all(b"HTTP/1.1 200 OK\r\nContent-Length: 5\r\n\r\nhello")
                .await
                .unwrap();
        }
    });

    let pool = crate::db::connect("sqlite::memory:").await.unwrap();
    let ch = crate::model::channel::create(
        &pool,
        crate::model::channel::NewChannel {
            name: "vod".to_string(),
            category: "test".to_string(),
            logo_url: None,
            channel_type: crate::model::channel::ChannelType::VodLoop,
            sort_order: 0,
            loop_anchor: None,
        },
    )
    .await
    .unwrap();

    let it1 = crate::model::playlist_item::create(
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

    let it2 = crate::model::playlist_item::create(
        &pool,
        crate::model::playlist_item::NewPlaylistItem {
            channel_id: ch.id,
            title: "ep2".to_string(),
            url: format!("http://127.0.0.1:{}/ep2.mp4", port),
            duration_secs: 3600,
            sort_order: 1,
        },
    )
    .await
    .unwrap();

    let cors_cache: crate::CorsCache =
        std::sync::Arc::new(tokio::sync::RwLock::new(std::collections::HashMap::new()));
    let client = reqwest::Client::builder()
        .timeout(std::time::Duration::from_secs(2))
        .build()
        .unwrap();

    check_all(&pool, &client, &cors_cache).await;

    // Both items must have been health-checked independently
    let updated1 = crate::model::playlist_item::get(&pool, it1.id)
        .await
        .unwrap()
        .unwrap();
    let updated2 = crate::model::playlist_item::get(&pool, it2.id)
        .await
        .unwrap()
        .unwrap();

    assert_eq!(
        updated1.last_status.as_deref(),
        Some("ok"),
        "ep1 must be health-checked independently"
    );
    assert_eq!(
        updated2.last_status.as_deref(),
        Some("ok"),
        "ep2 must be health-checked independently"
    );
}
```

- [ ] **Step 3: Run the tests to confirm they compile and pass**

The mechanism test passes immediately (pure HashSet logic). The integration test requires `check_all` to be accessible from the test module. In Rust, private functions are accessible within the same module's `#[cfg(test)]` block. Confirm:

```bash
cargo test test_probed_hosts_dedup_same_cdn test_check_all_health_checks_each_item_independently 2>&1 | tail -10
```

Expected: both tests pass (the dedup mechanism and independent health check both work before the refactor, since the refactor doesn't break per-item checking).

- [ ] **Step 4: Commit the tests**

```bash
git add src/health.rs
git commit -m "test: add CORS dedup mechanism test and per-item health check integration test (idea 23)"
```

---

### Task 4: Add CORS dedup to `check_all`

**Files:**
- Modify: `src/health.rs`

Move `probe_and_cache_cors` calls out of `check_source` and `check_playlist_item` and into `check_all`, controlled by a cycle-local `probed_hosts: HashSet<String>`. Both private functions drop the `cors_cache` parameter.

- [ ] **Step 1: Add `use std::collections::HashSet;` to the imports at the top of `src/health.rs`**

The file currently has:
```rust
use std::time::Duration;

use sqlx::SqlitePool;

use crate::model::source::{self, Source};
use crate::CorsCache;
```

Change to:
```rust
use std::collections::HashSet;
use std::time::Duration;

use sqlx::SqlitePool;

use crate::model::source::{self, Source};
use crate::CorsCache;
```

- [ ] **Step 2: Remove `cors_cache` from `check_source`, remove the CORS probe call, keep returning `bool`**

Replace `check_source` with:

```rust
async fn check_source(pool: &SqlitePool, client: &reqwest::Client, src: &Source) -> bool {
    run_check(
        client,
        &src.url,
        &src.kind,
        src.is_active,
        src.consecutive_failures,
        true,
        |status, reason, failures, is_active_change| async move {
            source::update_health(pool, src.id, status, reason.as_deref(), failures, is_active_change).await
        },
    )
    .await
}
```

- [ ] **Step 3: Remove `cors_cache` from `check_playlist_item`, remove the CORS probe call, keep returning `bool`**

Replace `check_playlist_item` with:

```rust
async fn check_playlist_item(
    pool: &SqlitePool,
    client: &reqwest::Client,
    item: &crate::model::playlist_item::PlaylistItem,
) -> bool {
    let kind = crate::model::source::SourceKind::detect(&item.url);
    run_check(
        client,
        &item.url,
        kind.as_str(),
        item.is_active,
        item.consecutive_failures,
        true,
        |status, reason, failures, is_active_change| async move {
            crate::model::playlist_item::update_health(
                pool,
                item.id,
                status,
                reason.as_deref(),
                failures,
                is_active_change,
            )
            .await
        },
    )
    .await
}
```

- [ ] **Step 4: Rewrite `check_all` with the dedup loop**

Replace `check_all` with:

```rust
async fn check_all(pool: &SqlitePool, client: &reqwest::Client, cors_cache: &CorsCache) {
    let mut probed_hosts: HashSet<String> = HashSet::new();

    let sources = match source::list_all(pool).await {
        Ok(s) => s,
        Err(e) => {
            tracing::error!("health: failed to fetch sources: {e}");
            return;
        }
    };
    for src in sources {
        let ok = check_source(pool, client, &src).await;
        if ok {
            let host = crate::media::hls::extract_manifest_host(&src.url);
            if probed_hosts.insert(host) {
                probe_and_cache_cors(client, cors_cache, &src.url).await;
            }
        }
    }

    let items = match crate::model::playlist_item::list_all(pool).await {
        Ok(i) => i,
        Err(e) => {
            tracing::error!("health: failed to fetch playlist items: {e}");
            return;
        }
    };
    for item in items {
        let ok = check_playlist_item(pool, client, &item).await;
        if ok {
            let host = crate::media::hls::extract_manifest_host(&item.url);
            if probed_hosts.insert(host) {
                probe_and_cache_cors(client, cors_cache, &item.url).await;
            }
        }
    }
}
```

- [ ] **Step 5: Run all tests**

```bash
cargo test 2>&1 | tail -15
```

Expected: all tests pass. Count should be the same as before plus the two new tests added in Task 3.

- [ ] **Step 6: Run clippy and fmt**

```bash
cargo clippy -- -D warnings 2>&1 | tail -20
cargo fmt
```

Expected: no warnings, no diff.

- [ ] **Step 7: Commit**

```bash
git add src/health.rs
git commit -m "fix: restore CORS host deduplication in health checker cycle (idea 23)"
```
