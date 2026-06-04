# DRY / Code Health Round 2 Implementation Plan

> **For agentic workers:** REQUIRED SUB-SKILL: Use superpowers:subagent-driven-development (recommended) or superpowers:executing-plans to implement this plan task-by-task. Steps use checkbox (`- [ ]`) syntax for tracking.

**Goal:** Eliminate six independent duplication and design-smell findings: type-safe enums for channel/source kinds, a shared duration-fetch helper, a simplified CORS probe path, a DRY guide-template macro, a `HealthClients` struct, and SQL query extraction.

**Architecture:** All six tasks are correctness-neutral refactors — no behaviour change, no schema change, no new dependencies. Each task is independently committable. Verification for every task: `cargo test` passes (all 215 tests).

**Tech Stack:** Rust 1.96, Axum 0.7, SQLx 0.7, Askama 0.12.

---

### Task 1: `ChannelType` extensions + `SourceKind` enum

**Files:**
- Modify: `src/model/channel.rs`
- Modify: `src/model/source.rs`

`ChannelType` already exists in `src/model/channel.rs` as a bare enum. Task 1 adds `FromStr`, `as_str()`, and `Display` to it, then changes `NewChannel.channel_type` and `UpdateChannel.channel_type` from `String` to `ChannelType`, removing the string-allowlist guard from `create`. It also adds the brand-new `SourceKind` enum to `src/model/source.rs` with the same traits plus a `detect()` method, and changes `NewSource.kind` from `String` to `SourceKind`.

Both model files have existing tests; some must be updated because the `NewChannel` / `NewSource` constructors change.

- [ ] **Step 1: Extend `ChannelType` in `src/model/channel.rs`**

Add `as_str`, `FromStr`, and `Display` immediately after the existing `Channel::channel_type` impl (after line 32):

```rust
impl ChannelType {
    pub fn as_str(&self) -> &'static str {
        match self {
            ChannelType::Live => "live",
            ChannelType::VodLoop => "vod_loop",
        }
    }
}

impl std::str::FromStr for ChannelType {
    type Err = anyhow::Error;
    fn from_str(s: &str) -> std::result::Result<Self, Self::Err> {
        match s {
            "live" => Ok(ChannelType::Live),
            "vod_loop" => Ok(ChannelType::VodLoop),
            _ => anyhow::bail!("invalid channel_type: {s}"),
        }
    }
}

impl std::fmt::Display for ChannelType {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        f.write_str(self.as_str())
    }
}
```

- [ ] **Step 2: Change `NewChannel.channel_type` and `UpdateChannel.channel_type` to `ChannelType`**

Replace:
```rust
pub struct NewChannel {
    pub name: String,
    pub category: String,
    pub logo_url: Option<String>,
    pub channel_type: String,
    pub sort_order: i64,
    pub loop_anchor: Option<DateTime<Utc>>,
}
```
with:
```rust
pub struct NewChannel {
    pub name: String,
    pub category: String,
    pub logo_url: Option<String>,
    pub channel_type: ChannelType,
    pub sort_order: i64,
    pub loop_anchor: Option<DateTime<Utc>>,
}
```

Replace:
```rust
pub struct UpdateChannel {
    pub name: String,
    pub category: String,
    pub logo_url: Option<String>,
    pub channel_type: String,
    pub sort_order: i64,
    pub loop_anchor: Option<DateTime<Utc>>,
}
```
with:
```rust
pub struct UpdateChannel {
    pub name: String,
    pub category: String,
    pub logo_url: Option<String>,
    pub channel_type: ChannelType,
    pub sort_order: i64,
    pub loop_anchor: Option<DateTime<Utc>>,
}
```

- [ ] **Step 3: Update `create` and `update` in `src/model/channel.rs`**

In `create`, remove the allowlist guard and change the `bind`:

```rust
pub async fn create(pool: &SqlitePool, input: NewChannel) -> Result<Channel> {
    let id = sqlx::query(
        "INSERT INTO channels (name, category, logo_url, type, sort_order, loop_anchor)
         VALUES (?, ?, ?, ?, ?, ?)",
    )
    .bind(&input.name)
    .bind(&input.category)
    .bind(&input.logo_url)
    .bind(input.channel_type.as_str())
    .bind(input.sort_order)
    .bind(input.loop_anchor)
    .execute(pool)
    .await?
    .last_insert_rowid();

    get(pool, id)
        .await?
        .ok_or_else(|| anyhow::anyhow!("channel not found after insert"))
}
```

In `update`, change the `bind`:

```rust
pub async fn update(pool: &SqlitePool, id: i64, input: UpdateChannel) -> Result<Option<Channel>> {
    let rows = sqlx::query(
        "UPDATE channels SET name = ?, category = ?, logo_url = ?, type = ?, sort_order = ?, loop_anchor = ? WHERE id = ?",
    )
    .bind(&input.name)
    .bind(&input.category)
    .bind(&input.logo_url)
    .bind(input.channel_type.as_str())
    .bind(input.sort_order)
    .bind(input.loop_anchor)
    .bind(id)
    .execute(pool)
    .await?
    .rows_affected();

    if rows == 0 {
        return Ok(None);
    }
    get(pool, id).await
}
```

- [ ] **Step 4: Update `model/channel.rs` tests to use enum**

In the `#[cfg(test)]` block, change the `live()` helper:

```rust
fn live(name: &str, category: &str) -> NewChannel {
    NewChannel {
        name: name.to_string(),
        category: category.to_string(),
        logo_url: None,
        channel_type: ChannelType::Live,
        sort_order: 0,
        loop_anchor: None,
    }
}
```

Update the two `UpdateChannel` constructors in the test functions to use `channel_type: ChannelType::Live` instead of `channel_type: "live".to_string()`:

In `test_update_channel_name_and_category`:
```rust
UpdateChannel {
    name: "CNN International".to_string(),
    category: "world".to_string(),
    logo_url: None,
    channel_type: ChannelType::Live,
    sort_order: 1,
    loop_anchor: None,
}
```

In `test_update_nonexistent_channel_returns_none`:
```rust
UpdateChannel {
    name: "Ghost".to_string(),
    category: "none".to_string(),
    logo_url: None,
    channel_type: ChannelType::Live,
    sort_order: 0,
    loop_anchor: None,
}
```

- [ ] **Step 5: Add `SourceKind` to `src/model/source.rs`**

Add after the `use` imports at the top of `src/model/source.rs` (before the `Source` struct):

```rust
/// Source media kind.
#[derive(Debug, Clone, PartialEq)]
pub enum SourceKind {
    Hls,
    YoutubeLive,
    Iptv,
}

impl SourceKind {
    pub fn as_str(&self) -> &'static str {
        match self {
            SourceKind::Hls => "hls",
            SourceKind::YoutubeLive => "youtube_live",
            SourceKind::Iptv => "iptv",
        }
    }

    /// Infers the kind from a URL using the same rules as the discover UI.
    pub fn detect(url: &str) -> Self {
        if url.contains("youtube.com") || url.contains("youtu.be") {
            SourceKind::YoutubeLive
        } else if url.contains(".m3u8") {
            SourceKind::Hls
        } else {
            SourceKind::Iptv
        }
    }
}

impl std::str::FromStr for SourceKind {
    type Err = anyhow::Error;
    fn from_str(s: &str) -> std::result::Result<Self, Self::Err> {
        match s {
            "hls" => Ok(SourceKind::Hls),
            "youtube_live" => Ok(SourceKind::YoutubeLive),
            "iptv" => Ok(SourceKind::Iptv),
            _ => anyhow::bail!("invalid source kind: {s}"),
        }
    }
}

impl std::fmt::Display for SourceKind {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        f.write_str(self.as_str())
    }
}
```

- [ ] **Step 6: Change `NewSource.kind` to `SourceKind` and update `create`**

Replace:
```rust
pub struct NewSource {
    pub channel_id: i64,
    pub kind: String,
    pub url: String,
    pub priority: i64,
}
```
with:
```rust
pub struct NewSource {
    pub channel_id: i64,
    pub kind: SourceKind,
    pub url: String,
    pub priority: i64,
}
```

In `create`, remove the allowlist guard and change the `bind`:

```rust
pub async fn create(pool: &SqlitePool, input: NewSource) -> Result<Source> {
    let id = sqlx::query(
        "INSERT INTO sources (channel_id, kind, url, priority, is_active) VALUES (?, ?, ?, ?, 1)",
    )
    .bind(input.channel_id)
    .bind(input.kind.as_str())
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
```

- [ ] **Step 7: Run the test suite**

```bash
cargo test
```

Expected: compile errors in handler files that construct `NewChannel` / `NewSource` with `String` fields. The model tests should pass. Fix compile errors by proceeding to Task 3, or if you want the tree to compile at this step, temporarily add `channel_type: "live".parse().unwrap()` at each call site and remove after Task 3.

> Note: it is acceptable for the tree to not compile between Task 1 and Task 3. Tasks 1–3 form a logical unit. Commit Task 1 only if the tree compiles.

- [ ] **Step 8: Run fmt and commit**

```bash
cargo fmt
git add src/model/channel.rs src/model/source.rs
git commit -m "refactor: add ChannelType/SourceKind enums with FromStr, as_str, detect"
```

---

### Task 2: `media::fetch_duration` helper + `playlist_item_create` update

**Files:**
- Modify: `src/media/mod.rs`
- Modify: `src/routes/admin/playlist.rs`

`src/media/mod.rs` currently only declares three submodules. We add a single public `fetch_duration` function that abstracts the resolver/HLS branch used in both playlist and discover handlers.

- [ ] **Step 1: Add `fetch_duration` to `src/media/mod.rs`**

Replace the entire file:

```rust
pub mod hls;
pub mod m3u;
pub mod resolver;

/// Fetches the duration (seconds) for a VOD URL.
/// Uses yt-dlp resolution for YouTube/resolvable URLs, HLS manifest parsing otherwise.
pub async fn fetch_duration(client: &reqwest::Client, url: &str) -> anyhow::Result<i64> {
    if resolver::needs_resolution(url) {
        resolver::fetch_duration_secs(url).await
    } else {
        hls::fetch_hls_duration(client, url).await
    }
}
```

- [ ] **Step 2: Update `playlist_item_create` in `src/routes/admin/playlist.rs`**

Change the import at the top of the file from:

```rust
use crate::{
    media::{hls, resolver},
    model::{playlist_item, playlist_item::NewPlaylistItem},
    AppState,
};
```

to:

```rust
use crate::{
    media,
    model::{playlist_item, playlist_item::NewPlaylistItem},
    AppState,
};
```

Replace the duration-fetch block (lines 41–55):

```rust
    if duration_secs <= 0 {
        if resolver::needs_resolution(&url) {
            duration_secs = resolver::fetch_duration_secs(&url).await.map_err(|e| {
                tracing::warn!(url = %url, error = %e, "failed to auto-fetch duration");
                StatusCode::UNPROCESSABLE_ENTITY
            })?;
        } else {
            duration_secs = hls::fetch_hls_duration(&state.http_client, &url)
                .await
                .map_err(|e| {
                    tracing::warn!(url = %url, error = %e, "failed to fetch HLS duration");
                    StatusCode::UNPROCESSABLE_ENTITY
                })?;
        }
    }
```

with:

```rust
    if duration_secs <= 0 {
        duration_secs = media::fetch_duration(&state.http_client, &url)
            .await
            .map_err(|e| {
                tracing::warn!(url = %url, error = %e, "failed to auto-fetch duration");
                StatusCode::UNPROCESSABLE_ENTITY
            })?;
    }
```

- [ ] **Step 3: Run the test suite**

```bash
cargo test
```

Expected: all tests pass.

- [ ] **Step 4: Run fmt and commit**

```bash
cargo fmt
git add src/media/mod.rs src/routes/admin/playlist.rs
git commit -m "refactor: extract media::fetch_duration helper, use in playlist_item_create"
```

---

### Task 3: Handler call-site updates — channels, sources, discover

**Files:**
- Modify: `src/routes/admin/channels.rs`
- Modify: `src/routes/admin/sources.rs`
- Modify: `src/routes/admin/discover/add.rs`
- Modify: `src/routes/admin/discover/mod.rs`

This task updates all handler sites that construct `NewChannel`, `UpdateChannel`, or `NewSource` to use the enums from Task 1, and replaces the `detect_source_kind` free function with `SourceKind::detect`. It also uses `media::fetch_duration` in `do_discover_add`.

- [ ] **Step 1: Update `channel_create` in `src/routes/admin/channels.rs`**

Replace this block (starting from the allowlist check):

```rust
    if !["live", "vod_loop"].contains(&form.channel_type.as_str()) {
        return Err(StatusCode::UNPROCESSABLE_ENTITY);
    }
    if form.name.trim().is_empty() || form.category.trim().is_empty() {
        return Err(StatusCode::UNPROCESSABLE_ENTITY);
    }
    let sort_order: i64 = form.sort_order.trim().parse().unwrap_or(0);
    let logo_url = if form.logo_url.trim().is_empty() {
        None
    } else {
        Some(form.logo_url.trim().to_string())
    };
    let loop_anchor = if form.channel_type.as_str() == "vod_loop" {
        parse_loop_anchor(&form.loop_anchor).or_else(|| Some(Utc::now()))
    } else {
        None
    };

    channel::create(
        &state.pool,
        channel::NewChannel {
            name: form.name.trim().to_string(),
            category: form.category.trim().to_string(),
            logo_url,
            channel_type: form.channel_type.clone(),
            sort_order,
            loop_anchor,
        },
    )
```

with:

```rust
    let channel_type = form.channel_type.parse::<channel::ChannelType>()
        .map_err(|_| StatusCode::UNPROCESSABLE_ENTITY)?;
    if form.name.trim().is_empty() || form.category.trim().is_empty() {
        return Err(StatusCode::UNPROCESSABLE_ENTITY);
    }
    let sort_order: i64 = form.sort_order.trim().parse().unwrap_or(0);
    let logo_url = if form.logo_url.trim().is_empty() {
        None
    } else {
        Some(form.logo_url.trim().to_string())
    };
    let loop_anchor = if channel_type == channel::ChannelType::VodLoop {
        parse_loop_anchor(&form.loop_anchor).or_else(|| Some(Utc::now()))
    } else {
        None
    };

    channel::create(
        &state.pool,
        channel::NewChannel {
            name: form.name.trim().to_string(),
            category: form.category.trim().to_string(),
            logo_url,
            channel_type,
            sort_order,
            loop_anchor,
        },
    )
```

- [ ] **Step 2: Update `channel_update` in `src/routes/admin/channels.rs`**

Replace this block (starting from the allowlist check):

```rust
    if !["live", "vod_loop"].contains(&form.channel_type.as_str()) {
        return Err(StatusCode::UNPROCESSABLE_ENTITY);
    }
    if form.name.trim().is_empty() || form.category.trim().is_empty() {
        return Err(StatusCode::UNPROCESSABLE_ENTITY);
    }
    let existing = channel::get(&state.pool, id)
        .await
        .map_err(internal_error)?
        .ok_or(StatusCode::NOT_FOUND)?;

    let sort_order: i64 = form.sort_order.trim().parse().unwrap_or(0);
    let logo_url = if form.logo_url.trim().is_empty() {
        None
    } else {
        Some(form.logo_url.trim().to_string())
    };
    let loop_anchor = if form.channel_type.as_str() == "vod_loop" {
        parse_loop_anchor(&form.loop_anchor).or(existing.loop_anchor)
    } else {
        None
    };

    channel::update(
        &state.pool,
        id,
        channel::UpdateChannel {
            name: form.name.trim().to_string(),
            category: form.category.trim().to_string(),
            logo_url,
            channel_type: form.channel_type.clone(),
            sort_order,
            loop_anchor,
        },
    )
```

with:

```rust
    let channel_type = form.channel_type.parse::<channel::ChannelType>()
        .map_err(|_| StatusCode::UNPROCESSABLE_ENTITY)?;
    if form.name.trim().is_empty() || form.category.trim().is_empty() {
        return Err(StatusCode::UNPROCESSABLE_ENTITY);
    }
    let existing = channel::get(&state.pool, id)
        .await
        .map_err(internal_error)?
        .ok_or(StatusCode::NOT_FOUND)?;

    let sort_order: i64 = form.sort_order.trim().parse().unwrap_or(0);
    let logo_url = if form.logo_url.trim().is_empty() {
        None
    } else {
        Some(form.logo_url.trim().to_string())
    };
    let loop_anchor = if channel_type == channel::ChannelType::VodLoop {
        parse_loop_anchor(&form.loop_anchor).or(existing.loop_anchor)
    } else {
        None
    };

    channel::update(
        &state.pool,
        id,
        channel::UpdateChannel {
            name: form.name.trim().to_string(),
            category: form.category.trim().to_string(),
            logo_url,
            channel_type,
            sort_order,
            loop_anchor,
        },
    )
```

- [ ] **Step 3: Update `source_create` in `src/routes/admin/sources.rs`**

Replace:

```rust
    if !["hls", "youtube_live", "iptv"].contains(&form.kind.as_str()) {
        return Err(StatusCode::UNPROCESSABLE_ENTITY);
    }
    if form.url.trim().is_empty() {
        return Err(StatusCode::UNPROCESSABLE_ENTITY);
    }
    let priority: i64 = form.priority.trim().parse().unwrap_or(1);
    source::create(
        &state.pool,
        source::NewSource {
            channel_id,
            kind: form.kind.clone(),
            url: form.url.trim().to_string(),
            priority,
        },
    )
```

with:

```rust
    let kind = form.kind.parse::<source::SourceKind>()
        .map_err(|_| StatusCode::UNPROCESSABLE_ENTITY)?;
    if form.url.trim().is_empty() {
        return Err(StatusCode::UNPROCESSABLE_ENTITY);
    }
    let priority: i64 = form.priority.trim().parse().unwrap_or(1);
    source::create(
        &state.pool,
        source::NewSource {
            channel_id,
            kind,
            url: form.url.trim().to_string(),
            priority,
        },
    )
```

Also add the `source` import to the `use crate::model::` line if not already present.

- [ ] **Step 4: Update `do_discover_add` in `src/routes/admin/discover/add.rs`**

At the top of `do_discover_add`, replace the two allowlist guards and update the `NewChannel` / `NewSource` constructors. Change this section:

```rust
    if url.is_empty() || (!url.starts_with("http://") && !url.starts_with("https://")) {
        return Err(StatusCode::UNPROCESSABLE_ENTITY);
    }
    if !["hls", "youtube_live", "iptv"].contains(&source_kind) {
        return Err(StatusCode::UNPROCESSABLE_ENTITY);
    }
```

to:

```rust
    if url.is_empty() || (!url.starts_with("http://") && !url.starts_with("https://")) {
        return Err(StatusCode::UNPROCESSABLE_ENTITY);
    }
    let source_kind = source_kind.parse::<source::SourceKind>()
        .map_err(|_| StatusCode::UNPROCESSABLE_ENTITY)?;
```

Replace:

```rust
        if !["live", "vod_loop"].contains(&new_channel_type) {
            return Err(StatusCode::UNPROCESSABLE_ENTITY);
        }
        let loop_anchor = if new_channel_type == "vod_loop" {
            Some(Utc::now())
        } else {
            None
        };
        let ch = channel::create(
            pool,
            channel::NewChannel {
                name: new_name.trim().to_string(),
                category: new_category.trim().to_string(),
                logo_url: None,
                channel_type: new_channel_type.to_string(),
                sort_order: 0,
                loop_anchor,
            },
        )
```

with:

```rust
        let new_channel_type = new_channel_type.parse::<channel::ChannelType>()
            .map_err(|_| StatusCode::UNPROCESSABLE_ENTITY)?;
        let loop_anchor = if new_channel_type == channel::ChannelType::VodLoop {
            Some(Utc::now())
        } else {
            None
        };
        let ch = channel::create(
            pool,
            channel::NewChannel {
                name: new_name.trim().to_string(),
                category: new_category.trim().to_string(),
                logo_url: None,
                channel_type: new_channel_type,
                sort_order: 0,
                loop_anchor,
            },
        )
```

Replace the `if ch.channel_type() == channel::ChannelType::VodLoop` duration-fetch block:

```rust
        if duration_secs <= 0 {
            if resolver::needs_resolution(url) {
                duration_secs = resolver::fetch_duration_secs(url).await.map_err(|e| {
                    tracing::warn!(url = %url, error = %e, "failed to auto-fetch duration in discover_add");
                    StatusCode::UNPROCESSABLE_ENTITY
                })?;
            } else {
                duration_secs = hls::fetch_hls_duration(client, url).await.map_err(|e| {
                    tracing::warn!(url = %url, error = %e, "failed to fetch HLS duration in discover_add");
                    StatusCode::UNPROCESSABLE_ENTITY
                })?;
            }
        }
```

with:

```rust
        if duration_secs <= 0 {
            duration_secs = crate::media::fetch_duration(client, url)
                .await
                .map_err(|e| {
                    tracing::warn!(url = %url, error = %e, "failed to auto-fetch duration");
                    StatusCode::UNPROCESSABLE_ENTITY
                })?;
        }
```

Replace the `source::create` call:

```rust
        source::create(
            pool,
            source::NewSource {
                channel_id,
                kind: source_kind.to_string(),
                url: url.to_string(),
                priority: 0,
            },
        )
```

with:

```rust
        source::create(
            pool,
            source::NewSource {
                channel_id,
                kind: source_kind,
                url: url.to_string(),
                priority: 0,
            },
        )
```

Remove any now-unused imports (`hls`, `resolver`) from the import block in `discover/add.rs`.

- [ ] **Step 5: Replace `detect_source_kind` in `src/routes/admin/discover/mod.rs`**

Grep for all call sites of `detect_source_kind` in the file:

```bash
grep -n "detect_source_kind" src/routes/admin/discover/mod.rs
```

At each call site, replace `detect_source_kind(some_url)` with `source::SourceKind::detect(some_url).as_str()`. Make sure `source` is imported: `use crate::model::source;`.

Then delete the entire `detect_source_kind` function:

```rust
pub fn detect_source_kind(url: &str) -> &'static str {
    if url.contains("youtube.com") || url.contains("youtu.be") {
        "youtube_live"
    } else if url.contains(".m3u8") {
        "hls"
    } else {
        "iptv"
    }
}
```

- [ ] **Step 6: Run the test suite**

```bash
cargo test
```

Expected: all tests pass.

- [ ] **Step 7: Run fmt and commit**

```bash
cargo fmt
git add src/routes/admin/channels.rs src/routes/admin/sources.rs \
        src/routes/admin/discover/add.rs src/routes/admin/discover/mod.rs
git commit -m "refactor: use ChannelType/SourceKind enums in handlers, remove string allowlist guards"
```

---

### Task 4: CORS pipeline simplification

**Files:**
- Modify: `src/routes/player.rs`

`resolve_direct_segments` currently re-implements the probe-and-cache pipeline. After this task it becomes a cache-check + delegate to `health::probe_and_cache_cors`. The `content: &str` parameter is removed since it is no longer used.

- [ ] **Step 1: Replace `resolve_direct_segments` in `src/routes/player.rs`**

Replace the entire function (lines 178–199):

```rust
async fn resolve_direct_segments(state: &AppState, content: &str, base_url: &str) -> bool {
    let host_key = hls::extract_manifest_host(base_url);
    {
        let cache = state.cors_cache.read().await;
        if let Some(&cached) = cache.get(&host_key) {
            return cached;
        }
    }
    let segment_url =
        match hls::find_segment_with_descent(&state.http_client, content, base_url).await {
            Some(u) => u,
            None => return false,
        };
    if !segment_url.starts_with("https://") {
        state.cors_cache.write().await.insert(host_key, false);
        return false;
    }
    let result = hls::probe_cors(&state.http_client, &segment_url).await;
    tracing::debug!(host = %host_key, cors = result, "CORS probe result cached");
    state.cors_cache.write().await.insert(host_key, result);
    result
}
```

with:

```rust
async fn resolve_direct_segments(state: &AppState, base_url: &str) -> bool {
    let host_key = crate::media::hls::extract_manifest_host(base_url);
    {
        let cache = state.cors_cache.read().await;
        if let Some(&cached) = cache.get(&host_key) {
            return cached;
        }
    }
    // Cache miss: delegate to health::probe_and_cache_cors.
    // Re-fetches the manifest internally; cache misses are rare (once per host per session).
    crate::health::probe_and_cache_cors(&state.http_client, &state.cors_cache, base_url)
        .await
        .unwrap_or(false)
}
```

- [ ] **Step 2: Update the call site in `stream_proxy`**

Find the call to `resolve_direct_segments` inside `stream_proxy` and remove the `&text` argument:

```rust
// Before
let direct = resolve_direct_segments(&state, &text, &url).await;

// After
let direct = resolve_direct_segments(&state, &url).await;
```

- [ ] **Step 3: Remove unused imports from `src/routes/player.rs`**

If `hls` is no longer imported at the top of the file for any other use in the module scope (check the rest of the file), remove or narrow the import. If `hls` is still used elsewhere, leave the import.

Run:
```bash
cargo clippy -- -D warnings 2>&1 | grep player
```

Fix any unused-import warnings.

- [ ] **Step 4: Run the test suite**

```bash
cargo test
```

Expected: all tests pass.

- [ ] **Step 5: Run fmt and commit**

```bash
cargo fmt
git add src/routes/player.rs
git commit -m "refactor: simplify resolve_direct_segments to cache-check + probe_and_cache_cors delegate"
```

---

### Task 5: Guide template macro

**Files:**
- Modify: `src/routes/guide/mod.rs`

Replace the two identical struct definitions (`GuidePageTemplate`, `EpgContentTemplate`) and the `guide_template!` construction macro with a single `define_guide_template!` macro that generates both the struct and a `From<GuideData>` impl. The field list lives exactly once — in the macro body.

- [ ] **Step 1: Replace the template structs and macro in `src/routes/guide/mod.rs`**

Replace lines 18–81 (the two `#[derive(Template)]` structs and the `guide_template!` macro):

```rust
// ── template structs ───────────────────────────────────────────────────────

macro_rules! define_guide_template {
    ($name:ident, $path:literal) => {
        #[derive(Template)]
        #[template(path = $path)]
        struct $name {
            categories: Vec<String>,
            active_category: String,
            offset_hours: i64,
            offset_prev: i64,
            offset_next: i64,
            window_label: String,
            labels: Vec<TimeLabel>,
            now_pct: Option<f64>,
            rows: Vec<ChannelRow>,
            channels_json: String,
        }

        impl From<GuideData> for $name {
            fn from(d: GuideData) -> Self {
                Self {
                    categories: d.categories,
                    active_category: d.active_category,
                    offset_hours: d.offset_hours,
                    offset_prev: d.offset_prev,
                    offset_next: d.offset_next,
                    window_label: d.window_label,
                    labels: d.labels,
                    now_pct: d.now_pct,
                    rows: d.rows,
                    channels_json: d.channels_json,
                }
            }
        }
    };
}

define_guide_template!(GuidePageTemplate, "guide.html");
define_guide_template!(EpgContentTemplate, "partials/epg_content.html");
```

- [ ] **Step 2: Update the two handler call sites**

In `guide_page` (line 108):
```rust
// Before
render_or_500(guide_template!(GuidePageTemplate, data))

// After
render_or_500(GuidePageTemplate::from(data))
```

In `guide_partial` (line 116):
```rust
// Before
render_or_500(guide_template!(EpgContentTemplate, data))

// After
render_or_500(EpgContentTemplate::from(data))
```

- [ ] **Step 3: Run the test suite**

```bash
cargo test
```

Expected: all tests pass.

- [ ] **Step 4: Run fmt and commit**

```bash
cargo fmt
git add src/routes/guide/mod.rs
git commit -m "refactor: replace duplicate guide template structs with define_guide_template! macro"
```

---

### Task 6: `HealthClients` struct

**Files:**
- Modify: `src/health.rs`
- Modify: `src/main.rs`

`health::start` currently takes three separate arguments. This task adds a `HealthClients` struct and changes the signature to accept it, making it easier to add or remove health-check dependencies in future without breaking all call sites.

- [ ] **Step 1: Add `HealthClients` and update `start` in `src/health.rs`**

Add `HealthClients` before the `start` function. Also update the `start` signature and body.

Add (before `pub fn start`):

```rust
/// Dependencies for the background health checker.
pub struct HealthClients {
    pub pool: SqlitePool,
    pub http_client: reqwest::Client,
    pub cors_cache: CorsCache,
}
```

Replace `pub fn start`:

```rust
pub fn start(clients: HealthClients) {
    tokio::spawn(async move {
        let mut interval = tokio::time::interval(CHECK_INTERVAL);
        interval.set_missed_tick_behavior(tokio::time::MissedTickBehavior::Skip);
        interval.tick().await;
        loop {
            interval.tick().await;
            check_all(&clients.pool, &clients.http_client, &clients.cors_cache).await;
        }
    });
}
```

- [ ] **Step 2: Update the call site in `src/main.rs`**

Find the `health::start(` call. Replace:

```rust
    health::start(
        state.pool.clone(),
        state.http_client.clone(),
        state.cors_cache.clone(),
    );
```

with:

```rust
    health::start(health::HealthClients {
        pool: state.pool.clone(),
        http_client: state.http_client.clone(),
        cors_cache: state.cors_cache.clone(),
    });
```

- [ ] **Step 3: Run the test suite**

```bash
cargo test
```

Expected: all tests pass.

- [ ] **Step 4: Run fmt and commit**

```bash
cargo fmt
git add src/health.rs src/main.rs
git commit -m "refactor: introduce HealthClients struct for health::start dependencies"
```

---

### Task 7: SQL query extraction to `model/source.rs`

**Files:**
- Modify: `src/model/source.rs`
- Modify: `src/routes/guide/data.rs`

Two `sqlx::query_scalar` calls that fetch distinct channel-id sets from `sources` are inlined in `build_guide_data`. They belong in `model/source.rs` alongside the other source queries.

- [ ] **Step 1: Add two query functions to `src/model/source.rs`**

Append before the `#[cfg(test)]` block (or at the end of the public API section):

```rust
/// Returns the set of channel IDs that have at least one source (active or not).
pub async fn channel_ids_with_any_sources(pool: &SqlitePool) -> Result<std::collections::HashSet<i64>> {
    sqlx::query_scalar::<_, i64>("SELECT DISTINCT channel_id FROM sources")
        .fetch_all(pool)
        .await
        .map(|v| v.into_iter().collect())
        .map_err(Into::into)
}

/// Returns the set of channel IDs that have at least one active source.
pub async fn channel_ids_with_active_sources(pool: &SqlitePool) -> Result<std::collections::HashSet<i64>> {
    sqlx::query_scalar::<_, i64>("SELECT DISTINCT channel_id FROM sources WHERE is_active = 1")
        .fetch_all(pool)
        .await
        .map(|v| v.into_iter().collect())
        .map_err(Into::into)
}
```

- [ ] **Step 2: Update `build_guide_data` in `src/routes/guide/data.rs`**

Find the two inline `sqlx::query_scalar` calls (around lines 73–85) and replace them:

```rust
// Before
let all_source_ids: std::collections::HashSet<i64> =
    sqlx::query_scalar::<_, i64>("SELECT DISTINCT channel_id FROM sources")
        .fetch_all(pool)
        .await?
        .into_iter()
        .collect();

let active_source_ids: std::collections::HashSet<i64> =
    sqlx::query_scalar::<_, i64>("SELECT DISTINCT channel_id FROM sources WHERE is_active = 1")
        .fetch_all(pool)
        .await?
        .into_iter()
        .collect();

// After
let all_source_ids = source::channel_ids_with_any_sources(pool).await?;
let active_source_ids = source::channel_ids_with_active_sources(pool).await?;
```

Make sure `source` is imported in `data.rs`: `use crate::model::source;`. Remove the `sqlx` import if it is no longer used directly in `data.rs` after this change (check with `cargo clippy`).

- [ ] **Step 3: Run the test suite**

```bash
cargo test
```

Expected: all tests pass.

- [ ] **Step 4: Run fmt and commit**

```bash
cargo fmt
git add src/model/source.rs src/routes/guide/data.rs
git commit -m "refactor: move distinct source channel-id queries to model/source.rs"
```
