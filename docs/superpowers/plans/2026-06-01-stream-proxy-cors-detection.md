# Stream Proxy CORS Detection Implementation Plan

> **For agentic workers:** REQUIRED SUB-SKILL: Use superpowers:subagent-driven-development (recommended) or superpowers:executing-plans to implement this plan task-by-task. Steps use checkbox (`- [ ]`) syntax for tracking.

**Goal:** Automatically detect when HLS segment servers send CORS headers and serve segments directly from the origin CDN, eliminating Fly.io egress for those streams; surface per-channel health and budget status as two pill badges in the EPG guide.

**Architecture:** An in-memory `cors_cache` (keyed by manifest `scheme://host`) is shared across the stream proxy handler, the health checker, and the guide route. The stream proxy probes a segment on first tune; the health checker re-probes on each 15-minute cycle so the guide shows status before any channel is played. `rewrite_hls_urls` gains a `direct_segments` flag that writes absolute segment URLs when true and leaves sub-playlist URLs always proxied.

**Tech Stack:** Rust/Axum, `tokio::sync::RwLock`, `reqwest`, Askama templates, HTMX

---

### Task 1: Extend `rewrite_hls_urls` with `direct_segments` flag

**Files:**
- Modify: `src/media/hls.rs`
- Modify: `src/routes/player.rs` (call site)

- [ ] **Step 1: Write failing tests for `direct_segments = true`**

Add to the `#[cfg(test)]` block in `src/media/hls.rs`:

```rust
#[test]
fn test_rewrite_hls_urls_direct_mode_segments_are_absolute() {
    let manifest = "#EXTM3U\nseg1.ts\n";
    let result = rewrite_hls_urls(manifest, "https://example.com/live/index.m3u8", true);
    assert_eq!(result, "#EXTM3U\nhttps://example.com/live/seg1.ts");
}

#[test]
fn test_rewrite_hls_urls_direct_mode_playlists_still_proxied() {
    let manifest = "#EXTM3U\nvariant.m3u8\n";
    let result = rewrite_hls_urls(manifest, "https://example.com/master.m3u8", true);
    assert!(result.contains("/stream-proxy?url="));
    assert!(!result.contains("\nhttps://example.com/variant.m3u8"));
}

#[test]
fn test_rewrite_hls_urls_proxy_mode_all_proxied() {
    let manifest = "#EXTM3U\nseg1.ts\n";
    let result = rewrite_hls_urls(manifest, "https://example.com/live/index.m3u8", false);
    assert!(result.contains("/stream-proxy?url="));
}
```

- [ ] **Step 2: Run tests and confirm they fail**

```bash
cargo test test_rewrite_hls_urls_direct 2>&1 | head -20
```

Expected: compile error — `rewrite_hls_urls` called with 3 args but only accepts 2.

- [ ] **Step 3: Update `rewrite_hls_urls` signature and logic**

In `src/media/hls.rs`, replace the function:

```rust
pub fn rewrite_hls_urls(content: &str, base_url: &str, direct_segments: bool) -> String {
    let base_dir = base_url
        .rsplit_once('/')
        .map(|(b, _)| b)
        .unwrap_or(base_url);
    let origin = {
        let after_scheme = base_url.find("://").map(|i| i + 3).unwrap_or(0);
        let host_len = base_url[after_scheme..]
            .find('/')
            .unwrap_or(base_url[after_scheme..].len());
        &base_url[..after_scheme + host_len]
    };

    content
        .lines()
        .map(|line| {
            if line.starts_with('#') || line.is_empty() {
                return line.to_string();
            }
            let abs = if line.starts_with("http://") || line.starts_with("https://") {
                line.to_string()
            } else if line.starts_with('/') {
                format!("{}{}", origin, line)
            } else {
                format!("{}/{}", base_dir, line)
            };
            let lower = abs.to_lowercase();
            if direct_segments && !lower.ends_with(".m3u8") && !lower.ends_with(".m3u") {
                abs
            } else {
                format!("/stream-proxy?url={}", pct_encode(&abs))
            }
        })
        .collect::<Vec<_>>()
        .join("\n")
}
```

- [ ] **Step 4: Fix the existing tests — add `false` as third argument**

Update every `rewrite_hls_urls(manifest, url)` call in the existing test block to `rewrite_hls_urls(manifest, url, false)`. There are 4 tests: `test_rewrite_hls_urls_absolute`, `test_rewrite_hls_urls_relative`, `test_rewrite_hls_urls_root_relative`, `test_rewrite_hls_urls_leaves_comments_unchanged`.

- [ ] **Step 5: Fix the call site in `stream_proxy`**

In `src/routes/player.rs` line 252, change:
```rust
let rewritten = hls::rewrite_hls_urls(&text, &q.url);
```
to:
```rust
let rewritten = hls::rewrite_hls_urls(&text, &q.url, false);
```

- [ ] **Step 6: Run all tests and confirm passing**

```bash
cargo test 2>&1 | tail -5
```

Expected: `test result: ok. N passed; 0 failed`

- [ ] **Step 7: Commit**

```bash
cargo fmt
git add src/media/hls.rs src/routes/player.rs
git commit -m "feat: add direct_segments flag to rewrite_hls_urls"
```

---

### Task 2: Add `find_first_segment_url`

**Files:**
- Modify: `src/media/hls.rs`

- [ ] **Step 1: Write failing tests**

Add to the `#[cfg(test)]` block in `src/media/hls.rs`:

```rust
#[test]
fn test_find_first_segment_url_returns_resolved_ts() {
    let manifest = "#EXTM3U\n#EXT-X-TARGETDURATION:6\nseg1.ts\nseg2.ts\n";
    let result = find_first_segment_url(manifest, "https://example.com/live/index.m3u8");
    assert_eq!(result, Some("https://example.com/live/seg1.ts".to_string()));
}

#[test]
fn test_find_first_segment_url_skips_m3u8_lines() {
    let manifest = "#EXTM3U\nvariant.m3u8\nseg1.ts\n";
    let result = find_first_segment_url(manifest, "https://example.com/live/index.m3u8");
    assert_eq!(result, Some("https://example.com/live/seg1.ts".to_string()));
}

#[test]
fn test_find_first_segment_url_returns_none_for_master_playlist() {
    let manifest = "#EXTM3U\n#EXT-X-STREAM-INF:BANDWIDTH=1000\nvariant.m3u8\n";
    let result = find_first_segment_url(manifest, "https://example.com/master.m3u8");
    assert_eq!(result, None);
}

#[test]
fn test_find_first_segment_url_absolute_segment() {
    let manifest = "#EXTM3U\nhttps://cdn.example.com/seg1.ts\n";
    let result = find_first_segment_url(manifest, "https://example.com/live/index.m3u8");
    assert_eq!(result, Some("https://cdn.example.com/seg1.ts".to_string()));
}

#[test]
fn test_find_first_segment_url_root_relative() {
    let manifest = "#EXTM3U\n/hls/seg1.ts\n";
    let result = find_first_segment_url(manifest, "https://example.com/live/index.m3u8");
    assert_eq!(result, Some("https://example.com/hls/seg1.ts".to_string()));
}

#[test]
fn test_find_first_segment_url_returns_none_for_empty_manifest() {
    assert_eq!(find_first_segment_url("#EXTM3U\n", "https://example.com/index.m3u8"), None);
}
```

- [ ] **Step 2: Run tests and confirm they fail**

```bash
cargo test test_find_first_segment_url 2>&1 | head -5
```

Expected: compile error — function not found.

- [ ] **Step 3: Implement `find_first_segment_url`**

Add to `src/media/hls.rs` (before the `#[cfg(test)]` block):

```rust
/// Returns the first resolved absolute segment URL from an HLS media playlist.
/// Skips comment lines, empty lines, and sub-playlist lines (`.m3u8`/`.m3u`).
/// Returns `None` for master playlists that contain only sub-playlist lines.
pub fn find_first_segment_url(content: &str, base_url: &str) -> Option<String> {
    let base_dir = base_url.rsplit_once('/').map(|(b, _)| b).unwrap_or(base_url);
    let after_scheme = base_url.find("://").map(|i| i + 3).unwrap_or(0);
    let host_len = base_url[after_scheme..]
        .find('/')
        .unwrap_or(base_url[after_scheme..].len());
    let origin = &base_url[..after_scheme + host_len];

    for line in content.lines() {
        if line.starts_with('#') || line.is_empty() {
            continue;
        }
        let lower = line.to_lowercase();
        if lower.ends_with(".m3u8") || lower.ends_with(".m3u") {
            continue;
        }
        let abs = if line.starts_with("http://") || line.starts_with("https://") {
            line.to_string()
        } else if line.starts_with('/') {
            format!("{}{}", origin, line)
        } else {
            format!("{}/{}", base_dir, line)
        };
        return Some(abs);
    }
    None
}
```

- [ ] **Step 4: Run tests and confirm passing**

```bash
cargo test test_find_first_segment_url 2>&1 | tail -5
```

Expected: all 6 tests pass.

- [ ] **Step 5: Commit**

```bash
cargo fmt
git add src/media/hls.rs
git commit -m "feat: add find_first_segment_url to hls module"
```

---

### Task 3: Add `has_cors_wildcard` and `probe_cors`

**Files:**
- Modify: `src/media/hls.rs`

- [ ] **Step 1: Write failing tests for `has_cors_wildcard`**

Add to the `#[cfg(test)]` block in `src/media/hls.rs`:

```rust
#[test]
fn test_has_cors_wildcard_returns_true_for_star() {
    let mut headers = reqwest::header::HeaderMap::new();
    headers.insert(
        "access-control-allow-origin",
        reqwest::header::HeaderValue::from_static("*"),
    );
    assert!(has_cors_wildcard(&headers));
}

#[test]
fn test_has_cors_wildcard_returns_false_when_absent() {
    assert!(!has_cors_wildcard(&reqwest::header::HeaderMap::new()));
}

#[test]
fn test_has_cors_wildcard_returns_false_for_specific_origin() {
    let mut headers = reqwest::header::HeaderMap::new();
    headers.insert(
        "access-control-allow-origin",
        reqwest::header::HeaderValue::from_static("https://example.com"),
    );
    assert!(!has_cors_wildcard(&headers));
}
```

- [ ] **Step 2: Run tests and confirm they fail**

```bash
cargo test test_has_cors_wildcard 2>&1 | head -5
```

Expected: compile error.

- [ ] **Step 3: Implement `has_cors_wildcard` and `probe_cors`**

Add to `src/media/hls.rs` (before the `#[cfg(test)]` block):

```rust
/// Returns true if the header map contains `Access-Control-Allow-Origin: *`.
pub fn has_cors_wildcard(headers: &reqwest::header::HeaderMap) -> bool {
    headers
        .get("access-control-allow-origin")
        .and_then(|v| v.to_str().ok())
        .map(|v| v.trim() == "*")
        .unwrap_or(false)
}

/// HEAD-requests `url` and returns true if the response includes `Access-Control-Allow-Origin: *`.
/// Returns false on any network or timeout error (proxy is the safe default).
pub async fn probe_cors(client: &reqwest::Client, url: &str) -> bool {
    match client
        .head(url)
        .timeout(std::time::Duration::from_secs(5))
        .send()
        .await
    {
        Ok(resp) => has_cors_wildcard(resp.headers()),
        Err(e) => {
            tracing::debug!(url = %url, error = %e, "CORS probe failed, defaulting to proxy");
            false
        }
    }
}
```

- [ ] **Step 4: Run tests and confirm passing**

```bash
cargo test test_has_cors_wildcard 2>&1 | tail -5
```

Expected: 3 tests pass.

- [ ] **Step 5: Commit**

```bash
cargo fmt
git add src/media/hls.rs
git commit -m "feat: add has_cors_wildcard and probe_cors to hls module"
```

---

### Task 4: Add `cors_cache` to `AppState`

**Files:**
- Modify: `src/lib.rs`
- Modify: `src/main.rs`
- Modify: `src/routes/player.rs` (`test_state`)
- Modify: `tests/http.rs` (`app`)

- [ ] **Step 1: Add `CorsCache` type alias and field to `AppState`**

In `src/lib.rs`, add imports and update the struct:

```rust
use std::collections::HashMap;
use std::sync::Arc;
use tokio::sync::RwLock;

pub type CorsCache = Arc<RwLock<HashMap<String, bool>>>;

#[derive(Clone)]
pub struct AppState {
    pub pool: SqlitePool,
    pub config: Arc<config::Config>,
    pub http_client: reqwest::Client,
    pub cors_cache: CorsCache,
}
```

- [ ] **Step 2: Run `cargo build` and fix every compile error**

```bash
cargo build 2>&1 | grep "error\[" | head -20
```

This reveals all places that construct `AppState`. Fix each one in the following steps.

- [ ] **Step 3: Update `main.rs`**

```rust
use mytv::{build_router, config, db, health, AppState, CorsCache};
use std::collections::HashMap;
use std::sync::Arc;
use tokio::sync::RwLock;

// Inside main():
let cors_cache: CorsCache = Arc::new(RwLock::new(HashMap::new()));

let state = AppState {
    pool,
    config: config.clone(),
    http_client,
    cors_cache,
};
```

- [ ] **Step 4: Update `test_state()` in `src/routes/player.rs`**

```rust
async fn test_state() -> AppState {
    let pool = db::connect("sqlite::memory:").await.unwrap();
    let config = std::sync::Arc::new(config::Config::from_env().unwrap());
    AppState {
        pool,
        config,
        http_client: reqwest::Client::new(),
        cors_cache: std::sync::Arc::new(
            tokio::sync::RwLock::new(std::collections::HashMap::new()),
        ),
    }
}
```

- [ ] **Step 5: Update `app()` in `tests/http.rs`**

```rust
use mytv::{build_router, config::Config, db, AppState, CorsCache};

async fn app() -> axum::Router {
    let pool = db::connect("sqlite::memory:").await.unwrap();
    sqlx::query(include_str!("fixtures/seed.sql"))
        .execute(&pool)
        .await
        .unwrap();
    let state = AppState {
        pool,
        config: Arc::new(Config {
            database_url: "sqlite::memory:".to_string(),
            admin_password: "test".to_string(),
            youtube_api_key: None,
            port: 0,
        }),
        http_client: reqwest::Client::new(),
        cors_cache: Arc::new(tokio::sync::RwLock::new(std::collections::HashMap::new())),
    };
    build_router(state)
}
```

- [ ] **Step 6: Run all tests and confirm passing**

```bash
cargo test 2>&1 | tail -5
```

Expected: all tests pass.

- [ ] **Step 7: Commit**

```bash
cargo fmt
git add src/lib.rs src/main.rs src/routes/player.rs tests/http.rs
git commit -m "feat: add cors_cache to AppState"
```

---

### Task 5: Wire CORS detection into `stream_proxy`

**Files:**
- Modify: `src/routes/player.rs`

- [ ] **Step 1: Add `extract_manifest_host` helper**

Add to `src/routes/player.rs` (in the stream proxy section, after the `StreamProxyQuery` struct):

```rust
fn extract_manifest_host(url: &str) -> String {
    let after = url.find("://").map(|i| i + 3).unwrap_or(0);
    let host_end = url[after..].find('/').unwrap_or(url[after..].len());
    url[..after + host_end].to_string()
}
```

- [ ] **Step 2: Add `resolve_direct_segments` helper**

Add immediately after `extract_manifest_host`:

```rust
async fn resolve_direct_segments(state: &AppState, content: &str, base_url: &str) -> bool {
    let segment_url = match hls::find_first_segment_url(content, base_url) {
        Some(u) => u,
        None => return false,
    };
    if !segment_url.starts_with("https://") {
        return false;
    }
    let host_key = extract_manifest_host(base_url);
    {
        let cache = state.cors_cache.read().await;
        if let Some(&cached) = cache.get(&host_key) {
            return cached;
        }
    }
    let result = hls::probe_cors(&state.http_client, &segment_url).await;
    tracing::debug!(host = %host_key, cors = result, "CORS probe result cached");
    state.cors_cache.write().await.insert(host_key, result);
    result
}
```

- [ ] **Step 3: Update `stream_proxy` to use `resolve_direct_segments`**

In the `is_playlist` branch of `stream_proxy`, replace:

```rust
let text = String::from_utf8_lossy(&body_bytes);
let rewritten = hls::rewrite_hls_urls(&text, &q.url, false);
```

with:

```rust
let text = String::from_utf8_lossy(&body_bytes);
let direct = resolve_direct_segments(&state, &text, &q.url).await;
let rewritten = hls::rewrite_hls_urls(&text, &q.url, direct);
```

- [ ] **Step 4: Run all tests and confirm passing**

```bash
cargo test 2>&1 | tail -5
```

Expected: all tests pass.

- [ ] **Step 5: Commit**

```bash
cargo fmt
git add src/routes/player.rs
git commit -m "feat: wire CORS detection into stream_proxy handler"
```

---

### Task 6: Add `HealthStatus` / `BudgetStatus` to guide

**Files:**
- Modify: `src/routes/guide.rs`

- [ ] **Step 1: Write failing tests for the new status derivation functions**

At the bottom of the `#[cfg(test)]` block in `src/routes/guide.rs`, replace the four `test_all_sources_down_*` tests with these:

```rust
#[test]
fn test_derive_health_status_live_has_active_source() {
    use std::collections::HashSet;
    let all: HashSet<i64> = [1].into_iter().collect();
    let active: HashSet<i64> = [1].into_iter().collect();
    assert_eq!(
        derive_health_status(1, &ChannelType::Live, &all, &active),
        HealthStatus::Healthy
    );
}

#[test]
fn test_derive_health_status_live_all_inactive() {
    use std::collections::HashSet;
    let all: HashSet<i64> = [1].into_iter().collect();
    let active: HashSet<i64> = HashSet::new();
    assert_eq!(
        derive_health_status(1, &ChannelType::Live, &all, &active),
        HealthStatus::Down
    );
}

#[test]
fn test_derive_health_status_no_sources_unknown() {
    use std::collections::HashSet;
    let all: HashSet<i64> = HashSet::new();
    let active: HashSet<i64> = HashSet::new();
    assert_eq!(
        derive_health_status(1, &ChannelType::Live, &all, &active),
        HealthStatus::Unknown
    );
}

#[test]
fn test_derive_health_status_vod_always_healthy() {
    use std::collections::HashSet;
    let all: HashSet<i64> = [1].into_iter().collect();
    let active: HashSet<i64> = HashSet::new();
    assert_eq!(
        derive_health_status(1, &ChannelType::VodLoop, &all, &active),
        HealthStatus::Healthy
    );
}

#[test]
fn test_derive_budget_status_http_always_proxied() {
    use std::collections::HashMap;
    let mut urls = HashMap::new();
    urls.insert(1i64, "http://example.com/stream.m3u8".to_string());
    assert_eq!(
        derive_budget_status(1, &urls, &HashMap::new()),
        BudgetStatus::Proxied
    );
}

#[test]
fn test_derive_budget_status_https_cache_hit_direct() {
    use std::collections::HashMap;
    let mut urls = HashMap::new();
    urls.insert(1i64, "https://example.com/stream.m3u8".to_string());
    let mut cache = HashMap::new();
    cache.insert("https://example.com".to_string(), true);
    assert_eq!(
        derive_budget_status(1, &urls, &cache),
        BudgetStatus::Direct
    );
}

#[test]
fn test_derive_budget_status_https_cache_hit_proxied() {
    use std::collections::HashMap;
    let mut urls = HashMap::new();
    urls.insert(1i64, "https://example.com/stream.m3u8".to_string());
    let mut cache = HashMap::new();
    cache.insert("https://example.com".to_string(), false);
    assert_eq!(
        derive_budget_status(1, &urls, &cache),
        BudgetStatus::Proxied
    );
}

#[test]
fn test_derive_budget_status_https_cache_miss_unknown() {
    use std::collections::HashMap;
    let mut urls = HashMap::new();
    urls.insert(1i64, "https://example.com/stream.m3u8".to_string());
    assert_eq!(
        derive_budget_status(1, &urls, &HashMap::new()),
        BudgetStatus::Unknown
    );
}

#[test]
fn test_derive_budget_status_no_source_unknown() {
    use std::collections::HashMap;
    assert_eq!(
        derive_budget_status(1, &HashMap::new(), &HashMap::new()),
        BudgetStatus::Unknown
    );
}
```

- [ ] **Step 2: Run tests and confirm they fail**

```bash
cargo test -p mytv -- guide 2>&1 | head -20
```

Expected: compile errors — `HealthStatus`, `BudgetStatus`, and the two `derive_*` functions don't exist yet.

- [ ] **Step 3: Add enums and update `ChannelRow`**

In `src/routes/guide.rs`, replace:

```rust
pub struct ChannelRow {
    pub name: String,
    pub category_icon: &'static str,
    pub all_sources_down: bool,
    pub programs: Vec<ProgramSlot>,
}
```

with:

```rust
#[derive(Debug, Clone, Copy, PartialEq)]
pub enum HealthStatus {
    Healthy,
    Down,
    Unknown,
}

#[derive(Debug, Clone, Copy, PartialEq)]
pub enum BudgetStatus {
    Direct,
    Proxied,
    Unknown,
}

pub struct ChannelRow {
    pub name: String,
    pub category_icon: &'static str,
    pub health_badge_class: &'static str,
    pub health_badge_char: &'static str,
    pub budget_badge_class: &'static str,
    pub budget_badge_char: &'static str,
    pub programs: Vec<ProgramSlot>,
}
```

- [ ] **Step 4: Add `derive_health_status` and `derive_budget_status` functions**

Replace the existing `is_all_sources_down` function with:

```rust
fn derive_health_status(
    channel_id: i64,
    channel_type: &ChannelType,
    all_source_ids: &std::collections::HashSet<i64>,
    active_source_ids: &std::collections::HashSet<i64>,
) -> HealthStatus {
    if !all_source_ids.contains(&channel_id) {
        return HealthStatus::Unknown;
    }
    match channel_type {
        ChannelType::Live => {
            if active_source_ids.contains(&channel_id) {
                HealthStatus::Healthy
            } else {
                HealthStatus::Down
            }
        }
        ChannelType::VodLoop => HealthStatus::Healthy,
    }
}

fn derive_budget_status(
    channel_id: i64,
    first_active_urls: &std::collections::HashMap<i64, String>,
    cors_cache: &std::collections::HashMap<String, bool>,
) -> BudgetStatus {
    let url = match first_active_urls.get(&channel_id) {
        Some(u) => u,
        None => return BudgetStatus::Unknown,
    };
    if url.starts_with("http://") {
        return BudgetStatus::Proxied;
    }
    let after = url.find("://").map(|i| i + 3).unwrap_or(0);
    let host_end = url[after..].find('/').unwrap_or(url[after..].len());
    let host_key = &url[..after + host_end];
    match cors_cache.get(host_key) {
        Some(&true) => BudgetStatus::Direct,
        Some(&false) => BudgetStatus::Proxied,
        None => BudgetStatus::Unknown,
    }
}

fn health_badge(status: HealthStatus) -> (&'static str, &'static str) {
    match status {
        HealthStatus::Healthy => ("health-ok", "●"),
        HealthStatus::Down => ("health-down", "●"),
        HealthStatus::Unknown => ("health-unknown", "○"),
    }
}

fn budget_badge(status: BudgetStatus) -> (&'static str, &'static str) {
    match status {
        BudgetStatus::Direct => ("budget-direct", "⚡"),
        BudgetStatus::Proxied => ("budget-proxied", "☁"),
        BudgetStatus::Unknown => ("budget-unknown", ""),
    }
}
```

- [ ] **Step 5: Update `build_guide_data` signature and body**

Change the signature to accept the CORS cache snapshot:

```rust
async fn build_guide_data(
    pool: &SqlitePool,
    cors_cache: &std::collections::HashMap<String, bool>,
    category: &str,
    offset_hours: i64,
) -> anyhow::Result<GuideData> {
```

Inside `build_guide_data`, after the `active_source_ids` query, add:

```rust
#[derive(sqlx::FromRow)]
struct SourceUrlRow {
    channel_id: i64,
    url: String,
}

let source_url_rows = sqlx::query_as::<_, SourceUrlRow>(
    "SELECT channel_id, url FROM sources WHERE is_active = 1 ORDER BY channel_id, priority",
)
.fetch_all(pool)
.await?;

let first_active_urls: std::collections::HashMap<i64, String> =
    source_url_rows
        .into_iter()
        .fold(std::collections::HashMap::new(), |mut acc, row| {
            acc.entry(row.channel_id).or_insert(row.url);
            acc
        });
```

In the channel loop, replace:

```rust
let all_sources_down = is_all_sources_down(
    ch.id,
    &ch.channel_type(),
    &all_source_ids,
    &active_source_ids,
);
rows.push(ChannelRow {
    name: ch.name.clone(),
    category_icon: category_icon(&ch.category),
    all_sources_down,
    programs,
});
```

with:

```rust
let health = derive_health_status(ch.id, &ch.channel_type(), &all_source_ids, &active_source_ids);
let budget = derive_budget_status(ch.id, &first_active_urls, cors_cache);
let (health_badge_class, health_badge_char) = health_badge(health);
let (budget_badge_class, budget_badge_char) = budget_badge(budget);
rows.push(ChannelRow {
    name: ch.name.clone(),
    category_icon: category_icon(&ch.category),
    health_badge_class,
    health_badge_char,
    budget_badge_class,
    budget_badge_char,
    programs,
});
```

- [ ] **Step 6: Update `guide_page` and `guide_partial` handlers to pass the cache**

In both handlers, change `build_guide_data(&state.pool, &category, offset_hours)` to:

```rust
let cors_snapshot = state.cors_cache.read().await.clone();
let data = build_guide_data(&state.pool, &cors_snapshot, &category, offset_hours)
    .await
    .map_err(|e| { ... })?;
```

- [ ] **Step 7: Run all tests and confirm passing**

```bash
cargo test 2>&1 | tail -5
```

Expected: all tests pass (the 9 new tests plus existing ones).

- [ ] **Step 8: Commit**

```bash
cargo fmt
git add src/routes/guide.rs
git commit -m "feat: add HealthStatus and BudgetStatus to guide channel rows"
```

---

### Task 7: Add CORS probing to health checker

**Files:**
- Modify: `src/health.rs`
- Modify: `src/main.rs`

- [ ] **Step 1: Update `health::start` signature**

In `src/health.rs`, add the import and update `start`:

```rust
use crate::CorsCache;

pub fn start(pool: SqlitePool, client: reqwest::Client, cors_cache: CorsCache) {
    tokio::spawn(async move {
        let mut interval = tokio::time::interval(CHECK_INTERVAL);
        interval.set_missed_tick_behavior(tokio::time::MissedTickBehavior::Skip);
        interval.tick().await;
        loop {
            interval.tick().await;
            check_all(&pool, &client, &cors_cache).await;
        }
    });
}
```

- [ ] **Step 2: Update `check_all` and add `probe_cors_for_source`**

Replace `check_all`:

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
        check_one(pool, client, &src).await;
        if src.url.starts_with("https://") {
            probe_cors_for_source(client, cors_cache, &src).await;
        }
    }
}
```

Add `probe_cors_for_source` and `extract_manifest_host`:

```rust
fn extract_manifest_host(url: &str) -> String {
    let after = url.find("://").map(|i| i + 3).unwrap_or(0);
    let host_end = url[after..].find('/').unwrap_or(url[after..].len());
    url[..after + host_end].to_string()
}

async fn probe_cors_for_source(client: &reqwest::Client, cors_cache: &CorsCache, src: &Source) {
    let body = match client
        .get(&src.url)
        .timeout(HTTP_TIMEOUT)
        .send()
        .await
    {
        Ok(r) if r.status().is_success() => match r.text().await {
            Ok(t) => t,
            Err(_) => return,
        },
        _ => return,
    };

    let segment_url = match crate::media::hls::find_first_segment_url(&body, &src.url) {
        Some(u) => u,
        None => return,
    };

    let result = crate::media::hls::probe_cors(client, &segment_url).await;
    let host_key = extract_manifest_host(&src.url);
    cors_cache.write().await.insert(host_key.clone(), result);
    tracing::debug!(source_id = src.id, host = %host_key, cors = result, "CORS probe cached");
}
```

- [ ] **Step 3: Update `main.rs` call site**

Change:

```rust
health::start(state.pool.clone(), state.http_client.clone());
```

to:

```rust
health::start(state.pool.clone(), state.http_client.clone(), state.cors_cache.clone());
```

- [ ] **Step 4: Run all tests and confirm passing**

```bash
cargo test 2>&1 | tail -5
```

Expected: all tests pass.

- [ ] **Step 5: Commit**

```bash
cargo fmt
git add src/health.rs src/main.rs
git commit -m "feat: probe CORS for HTTPS sources in health checker cycle"
```

---

### Task 8: Update guide template with health and budget badges

**Files:**
- Modify: `templates/partials/epg_content.html`
- Modify: `templates/guide.html` (check if it also renders `channel-col` and apply the same change)
- Modify: `templates/base.html` (add CSS)

- [ ] **Step 1: Update `templates/partials/epg_content.html`**

Replace line 43:

```html
<div class="channel-col">{% if row.all_sources_down %}⚠ {% endif %}{{ row.category_icon }} {{ row.name }}</div>
```

with:

```html
<div class="channel-col">
  <span class="status-badge {{ row.health_badge_class }}">{{ row.health_badge_char }}</span><span class="status-badge {{ row.budget_badge_class }}">{{ row.budget_badge_char }}</span> {{ row.category_icon }} {{ row.name }}
</div>
```

- [ ] **Step 2: Check `templates/guide.html` for a duplicate `channel-col` render**

```bash
grep -n "channel-col\|all_sources_down" templates/guide.html
```

If found, apply the same replacement as Step 1.

- [ ] **Step 3: Add CSS to `templates/base.html`**

Find the existing `<style>` block and add:

```css
.status-badge {
    display: inline-block;
    width: 1.4em;
    text-align: center;
    font-size: 0.72rem;
    border-radius: 3px;
    padding: 1px 2px;
    vertical-align: middle;
    line-height: 1;
}
.health-ok   { color: #4caf50; }
.health-down { color: #e94560; }
.health-unknown { color: #666; }
.budget-direct  { background: #0d2a40; color: #64b5f6; }
.budget-proxied { background: #3a2800; color: #ffb74d; }
.budget-unknown { background: transparent; }
```

- [ ] **Step 4: Build and run**

```bash
cargo build && cargo run
```

Open `http://localhost:3000/guide` in a browser. Confirm:
- Every channel row shows two badge spans with identical width
- Health badge: green dot for channels with active sources, red for all-down, grey circle for no sources
- Budget badge: blue ⚡ for confirmed-direct, amber ☁ for proxied, invisible placeholder for unknown (all rows still aligned)

- [ ] **Step 5: Run full test suite**

```bash
cargo test 2>&1 | tail -5
```

Expected: all tests pass.

- [ ] **Step 6: Commit**

```bash
cargo fmt
git add templates/partials/epg_content.html templates/guide.html templates/base.html
git commit -m "feat: render health and budget badges in EPG guide channel column"
```
