# DRY / Code Health Round 2 — Design

## Problem

Five independent duplication and design-smell findings left over from the previous code-health refactor. All are correctness-neutral — no behaviour change, no schema change.

## Scope

Six tasks (one per finding plus one small SQL extraction). Each is independent.

---

## Finding 1: `ChannelType` and `SourceKind` enums

**Files:** `src/model/channel.rs`, `src/model/source.rs`, `src/routes/admin/channels.rs`, `src/routes/admin/sources.rs`, `src/routes/admin/discover/add.rs`, `src/routes/admin/discover/mod.rs`

### Design

Add `pub enum ChannelType { Live, VodLoop }` to `src/model/channel.rs` and `pub enum SourceKind { Hls, YoutubeLive, Iptv }` to `src/model/source.rs`. Each enum gets:

- `as_str(&self) -> &'static str` — canonical DB string
- `impl FromStr` — parses the canonical string, returns `Err` on unknown value
- `impl Display` — delegates to `as_str()`

`SourceKind` also gets:

```rust
pub fn detect(url: &str) -> Self {
    if resolver::needs_resolution(url) {
        SourceKind::YoutubeLive
    } else if url.contains("iptv") {
        SourceKind::Iptv
    } else {
        SourceKind::Hls
    }
}
```

This replaces the `detect_source_kind` free function in `src/routes/admin/discover/mod.rs`.

### Model layer changes

`create` functions accept the enum instead of `String`:

- `channel::create`: `channel_type: ChannelType` — stores `channel_type.as_str()`; removes the `!["live","vod_loop"].contains(...)` guard (type guarantees validity)
- `source::create`: `kind: SourceKind` — stores `kind.as_str()`; removes the `!["hls","youtube_live","iptv"].contains(...)` guard

### Handler layer changes

All three handler sites parse the form string to enum and pass the enum to the model:

```rust
let channel_type = form.channel_type.parse::<ChannelType>()
    .map_err(|_| StatusCode::UNPROCESSABLE_ENTITY)?;
// channel_create / channel_update: pass channel_type to model::channel::create / update
```

```rust
let kind = form.kind.parse::<SourceKind>()
    .map_err(|_| StatusCode::UNPROCESSABLE_ENTITY)?;
// source_create: pass kind to model::source::create
```

`discover/add.rs` uses `SourceKind::detect(&url)` instead of calling `detect_source_kind`.

---

## Finding 2: CORS pipeline duplication

**Files:** `src/routes/player.rs`

### Design

`resolve_direct_segments` becomes a cache-check + delegate. The `content` parameter is removed (no longer needed):

```rust
async fn resolve_direct_segments(state: &AppState, base_url: &str) -> bool {
    let host_key = hls::extract_manifest_host(base_url);
    {
        let cache = state.cors_cache.read().await;
        if let Some(&cached) = cache.get(&host_key) {
            return cached;
        }
    }
    // Cache miss: delegate to health::probe_and_cache_cors.
    // Re-fetches the manifest internally; acceptable — cache misses are rare (once per host per session).
    health::probe_and_cache_cors(&state.http_client, &state.cors_cache, base_url)
        .await
        .unwrap_or(false)
}
```

The call site in `stream_proxy` drops the `content` / `&text` argument:

```rust
// Before
let direct = resolve_direct_segments(&state, &text, &url).await;
// After
let direct = resolve_direct_segments(&state, &url).await;
```

`health::probe_and_cache_cors` is already `pub` — no visibility change needed.

---

## Finding 3: Duration auto-fetch helper

**Files:** `src/media/mod.rs`, `src/routes/admin/playlist.rs`, `src/routes/admin/discover/add.rs`

### Design

Add to `src/media/mod.rs`:

```rust
pub async fn fetch_duration(client: &reqwest::Client, url: &str) -> Result<i64, anyhow::Error> {
    if resolver::needs_resolution(url) {
        resolver::fetch_duration_secs(url).await
    } else {
        hls::fetch_hls_duration(client, url).await
    }
}
```

Both `playlist_item_create` and `do_discover_add` replace their duplicated branch:

```rust
// Before (both files)
if resolver::needs_resolution(&url) {
    duration_secs = resolver::fetch_duration_secs(&url).await.map_err(|e| { ... })?;
} else {
    duration_secs = hls::fetch_hls_duration(&state.http_client, &url).await.map_err(|e| { ... })?;
}

// After
duration_secs = crate::media::fetch_duration(&state.http_client, &url)
    .await
    .map_err(|e| {
        tracing::warn!(url = %url, error = %e, "failed to auto-fetch duration");
        StatusCode::UNPROCESSABLE_ENTITY
    })?;
```

---

## Finding 4: Guide template macro

**Files:** `src/routes/guide/mod.rs`

### Design

Replace the two struct definitions and the `guide_template!` construction macro with a single `define_guide_template!` macro:

```rust
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

Call sites change from `guide_template!(GuidePageTemplate, data)` to `GuidePageTemplate::from(data)`.

The field list now exists once — in the macro body.

---

## Finding 5: `HealthClients` struct

**Files:** `src/health.rs`, `src/main.rs`

### Design

Add to `src/health.rs`:

```rust
pub struct HealthClients {
    pub pool: SqlitePool,
    pub http_client: reqwest::Client,
    pub cors_cache: CorsCache,
}
```

Change `health::start` signature:

```rust
// Before
pub fn start(pool: SqlitePool, client: reqwest::Client, cors_cache: CorsCache)

// After
pub fn start(clients: HealthClients)
```

Internal references: `pool` → `clients.pool`, `client` → `clients.http_client`, `cors_cache` → `clients.cors_cache`.

`src/main.rs` call site:

```rust
health::start(health::HealthClients {
    pool: pool.clone(),
    http_client: http_client.clone(),
    cors_cache: cors_cache.clone(),
});
```

---

## Finding 6: Inline SQL queries in guide handler

**Files:** `src/routes/guide/data.rs`, `src/model/source.rs`

### Design

The two `sqlx::query_scalar` calls that fetch distinct channel-id sets from `sources` in `build_guide_data` (`guide/data.rs`) are moved to `model/source.rs`:

```rust
// src/model/source.rs
pub async fn channel_ids_with_active_sources(pool: &SqlitePool) -> Result<Vec<i64>> {
    Ok(sqlx::query_scalar("SELECT DISTINCT channel_id FROM sources WHERE is_active = 1")
        .fetch_all(pool)
        .await?)
}

pub async fn channel_ids_with_any_sources(pool: &SqlitePool) -> Result<Vec<i64>> {
    Ok(sqlx::query_scalar("SELECT DISTINCT channel_id FROM sources")
        .fetch_all(pool)
        .await?)
}
```

`build_guide_data` calls these instead of inlining the queries.

---

## Files changed summary

| File | Change |
|------|--------|
| `src/model/channel.rs` | Add `ChannelType` enum; update `create` to accept enum |
| `src/model/source.rs` | Add `SourceKind` enum with `detect()`; update `create`; add 2 query fns |
| `src/routes/admin/channels.rs` | Parse form → `ChannelType` in `channel_create` and `channel_update` |
| `src/routes/admin/sources.rs` | Parse form → `SourceKind` in `source_create` |
| `src/routes/admin/discover/add.rs` | Use `SourceKind::detect()` + `media::fetch_duration` |
| `src/routes/admin/discover/mod.rs` | Remove `detect_source_kind` fn |
| `src/routes/player.rs` | Simplify `resolve_direct_segments`; drop `content` param from call site |
| `src/media/mod.rs` | Add `fetch_duration` helper |
| `src/routes/guide/mod.rs` | Replace two structs + `guide_template!` with `define_guide_template!` |
| `src/health.rs` | Add `HealthClients`; update `start` signature |
| `src/main.rs` | Construct `HealthClients` for `health::start` call |
| `src/routes/guide/data.rs` | Use model fns instead of inline SQL |

No behaviour change. No schema change. All existing tests must continue to pass.
