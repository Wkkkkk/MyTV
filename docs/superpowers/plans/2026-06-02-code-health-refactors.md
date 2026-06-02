# Code-Health Refactors Implementation Plan

> **For agentic workers:** REQUIRED SUB-SKILL: Use superpowers:subagent-driven-development (recommended) or superpowers:executing-plans to implement this plan task-by-task. Steps use checkbox (`- [ ]`) syntax for tracking.

**Goal:** Split the two oversized route modules (`discover.rs`, `guide.rs`) into focused submodules and remove duplication in `player.rs` and `hls.rs` — with zero behavior change.

**Architecture:** Pure refactor. Convert `foo.rs` → `foo/mod.rs` directory modules that re-export the same public names (router wiring and integration tests untouched). Extract shared helpers for duplicated logic. Tests move alongside the code they cover; no test logic changes. The existing 117-test suite is the regression net.

**Tech Stack:** Rust 1.96, Axum 0.7, SQLx 0.7, Askama 0.12.

**Spec:** `docs/superpowers/specs/2026-06-02-code-health-refactors-design.md`

**Conventions for every task:**
- This is a refactor — there is no "write a failing test" step. The verification step is the **full existing suite staying green**.
- Run `cargo fmt` before every commit (CI fails on any diff).
- Commit messages end with the standard `Co-Authored-By` trailer used in this repo.

---

## Task 1: Extract shared URL resolver in `media/hls.rs`

**Files:**
- Modify: `src/media/hls.rs`

The origin/base-dir/"resolve manifest line to absolute URL" logic is copy-pasted in
`rewrite_hls_urls` (lines ~49-59, 67-72), `resolve_uri` (lines ~99-115), and
`find_first_segment_url` (lines ~122-147). `resolve_uri` already encapsulates the
resolution. Collapse the three copies to one.

- [ ] **Step 1: Add a private `origin_of` helper and reuse it**

Add near `resolve_uri` (private fn):

```rust
/// Returns the `scheme://host` prefix of a URL (no path, no query).
fn origin_of(url: &str) -> &str {
    let after_scheme = url.find("://").map(|i| i + 3).unwrap_or(0);
    let host_len = url[after_scheme..]
        .find('/')
        .unwrap_or(url[after_scheme..].len());
    &url[..after_scheme + host_len]
}
```

Rewrite `resolve_uri` to use it:

```rust
/// Resolves a URI from an HLS manifest relative to the manifest's own URL.
fn resolve_uri(uri: &str, base_url: &str) -> String {
    if uri.starts_with("http://") || uri.starts_with("https://") {
        return uri.to_string();
    }
    if uri.starts_with('/') {
        return format!("{}{}", origin_of(base_url), uri);
    }
    let base_dir = base_url
        .rsplit_once('/')
        .map(|(b, _)| b)
        .unwrap_or(base_url);
    format!("{}/{}", base_dir, uri)
}
```

Refactor `extract_manifest_host` to reuse `origin_of` (behavior identical — it returns
an owned `String`):

```rust
/// Extracts `scheme://host` from a URL, stripping any path/query.
/// This is the canonical CORS-cache key (the source-URL host).
pub fn extract_manifest_host(url: &str) -> String {
    origin_of(url).to_string()
}
```

- [ ] **Step 2: Make `rewrite_hls_urls` use `resolve_uri`**

Replace the body's manual origin/base_dir computation and the inline
`if http / else if '/' / else` block with a `resolve_uri` call. The post-resolution
proxy/direct decision stays exactly as-is:

```rust
pub fn rewrite_hls_urls(content: &str, base_url: &str, direct_segments: bool) -> String {
    content
        .lines()
        .map(|line| {
            if line.starts_with('#') || line.is_empty() {
                return line.to_string();
            }
            let abs = resolve_uri(line, base_url);
            let lower = abs.to_lowercase();
            let path = lower.split('?').next().unwrap_or(&lower);
            if direct_segments && !path.ends_with(".m3u8") && !path.ends_with(".m3u") {
                abs
            } else {
                format!("/stream-proxy?url={}", pct_encode(&abs))
            }
        })
        .collect::<Vec<_>>()
        .join("\n")
}
```

- [ ] **Step 3: Make `find_first_segment_url` use `resolve_uri`**

```rust
pub fn find_first_segment_url(content: &str, base_url: &str) -> Option<String> {
    for line in content.lines() {
        if line.starts_with('#') || line.is_empty() {
            continue;
        }
        let lower = line.to_lowercase();
        let path = lower.split('?').next().unwrap_or(&lower);
        if path.ends_with(".m3u8") || path.ends_with(".m3u") {
            continue;
        }
        return Some(resolve_uri(line, base_url));
    }
    None
}
```

- [ ] **Step 4: Verify the suite stays green**

Run: `cargo test --lib media::hls`
Expected: all hls tests PASS (rewrite, resolve_uri, find_first_segment, manifest host).

Then full check:
Run: `cargo test && cargo clippy -- -D warnings`
Expected: PASS, no warnings.

- [ ] **Step 5: Commit**

```bash
cargo fmt
git add src/media/hls.rs
git commit -m "$(printf 'refactor: extract shared URL resolver in media/hls.rs\n\nCo-Authored-By: Claude Opus 4.8 (1M context) <noreply@anthropic.com>')"
```

---

## Task 2: Dedupe `TuneResponse` builders in `routes/player.rs`

**Files:**
- Modify: `src/routes/player.rs:69-205`

Four near-identical builders. Collapse to one response constructor + shared live/vod
helpers.

- [ ] **Step 1: Add the `tune_response` constructor**

Add above `tune_live` (note `channel::Channel` is already imported as `ch`'s type):

```rust
fn tune_response(
    ch: &channel::Channel,
    url: String,
    start_offset_secs: i64,
) -> Json<TuneResponse> {
    Json(TuneResponse {
        url,
        start_offset_secs,
        name: ch.name.clone(),
        logo_url: ch.logo_url.clone(),
        category: ch.category.clone(),
        channel_type: ch.r#type.clone(),
    })
}
```

- [ ] **Step 2: Collapse `tune_live` into `next_live`**

`tune_live` differs from `next_live` only by the `failed_url` filter (a no-op when
`None`). Replace both with:

```rust
async fn next_live(
    state: &AppState,
    ch: &channel::Channel,
    failed_url: Option<&str>,
) -> Result<Json<TuneResponse>, StatusCode> {
    let sources = source::list_active_for_channel(&state.pool, ch.id)
        .await
        .map_err(|_| StatusCode::INTERNAL_SERVER_ERROR)?;

    for src in sources
        .iter()
        .filter(|s| Some(s.url.as_str()) != failed_url)
    {
        match resolver::resolve_url(&src.url).await {
            Ok(url) => return Ok(tune_response(ch, url, 0)),
            Err(e) => {
                tracing::warn!(url = %src.url, error = %e, "resolver failed, trying next source")
            }
        }
    }
    Err(StatusCode::SERVICE_UNAVAILABLE)
}
```

Delete the old `tune_live` fn. Update its sole caller in `tune` (around line 32-48):
find `tune_live(&state, &ch)` and replace with `next_live(&state, &ch, None)`.

- [ ] **Step 3: Extract the shared VOD position prelude**

Add a helper that both VOD paths share (the empty-playlist 503 and position lookup):

```rust
async fn vod_items_and_index(
    state: &AppState,
    ch: &channel::Channel,
    now_secs: i64,
) -> Result<(Vec<playlist_item::PlaylistItem>, usize, i64), StatusCode> {
    let anchor_secs = ch
        .loop_anchor
        .ok_or(StatusCode::INTERNAL_SERVER_ERROR)?
        .timestamp();

    let items = playlist_item::list_for_channel(&state.pool, ch.id)
        .await
        .map_err(|_| StatusCode::INTERNAL_SERVER_ERROR)?;

    if items.is_empty() {
        return Err(StatusCode::SERVICE_UNAVAILABLE);
    }

    let (idx, offset) = playlist_item::current_position(&items, now_secs, anchor_secs)
        .ok_or(StatusCode::SERVICE_UNAVAILABLE)?;
    Ok((items, idx, offset))
}
```

- [ ] **Step 4: Rewrite the two VOD builders on top of the helper**

```rust
async fn tune_vod_at(
    state: &AppState,
    ch: &channel::Channel,
    now_secs: i64,
) -> Result<Json<TuneResponse>, StatusCode> {
    let (items, idx, offset) = vod_items_and_index(state, ch, now_secs).await?;
    let item = &items[idx];
    match resolver::resolve_url(&item.url).await {
        Ok(url) => Ok(tune_response(ch, url, offset)),
        Err(e) => {
            tracing::warn!(url = %item.url, error = %e, "resolver failed for vod item");
            Err(StatusCode::SERVICE_UNAVAILABLE)
        }
    }
}

async fn next_vod_at(
    state: &AppState,
    ch: &channel::Channel,
    now_secs: i64,
) -> Result<Json<TuneResponse>, StatusCode> {
    let (items, idx, _) = vod_items_and_index(state, ch, now_secs).await?;
    let next_idx = (idx + 1) % items.len();
    let item = &items[next_idx];
    match resolver::resolve_url(&item.url).await {
        Ok(url) => Ok(tune_response(ch, url, 0)),
        Err(e) => {
            tracing::warn!(url = %item.url, error = %e, "resolver failed for vod item");
            Err(StatusCode::SERVICE_UNAVAILABLE)
        }
    }
}
```

Confirm `playlist_item` is imported at the top of the file (it is — used by the existing
VOD fns). If `tune_response`'s return is used in a `Result::Ok`, no extra `Json(...)`
wrapping is needed since the helper already returns `Json<...>`.

- [ ] **Step 5: Verify**

Run: `cargo test --lib routes::player && cargo test --test http`
Expected: PASS — covers channels 1 (live OK), 2 (all down → 503), 3 (fallback / failed_url
→ 503), 4 (VOD has items → 200), 5 (VOD empty → 503).

Run: `cargo test && cargo clippy -- -D warnings`
Expected: PASS, no warnings.

- [ ] **Step 6: Commit**

```bash
cargo fmt
git add src/routes/player.rs
git commit -m "$(printf 'refactor: dedupe TuneResponse builders in routes/player.rs\n\nCo-Authored-By: Claude Opus 4.8 (1M context) <noreply@anthropic.com>')"
```

---

## Task 3: Split `routes/admin/discover.rs` into a `discover/` module

**Files:**
- Create: `src/routes/admin/discover/mod.rs`
- Create: `src/routes/admin/discover/add.rs`
- Create: `src/routes/admin/discover/youtube.rs`
- Create: `src/routes/admin/discover/m3u.rs`
- Delete: `src/routes/admin/discover.rs`

This is a mechanical move. Cargo treats `discover.rs` and `discover/mod.rs` as the same
module path, so `routes::admin::discover::*` and the `pub use discover::{...}` in
`routes/admin/mod.rs` keep working as long as the names are re-exported.

- [ ] **Step 1: Create `discover/add.rs`**

Move `DiscoverAddParams` (lines 366-377), `do_discover_add` (379-481), and the 6
`do_discover_add` tests (`test_add_*` in the test module: lines 725-889) here. Header:

```rust
use axum::http::StatusCode;
use chrono::Utc;

use crate::routes::internal_error;
use crate::{
    media::{hls, resolver},
    model::{channel, playlist_item, source},
};

// ... DiscoverAddParams + do_discover_add bodies, verbatim ...

#[cfg(test)]
mod tests {
    use super::*;
    use crate::{db, model::{channel, playlist_item, source}};
    use axum::http::StatusCode;
    use chrono::Utc;

    async fn test_pool() -> sqlx::SqlitePool {
        db::connect("sqlite::memory:").await.unwrap()
    }
    // ... the 6 test_add_* fns, verbatim ...
}
```

- [ ] **Step 2: Create `discover/youtube.rs`**

Move `YoutubeResultRow` (34-41), `parse_iso8601_duration` (339-362),
`fetch_youtube_results` (485-573), and the `test_parse_iso8601_duration` test (716-723).
Header:

```rust
pub struct YoutubeResultRow {
    pub title: String,
    pub channel_title: String,
    pub is_live: bool,
    pub duration_secs: i64,
    pub url: String,
    pub form_id: usize,
}

pub fn parse_iso8601_duration(s: &str) -> i64 { /* verbatim */ }

pub(super) async fn fetch_youtube_results(
    keyword: &str,
    api_key: &str,
    client: &reqwest::Client,
) -> anyhow::Result<Vec<YoutubeResultRow>> { /* verbatim */ }

#[cfg(test)]
mod tests {
    use super::*;
    #[test]
    fn test_parse_iso8601_duration() { /* verbatim */ }
}
```

`parse_iso8601_duration` stays `pub` (it was `pub`); `fetch_youtube_results` becomes
`pub(super)` (only the handler in mod.rs calls it).

- [ ] **Step 3: Create `discover/m3u.rs`**

Move `M3uResultRow` (25-32), `country_to_code` (587-669), `url_is_reachable` (575-585),
`fetch_m3u` (671-684). Header:

```rust
pub struct M3uResultRow {
    pub name: String,
    pub group: String,
    pub country: String,
    pub url: String,
    pub source_kind: String,
    pub form_id: usize,
}

pub(super) async fn fetch_m3u(
    client: &reqwest::Client,
    country_code: Option<&str>,
) -> anyhow::Result<String> { /* verbatim */ }

pub(super) async fn url_is_reachable(client: &reqwest::Client, url: &str) -> bool { /* verbatim */ }

pub(super) fn country_to_code(input: &str) -> Option<String> { /* verbatim */ }
```

- [ ] **Step 4: Create `discover/mod.rs`**

Holds: submodule declarations + re-exports, `DiscoverChannelOption`, all template structs,
all form/query types, `detect_source_kind` + its test, and the 6 handlers. The handlers
reference moved items via the submodule paths (`add::do_discover_add`,
`add::DiscoverAddParams`, `youtube::fetch_youtube_results`, `m3u::*`). Top of file:

```rust
mod add;
mod m3u;
mod youtube;

pub use add::{do_discover_add, DiscoverAddParams};
pub use youtube::parse_iso8601_duration;

use askama::Template;
use axum::{
    extract::{Form, State},
    http::StatusCode,
    response::{Html, IntoResponse, Redirect},
};
use serde::Deserialize;

use crate::routes::{internal_error, render};
use crate::{
    media::m3u as media_m3u,
    media::resolver,
    model::channel,
    AppState,
};

use m3u::{M3uResultRow};
use youtube::{YoutubeResultRow};
```

Then move verbatim: `DiscoverChannelOption`, the 5 template structs, the form/query
structs, `detect_source_kind`, and the 6 handler fns. Within handlers, update call sites:
- `discover_m3u_search`: `fetch_m3u` → `m3u::fetch_m3u`, `country_to_code` →
  `m3u::country_to_code`, `url_is_reachable` → `m3u::url_is_reachable`. Note it also uses
  `crate::media::m3u::{parse_m3u, filter_m3u}` — keep those via the `media_m3u` alias to
  avoid colliding with the local `m3u` submodule (`media_m3u::parse_m3u`,
  `media_m3u::filter_m3u`).
- `discover_youtube_search`: `fetch_youtube_results` → `youtube::fetch_youtube_results`.
- `discover_add`: `do_discover_add` / `DiscoverAddParams` are in scope via the `pub use`.

Move the `test_detect_source_kind` test into a `#[cfg(test)] mod tests` in mod.rs:

```rust
#[cfg(test)]
mod tests {
    use super::*;
    #[test]
    fn test_detect_source_kind() { /* verbatim */ }
}
```

- [ ] **Step 5: Delete the old file**

```bash
git rm src/routes/admin/discover.rs
```

(The four new files plus the `mod discover;` line already in `routes/admin/mod.rs:2`
provide the module.)

- [ ] **Step 6: Verify**

Run: `cargo build 2>&1 | head -40`
Expected: clean build. Fix any import-path errors (most likely: the `media_m3u` alias, or
a missing `pub(super)`/`use`).

Run: `cargo test discover && cargo test && cargo clippy -- -D warnings`
Expected: all PASS (the 6 add tests, parse_iso8601, detect_source_kind), no warnings.

- [ ] **Step 7: Commit**

```bash
cargo fmt
git add src/routes/admin/
git commit -m "$(printf 'refactor: split discover.rs into focused submodules\n\nCo-Authored-By: Claude Opus 4.8 (1M context) <noreply@anthropic.com>')"
```

---

## Task 4: Split `routes/guide.rs` into a `guide/` module

**Files:**
- Create: `src/routes/guide/mod.rs`
- Create: `src/routes/guide/layout.rs`
- Create: `src/routes/guide/badges.rs`
- Create: `src/routes/guide/data.rs`
- Delete: `src/routes/guide.rs`

- [ ] **Step 1: Create `guide/layout.rs`**

Move the pure geometry + display types: `ProgramSlot` (24-30), `TimeLabel` (32-35),
`compute_window` (190-196), `entry_to_slot` (198-218), `now_line_pct` (220-231),
`time_labels` (233-255), and their tests (the geometry tests in the test module:
`test_compute_window_*`, `test_entry_to_slot_*`, `test_now_line_pct_*`, `test_time_labels_*`,
plus the `dt`, `make_entry`, `w` helpers). Header:

```rust
use chrono::{DateTime, Utc};
use crate::epg;

pub struct ProgramSlot {
    pub title: String,
    pub is_live: bool,
    pub left_pct: f64,
    pub width_pct: f64,
    pub channel_id: i64,
}

pub struct TimeLabel {
    pub label: String,
    pub left_pct: f64,
}

// compute_window, entry_to_slot, now_line_pct, time_labels — verbatim, all pub

#[cfg(test)]
mod tests {
    use super::*;
    fn dt(secs: i64) -> DateTime<Utc> { DateTime::from_timestamp(secs, 0).unwrap() }
    fn make_entry(channel_id: i64, start: i64, end: i64, is_live: bool) -> epg::ProgramEntry { /* verbatim */ }
    fn w() -> (DateTime<Utc>, DateTime<Utc>) { (dt(0), dt(14400)) }
    // test_compute_window_*, test_entry_to_slot_*, test_now_line_pct_*, test_time_labels_* — verbatim
}
```

- [ ] **Step 2: Create `guide/badges.rs`**

Move `HealthStatus` (37-42), `category_icon` (54-87), `derive_health_status` (89-108),
`budget_for_url` (110-118), `vod_budget_url` (120-138), `health_badge` (140-146), and
their tests (`test_category_icon_*`, `test_derive_health_status_*`, `test_budget_for_url_*`,
`test_vod_budget_url_*`, plus the `mk_item` helper). Header:

```rust
use chrono::{DateTime, Utc};

use crate::{
    budget::{status_for_url, BudgetStatus},
    model::playlist_item,
    model::channel::ChannelType,
};

#[derive(Debug, Clone, Copy, PartialEq)]
pub enum HealthStatus {
    Healthy,
    Down,
    Unknown,
}

// category_icon, derive_health_status, budget_for_url, vod_budget_url, health_badge — verbatim
// (all pub(super) except keep signatures as-is; they are crate-internal helpers)

#[cfg(test)]
mod tests {
    use super::*;
    use std::collections::HashMap;
    fn mk_item(url: &str, dur: i64) -> playlist_item::PlaylistItem { /* verbatim */ }
    // test_category_icon_known_categories, test_derive_health_status_*,
    // test_budget_for_url_*, test_vod_budget_url_* — verbatim
}
```

Note: `derive_health_status` takes `&ChannelType`; import `ChannelType` here. The
functions can be `pub(super)` since only `data.rs` (sibling) and tests call them — use
`pub(crate)` or `pub(super)`; `pub(super)` won't reach a sibling submodule, so use
`pub(crate)` OR re-export. **Use `pub(crate)`** for `category_icon`,
`derive_health_status`, `budget_for_url`, `vod_budget_url`, `health_badge`, and
`HealthStatus` so `data.rs` can call them.

- [ ] **Step 3: Create `guide/data.rs`**

Move `ChannelRow` (44-52), `GuideData` (259-270), `build_guide_data` (272-398). Header:

```rust
use chrono::Utc;
use sqlx::SqlitePool;

use crate::{
    budget::budget_badge,
    epg,
    model::{
        channel::{self, Channel, ChannelType},
        playlist_item,
    },
};

use super::badges::{
    budget_for_url, category_icon, derive_health_status, health_badge, vod_budget_url,
};
use super::layout::{compute_window, entry_to_slot, now_line_pct, time_labels, ProgramSlot, TimeLabel};

pub struct ChannelRow {
    pub name: String,
    pub category_icon: &'static str,
    pub health_badge_class: &'static str,
    pub health_badge_char: &'static str,
    pub budget_badge_class: &'static str,
    pub budget_badge_char: &'static str,
    pub programs: Vec<ProgramSlot>,
}

pub(super) struct GuideData {
    pub categories: Vec<String>,
    pub active_category: String,
    pub offset_hours: i64,
    pub offset_prev: i64,
    pub offset_next: i64,
    pub window_label: String,
    pub labels: Vec<TimeLabel>,
    pub now_pct: Option<f64>,
    pub rows: Vec<ChannelRow>,
    pub channels_json: String,
}

pub(super) async fn build_guide_data(
    pool: &SqlitePool,
    cors_cache: &std::collections::HashMap<String, bool>,
    category: &str,
    offset_hours: i64,
) -> anyhow::Result<GuideData> { /* verbatim body */ }
```

The `GuideData` fields must become `pub`/`pub(super)` (they are bare today because struct
and consumer were in one file). The handlers in mod.rs read every field, so mark them
`pub(super)`. `build_guide_data` uses `serde_json` — keep `use serde_json;` or call it via
the fully-qualified path; the body already references `serde_json::` so add the import.

- [ ] **Step 4: Create `guide/mod.rs` with the param + template dedup**

```rust
mod badges;
mod data;
mod layout;

pub use layout::{compute_window, entry_to_slot, now_line_pct, time_labels};

use askama::Template;
use axum::{
    extract::{Query, State},
    http::StatusCode,
    response::Html,
};
use serde::Deserialize;

use crate::AppState;

use data::{build_guide_data, ChannelRow, GuideData};
use layout::TimeLabel;

#[derive(Template)]
#[template(path = "guide.html")]
struct GuidePageTemplate {
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

#[derive(Template)]
#[template(path = "partials/epg_content.html")]
struct EpgContentTemplate {
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

#[derive(Debug, Deserialize)]
pub struct GuideQuery {
    pub category: Option<String>,
    pub offset: Option<i64>,
}

fn parse_query(params: GuideQuery) -> (String, i64) {
    let category = params.category.unwrap_or_else(|| "all".to_string());
    let offset_hours = params.offset.unwrap_or(-2).clamp(-48, 48);
    (category, offset_hours)
}

/// Builds a guide template of type `$t` from a `GuideData`, moving every field across.
macro_rules! guide_template {
    ($t:ident, $d:expr) => {{
        let d = $d;
        $t {
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
    }};
}

async fn load_data(state: &AppState, params: GuideQuery) -> Result<GuideData, StatusCode> {
    let (category, offset_hours) = parse_query(params);
    let cors_snapshot = state.cors_cache.read().await.clone();
    build_guide_data(&state.pool, &cors_snapshot, &category, offset_hours)
        .await
        .map_err(|e| {
            tracing::error!("guide data error: {e}");
            StatusCode::INTERNAL_SERVER_ERROR
        })
}

fn render_or_500<T: Template>(t: T) -> Result<Html<String>, StatusCode> {
    t.render().map(Html).map_err(|e| {
        tracing::error!("template render error: {e}");
        StatusCode::INTERNAL_SERVER_ERROR
    })
}

pub async fn guide_page(
    State(state): State<AppState>,
    Query(params): Query<GuideQuery>,
) -> Result<Html<String>, StatusCode> {
    let data = load_data(&state, params).await?;
    render_or_500(guide_template!(GuidePageTemplate, data))
}

pub async fn guide_partial(
    State(state): State<AppState>,
    Query(params): Query<GuideQuery>,
) -> Result<Html<String>, StatusCode> {
    let data = load_data(&state, params).await?;
    render_or_500(guide_template!(EpgContentTemplate, data))
}
```

This preserves byte-identical output: the rendered template structs carry the same fields,
and the param defaults/clamps are unchanged.

- [ ] **Step 5: Delete the old file**

```bash
git rm src/routes/guide.rs
```

(`routes/mod.rs` already declares `pub mod guide;` — confirm with
`grep -n "mod guide" src/routes/mod.rs`; it resolves to `guide/mod.rs` automatically.)

- [ ] **Step 6: Verify**

Run: `cargo build 2>&1 | head -40`
Expected: clean. Likely fixups: visibility (`pub(crate)` on badge helpers,
`pub(super)` on GuideData fields), the `serde_json` import in data.rs.

Run: `cargo test guide && cargo test && cargo clippy -- -D warnings`
Expected: all geometry + badge tests PASS, integration guide route tests PASS, no warnings.

Manual sanity (optional): `cargo run` then load `/guide` and `/guide/partial?offset=2` —
the HTML should be unchanged.

- [ ] **Step 7: Commit**

```bash
cargo fmt
git add src/routes/
git commit -m "$(printf 'refactor: split guide.rs into focused submodules\n\nCo-Authored-By: Claude Opus 4.8 (1M context) <noreply@anthropic.com>')"
```

---

## Task 5: Final whole-suite verification

**Files:** none (verification only).

- [ ] **Step 1: Full green check**

Run: `cargo fmt --check && cargo clippy -- -D warnings && cargo test`
Expected: no fmt diff, no clippy warnings, all 117 tests PASS.

- [ ] **Step 2: Confirm line-count reduction & no orphan files**

Run: `wc -l src/routes/admin/discover/*.rs src/routes/guide/*.rs src/routes/player.rs src/media/hls.rs`
Expected: each new module file is meaningfully smaller than the originals; no
`discover.rs` / `guide.rs` remain (`ls src/routes/admin/discover.rs src/routes/guide.rs`
should report "No such file").

- [ ] **Step 3: Mark idea 14 done**

Edit `docs/IDEAS.md` line 25: prefix item 14 with `~~` strikethrough and append a
one-line `— done:` summary noting the four splits/dedups, matching the style of items
10-12. Commit:

```bash
cargo fmt
git add docs/IDEAS.md
git commit -m "$(printf 'docs: mark idea 14 (code-health refactors) done\n\nCo-Authored-By: Claude Opus 4.8 (1M context) <noreply@anthropic.com>')"
```

---

## Self-review notes

- **Spec coverage:** Part 1 (hls) → Task 1; Part 3 (player) → Task 2; Part 1-discover →
  Task 3; Part 2 (guide) incl. both micro-dedups → Task 4; verification → Task 5. All four
  spec parts mapped.
- **Type consistency:** `tune_response`, `vod_items_and_index`, `origin_of`, `parse_query`,
  `load_data`, `render_or_500`, and the `guide_template!` macro are each defined once and
  referenced with matching signatures.
- **Visibility:** badge helpers + `HealthStatus` are `pub(crate)` (sibling submodule
  access); `GuideData` fields and `build_guide_data` are `pub(super)`; discover submodule
  helpers are `pub(super)` except the already-`pub` `parse_iso8601_duration`,
  `do_discover_add`, `DiscoverAddParams` (re-exported from mod.rs).
