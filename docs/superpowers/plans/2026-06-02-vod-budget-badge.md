# VOD CORS Budget Badge Implementation Plan

> **For agentic workers:** REQUIRED SUB-SKILL: Use superpowers:subagent-driven-development (recommended) or superpowers:executing-plans to implement this plan task-by-task. Steps use checkbox (`- [ ]`) syntax for tracking.

**Goal:** Show the network-budget badge (⚡/☁) for VOD channels in the guide and admin, deriving it from playlist-item URLs and populating the CORS cache for those hosts.

**Architecture:** Extract the CORS-probe-and-cache logic into one shared helper. The guide derives a VOD channel's badge from its *currently-playing* playlist item. A per-item admin Test button and a new background-checker sweep keep the CORS cache warm for VOD item hosts.

**Tech Stack:** Rust, Axum 0.7, SQLx 0.7 (SQLite), Askama 0.12, HTMX, reqwest.

**Spec:** `docs/superpowers/specs/2026-06-02-vod-budget-badge-design.md`

**Conventions for every commit in this plan:**
- Run `cargo fmt` before committing (CI fails on any diff; toolchain pinned to 1.96).
- Append this trailer to every commit message:
  `Co-Authored-By: Claude Opus 4.8 (1M context) <noreply@anthropic.com>`

---

## File Structure

- `src/health.rs` — new `probe_and_cache_cors` helper (Task 1); new `probe_all_playlist_cors` background step (Task 5); `check_source` refactored to delegate.
- `src/model/playlist_item.rs` — new `list_all` query (Task 2).
- `src/routes/guide.rs` — replace `derive_budget_status` with `budget_for_url` + `vod_budget_url`; refactor the per-channel loop (Task 3).
- `src/routes/admin/mod.rs` — `AdminPlaylistItemRow` gains budget fields + `apply_budget`; export `playlist_item_test` (Task 4).
- `src/routes/admin/channels.rs` — `channel_detail` applies budget to playlist items (Task 4).
- `src/routes/admin/playlist.rs` — new `playlist_item_test` handler (Task 4).
- `templates/admin/partials/playlist_item_row.html` — new row partial with Budget cell + Test button (Task 4).
- `templates/admin/channel_detail.html` — Budget column header + include the partial (Task 4).
- `src/lib.rs` — register `POST /admin/playlist/:id/test` (Task 4).
- `tests/http.rs` — guide VOD badge test (Task 3); playlist Test button test (Task 4).
- `docs/IDEAS.md` — mark idea 12 done (Task 5).

---

## Task 1: Shared `probe_and_cache_cors` helper

**Files:**
- Modify: `src/health.rs` (add helper; refactor `check_source`; add tests)

- [ ] **Step 1: Write the failing tests**

Add to the `#[cfg(test)] mod tests` block in `src/health.rs`:

```rust
    #[tokio::test]
    async fn test_probe_and_cache_cors_skips_non_https() {
        let cache: crate::CorsCache =
            std::sync::Arc::new(tokio::sync::RwLock::new(std::collections::HashMap::new()));
        let client = reqwest::Client::new();
        let result = probe_and_cache_cors(&client, &cache, "http://x.example.com/s.m3u8").await;
        assert_eq!(result, None);
        assert!(cache.read().await.is_empty());
    }

    #[tokio::test]
    async fn test_probe_and_cache_cors_skips_resolution_needed() {
        let cache: crate::CorsCache =
            std::sync::Arc::new(tokio::sync::RwLock::new(std::collections::HashMap::new()));
        let client = reqwest::Client::new();
        let result =
            probe_and_cache_cors(&client, &cache, "https://youtube.com/watch?v=abc").await;
        assert_eq!(result, None);
        assert!(cache.read().await.is_empty());
    }
```

- [ ] **Step 2: Run tests to verify they fail**

Run: `cargo test --lib health::tests::test_probe_and_cache_cors`
Expected: FAIL — `cannot find function probe_and_cache_cors in this scope`.

- [ ] **Step 3: Add the helper**

In `src/health.rs`, after the `check_source` function, add:

```rust
/// Probes CORS for one URL and caches the result keyed by host. Returns `None`
/// (a no-op, leaving the cache unchanged) for non-HTTPS URLs or resolution-needed
/// (youtube/twitch) URLs, which have no stable HLS manifest to probe.
pub async fn probe_and_cache_cors(
    client: &reqwest::Client,
    cors_cache: &CorsCache,
    url: &str,
) -> Option<bool> {
    if !url.starts_with("https://") || crate::media::resolver::needs_resolution(url) {
        return None;
    }
    let result = crate::media::hls::probe_source_cors(client, url).await?;
    let host = crate::media::hls::extract_manifest_host(url);
    cors_cache.write().await.insert(host.clone(), result);
    tracing::debug!(host = %host, cors = result, "CORS probe cached");
    Some(result)
}
```

`CorsCache` is already in scope — `src/health.rs` imports `use crate::CorsCache;` at the top.

- [ ] **Step 4: Refactor `check_source` to delegate**

In `src/health.rs`, replace this block at the end of `check_source`:

```rust
    // Only probe CORS for reachable HTTPS sources: a down source would just
    // incur a second timeout, and its cached budget is best left as-is.
    if ok && src.url.starts_with("https://") {
        if let Some(result) = crate::media::hls::probe_source_cors(client, &src.url).await {
            let host_key = crate::media::hls::extract_manifest_host(&src.url);
            cors_cache.write().await.insert(host_key.clone(), result);
            tracing::debug!(source_id = src.id, host = %host_key, cors = result, "CORS probe cached");
        }
    }
```

with:

```rust
    // Only probe CORS for reachable sources: a down source would just incur a
    // second timeout, and its cached budget is best left as-is. The helper itself
    // skips non-HTTPS and resolution-needed URLs.
    if ok {
        probe_and_cache_cors(client, cors_cache, &src.url).await;
    }
```

- [ ] **Step 5: Run tests to verify they pass**

Run: `cargo test --lib health::`
Expected: PASS (all health tests, including the two new ones).

- [ ] **Step 6: Commit**

```bash
cargo fmt
git add src/health.rs
git commit -m "refactor: extract probe_and_cache_cors helper from check_source

Co-Authored-By: Claude Opus 4.8 (1M context) <noreply@anthropic.com>"
```

---

## Task 2: `playlist_item::list_all` query

**Files:**
- Modify: `src/model/playlist_item.rs` (add `list_all`; add test)

- [ ] **Step 1: Write the failing test**

Add to the `#[cfg(test)] mod tests` block in `src/model/playlist_item.rs`:

```rust
    #[tokio::test]
    async fn test_list_all_returns_items_across_channels() {
        let pool = test_pool().await;
        let ch = make_channel(&pool).await;
        create(&pool, item(ch.id, "ep1", 1800, 0)).await.unwrap();
        create(&pool, item(ch.id, "ep2", 2400, 1)).await.unwrap();

        let all = list_all(&pool).await.unwrap();
        assert_eq!(all.len(), 2);
    }
```

- [ ] **Step 2: Run test to verify it fails**

Run: `cargo test --lib playlist_item::tests::test_list_all`
Expected: FAIL — `cannot find function list_all in this scope`.

- [ ] **Step 3: Add the query**

In `src/model/playlist_item.rs`, after `list_for_channel`, add:

```rust
pub async fn list_all(pool: &SqlitePool) -> Result<Vec<PlaylistItem>> {
    sqlx::query_as::<_, PlaylistItem>(
        "SELECT * FROM playlist_items ORDER BY channel_id, sort_order ASC",
    )
    .fetch_all(pool)
    .await
    .map_err(Into::into)
}
```

- [ ] **Step 4: Run test to verify it passes**

Run: `cargo test --lib playlist_item::tests::test_list_all`
Expected: PASS.

- [ ] **Step 5: Commit**

```bash
cargo fmt
git add src/model/playlist_item.rs
git commit -m "feat: add playlist_item::list_all query

Co-Authored-By: Claude Opus 4.8 (1M context) <noreply@anthropic.com>"
```

---

## Task 3: Guide derivation for VOD (currently-playing item)

**Files:**
- Modify: `src/routes/guide.rs` (replace `derive_budget_status`; add `vod_budget_url`; refactor loop; update tests)
- Test: `tests/http.rs` (guide VOD badge integration test)

- [ ] **Step 1: Write the failing unit tests**

In `src/routes/guide.rs`, replace the existing `test_derive_budget_status_no_source_unknown` test with these, and add the `mk_item` helper inside `mod tests` (next to the existing `dt` helper):

```rust
    fn mk_item(url: &str, dur: i64) -> playlist_item::PlaylistItem {
        playlist_item::PlaylistItem {
            id: 0,
            channel_id: 1,
            title: "t".into(),
            url: url.into(),
            duration_secs: dur,
            sort_order: 0,
        }
    }

    #[test]
    fn test_budget_for_url_none_is_unknown() {
        use std::collections::HashMap;
        assert_eq!(budget_for_url(None, &HashMap::new()), BudgetStatus::Unknown);
    }

    #[test]
    fn test_budget_for_url_http_is_proxied() {
        use std::collections::HashMap;
        assert_eq!(
            budget_for_url(Some("http://x.example.com/s.m3u8"), &HashMap::new()),
            BudgetStatus::Proxied
        );
    }

    #[test]
    fn test_budget_for_url_https_cache_hit_direct() {
        use std::collections::HashMap;
        let mut cache = HashMap::new();
        cache.insert("https://x.example.com".to_string(), true);
        assert_eq!(
            budget_for_url(Some("https://x.example.com/s.m3u8"), &cache),
            BudgetStatus::Direct
        );
    }

    #[test]
    fn test_vod_budget_url_empty_is_none() {
        assert_eq!(vod_budget_url(&[], None, dt(0)), None);
    }

    #[test]
    fn test_vod_budget_url_no_anchor_uses_first_item() {
        let items = vec![mk_item("https://a/1.mp4", 100), mk_item("https://b/2.mp4", 100)];
        assert_eq!(
            vod_budget_url(&items, None, dt(150)).as_deref(),
            Some("https://a/1.mp4")
        );
    }

    #[test]
    fn test_vod_budget_url_uses_currently_playing_item() {
        let items = vec![mk_item("https://a/1.mp4", 100), mk_item("https://b/2.mp4", 100)];
        // anchor=0, now=150 → 150s into the loop → second item (after the first 100s)
        assert_eq!(
            vod_budget_url(&items, Some(dt(0)), dt(150)).as_deref(),
            Some("https://b/2.mp4")
        );
    }
```

- [ ] **Step 2: Run tests to verify they fail**

Run: `cargo test --lib guide::tests::test_budget_for_url guide::tests::test_vod_budget_url`
Expected: FAIL — `cannot find function budget_for_url` / `vod_budget_url`.

- [ ] **Step 3: Replace `derive_budget_status` with the two new helpers**

In `src/routes/guide.rs`, replace the `derive_budget_status` function (around lines 110-119):

```rust
fn derive_budget_status(
    channel_id: i64,
    first_active_urls: &std::collections::HashMap<i64, String>,
    cors_cache: &std::collections::HashMap<String, bool>,
) -> BudgetStatus {
    match first_active_urls.get(&channel_id) {
        Some(url) => status_for_url(url, cors_cache),
        None => BudgetStatus::Unknown,
    }
}
```

with:

```rust
fn budget_for_url(
    url: Option<&str>,
    cors_cache: &std::collections::HashMap<String, bool>,
) -> BudgetStatus {
    match url {
        Some(u) => status_for_url(u, cors_cache),
        None => BudgetStatus::Unknown,
    }
}

/// The URL whose host determines a VOD channel's guide budget badge: the
/// currently-playing item (via the loop anchor), falling back to the first item
/// when there is no anchor. `None` for an empty playlist.
fn vod_budget_url(
    items: &[playlist_item::PlaylistItem],
    anchor: Option<DateTime<Utc>>,
    now: DateTime<Utc>,
) -> Option<String> {
    if items.is_empty() {
        return None;
    }
    let idx = match anchor {
        Some(a) => playlist_item::current_position(items, now.timestamp(), a.timestamp())
            .map(|(i, _)| i)
            .unwrap_or(0),
        None => 0,
    };
    Some(items[idx].url.clone())
}
```

- [ ] **Step 4: Refactor the per-channel loop in `build_guide_data`**

In `src/routes/guide.rs`, replace the loop body (around lines 316-351) that begins `let mut rows = Vec::new();` through the `rows.push(ChannelRow { ... });` block with:

```rust
    let mut rows = Vec::new();
    for ch in &channels {
        let (entries, budget_url) = match ch.channel_type() {
            ChannelType::Live => (
                vec![epg::live_entry(ch.id, &ch.name, window_start, window_end)],
                first_active_urls.get(&ch.id).cloned(),
            ),
            ChannelType::VodLoop => {
                let items = playlist_item::list_for_channel(pool, ch.id).await?;
                let entries = match ch.loop_anchor {
                    Some(anchor) => {
                        epg::vod_schedule(ch.id, &items, anchor.timestamp(), window_start, window_end)
                    }
                    None => vec![],
                };
                let budget_url = vod_budget_url(&items, ch.loop_anchor, now);
                (entries, budget_url)
            }
        };
        let programs: Vec<ProgramSlot> = entries
            .iter()
            .filter_map(|e| entry_to_slot(e, window_start, window_end))
            .collect();
        let health = derive_health_status(
            ch.id,
            &ch.channel_type(),
            &all_source_ids,
            &active_source_ids,
        );
        let budget = budget_for_url(budget_url.as_deref(), cors_cache);
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
    }
```

- [ ] **Step 5: Run unit tests to verify they pass**

Run: `cargo test --lib guide::`
Expected: PASS.

- [ ] **Step 6: Write the failing integration test**

Add to `tests/http.rs`, after `test_guide_renders_direct_budget_badge_from_cache`:

```rust
#[tokio::test]
async fn test_guide_renders_vod_budget_badge_from_cache() {
    // Channel 4 (VOD) plays items hosted on https://vod.example.com. Only that
    // host is seeded into the cache, so the lightning badge must come from VOD.
    let response = app_with_cors("https://vod.example.com", true)
        .await
        .oneshot(req("/guide"))
        .await
        .unwrap();
    assert_eq!(response.status(), StatusCode::OK);
    let body = body_text(response).await;
    assert!(
        body.contains("\u{26A1}"),
        "guide should show the direct budget badge (lightning) for the VOD channel"
    );
}
```

- [ ] **Step 7: Run the integration test**

Run: `cargo test --test http test_guide_renders_vod_budget_badge_from_cache`
Expected: PASS.

- [ ] **Step 8: Commit**

```bash
cargo fmt
git add src/routes/guide.rs tests/http.rs
git commit -m "feat: derive VOD guide budget badge from currently-playing item

Co-Authored-By: Claude Opus 4.8 (1M context) <noreply@anthropic.com>"
```

---

## Task 4: Per-item Test button + Budget column (admin)

**Files:**
- Modify: `src/routes/admin/mod.rs` (`AdminPlaylistItemRow` fields + `apply_budget` + `From` default; export handler)
- Modify: `src/routes/admin/channels.rs` (`channel_detail` applies budget to items)
- Modify: `src/routes/admin/playlist.rs` (new `playlist_item_test` handler)
- Create: `templates/admin/partials/playlist_item_row.html`
- Modify: `templates/admin/channel_detail.html` (Budget column + include partial)
- Modify: `src/lib.rs` (register route)
- Test: `tests/http.rs` (Test button integration test)

- [ ] **Step 1: Extend `AdminPlaylistItemRow` in `src/routes/admin/mod.rs`**

Replace the `AdminPlaylistItemRow` struct (lines 54-60):

```rust
pub struct AdminPlaylistItemRow {
    pub id: i64,
    pub title: String,
    pub url: String,
    pub duration_secs: i64,
    pub sort_order: i64,
}
```

with:

```rust
pub struct AdminPlaylistItemRow {
    pub id: i64,
    pub title: String,
    pub url: String,
    pub duration_secs: i64,
    pub sort_order: i64,
    pub budget_badge_class: &'static str,
    pub budget_badge_char: &'static str,
}

impl AdminPlaylistItemRow {
    /// Fills the budget badge fields from a CORS-cache snapshot, keyed by this item's URL host.
    pub fn apply_budget(&mut self, cors_cache: &std::collections::HashMap<String, bool>) {
        let (class, glyph) =
            crate::budget::budget_badge(crate::budget::status_for_url(&self.url, cors_cache));
        self.budget_badge_class = class;
        self.budget_badge_char = glyph;
    }
}
```

- [ ] **Step 2: Update the `From<PlaylistItem>` impl in `src/routes/admin/mod.rs`**

Replace the existing impl (lines 105-115):

```rust
impl From<playlist_item::PlaylistItem> for AdminPlaylistItemRow {
    fn from(i: playlist_item::PlaylistItem) -> Self {
        Self {
            id: i.id,
            title: i.title,
            url: i.url,
            duration_secs: i.duration_secs,
            sort_order: i.sort_order,
        }
    }
}
```

with:

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
        }
    }
}
```

- [ ] **Step 3: Export the new handler in `src/routes/admin/mod.rs`**

Replace:

```rust
pub use playlist::{playlist_item_create, playlist_item_delete};
```

with:

```rust
pub use playlist::{playlist_item_create, playlist_item_delete, playlist_item_test};
```

- [ ] **Step 4: Apply budget to playlist items in `channel_detail`**

In `src/routes/admin/channels.rs`, the `cors` snapshot is already taken (line ~274) and used for `sources`. After the `let sources: Vec<AdminSourceRow> = ...` block, add a mapped `playlist_items` binding:

```rust
    let playlist_items: Vec<AdminPlaylistItemRow> = items
        .into_iter()
        .map(|i| {
            let mut row: AdminPlaylistItemRow = i.into();
            row.apply_budget(&cors);
            row
        })
        .collect();
```

Then in the `render(ChannelDetailTemplate { ... })` call, replace:

```rust
        playlist_items: items.into_iter().map(Into::into).collect(),
```

with:

```rust
        playlist_items,
```

(`items` is consumed by the new binding; it was only borrowed earlier for `vod_schedule`, so this compiles.)

- [ ] **Step 5: Add the `playlist_item_test` handler in `src/routes/admin/playlist.rs`**

At the top of `src/routes/admin/playlist.rs`, update imports. Replace:

```rust
use axum::{
    extract::{Path, State},
    http::StatusCode,
    response::{IntoResponse, Redirect},
};
use serde::Deserialize;

use crate::routes::internal_error;
use crate::{
    media::{hls, resolver},
    model::{playlist_item, playlist_item::NewPlaylistItem},
    AppState,
};
```

with:

```rust
use askama::Template;
use axum::{
    extract::{Path, State},
    http::StatusCode,
    response::{Html, IntoResponse, Redirect},
};
use serde::Deserialize;

use crate::routes::admin::AdminPlaylistItemRow;
use crate::routes::{internal_error, render};
use crate::{
    media::{hls, resolver},
    model::{playlist_item, playlist_item::NewPlaylistItem},
    AppState,
};

#[derive(Template)]
#[template(path = "admin/partials/playlist_item_row.html")]
struct PlaylistItemRowTemplate {
    item: AdminPlaylistItemRow,
}
```

Then add the handler at the end of the file:

```rust
pub async fn playlist_item_test(
    State(state): State<AppState>,
    Path(item_id): Path<i64>,
) -> Result<Html<String>, StatusCode> {
    let item = playlist_item::get(&state.pool, item_id)
        .await
        .map_err(internal_error)?
        .ok_or(StatusCode::NOT_FOUND)?;

    crate::health::probe_and_cache_cors(&state.http_client, &state.cors_cache, &item.url).await;

    let cors = state.cors_cache.read().await.clone();
    let mut row: AdminPlaylistItemRow = item.into();
    row.apply_budget(&cors);

    render(PlaylistItemRowTemplate { item: row })
}
```

- [ ] **Step 6: Create the row partial `templates/admin/partials/playlist_item_row.html`**

```html
<tr id="pl-row-{{ item.id }}">
  <td style="color:#555">{{ item.sort_order }}</td>
  <td>{{ item.title }}</td>
  <td style="word-break:break-all;max-width:360px;font-size:0.78rem">{{ item.url }}</td>
  <td style="white-space:nowrap">{{ item.duration_secs }}s</td>
  <td>
    {% if item.budget_badge_char.is_empty() %}
    <span style="color:#888" title="Network budget not yet probed">·</span>
    {% else %}
    <span class="{{ item.budget_badge_class }}" title="Network budget">{{ item.budget_badge_char }}</span>
    {% endif %}
  </td>
  <td style="white-space:nowrap">
    <button class="btn btn-sm" type="button"
            hx-post="/admin/playlist/{{ item.id }}/test"
            hx-target="#pl-row-{{ item.id }}"
            hx-swap="outerHTML"
            hx-disabled-elt="this">Test</button>
    <form action="/admin/playlist/{{ item.id }}/delete" method="post"
          style="display:inline-block;margin-left:4px">
      <button class="btn btn-sm btn-danger" type="submit"
              onclick="return confirm('Remove this item?')">Delete</button>
    </form>
  </td>
</tr>
```

- [ ] **Step 7: Update `templates/admin/channel_detail.html`**

Replace the playlist table header row:

```html
      <tr><th>#</th><th>Title</th><th>URL</th><th>Duration</th><th></th></tr>
```

with:

```html
      <tr><th>#</th><th>Title</th><th>URL</th><th>Duration</th><th>Budget</th><th></th></tr>
```

Then replace the inline playlist `<tr> ... </tr>` block inside `{% for item in playlist_items %}`:

```html
      {% for item in playlist_items %}
      <tr>
        <td style="color:#555">{{ item.sort_order }}</td>
        <td>{{ item.title }}</td>
        <td style="word-break:break-all;max-width:360px;font-size:0.78rem">{{ item.url }}</td>
        <td style="white-space:nowrap">{{ item.duration_secs }}s</td>
        <td>
          <form action="/admin/playlist/{{ item.id }}/delete" method="post">
            <button class="btn btn-sm btn-danger" type="submit"
                    onclick="return confirm('Remove this item?')">Delete</button>
          </form>
        </td>
      </tr>
      {% endfor %}
```

with:

```html
      {% for item in playlist_items %}
      {% include "admin/partials/playlist_item_row.html" %}
      {% endfor %}
```

- [ ] **Step 8: Register the route in `src/lib.rs`**

After the `/playlist/:id/delete` route (lines 67-70), add:

```rust
        .route(
            "/playlist/:id/test",
            post(routes::admin::playlist_item_test),
        )
```

- [ ] **Step 9: Build to verify it compiles**

Run: `cargo build`
Expected: compiles cleanly (Askama validates the templates at compile time).

- [ ] **Step 10: Write the failing integration test**

Add to `tests/http.rs`, after the VOD guide test:

```rust
#[tokio::test]
async fn test_playlist_item_test_returns_row_partial() {
    // Playlist item 1 belongs to VOD channel 4 (https, unreachable in tests).
    let response = app()
        .await
        .oneshot(authed_post("/admin/playlist/1/test"))
        .await
        .unwrap();
    assert_eq!(response.status(), StatusCode::OK);
    let body = body_text(response).await;
    assert!(
        body.contains("pl-row-1"),
        "response should be the playlist row partial"
    );
}
```

- [ ] **Step 11: Run the integration test**

Run: `cargo test --test http test_playlist_item_test_returns_row_partial`
Expected: PASS.

- [ ] **Step 12: Commit**

```bash
cargo fmt
git add src/routes/admin/mod.rs src/routes/admin/channels.rs src/routes/admin/playlist.rs \
        templates/admin/partials/playlist_item_row.html templates/admin/channel_detail.html \
        src/lib.rs tests/http.rs
git commit -m "feat: per-item budget badge + Test button on VOD playlist

Co-Authored-By: Claude Opus 4.8 (1M context) <noreply@anthropic.com>"
```

---

## Task 5: Background checker sweeps VOD item hosts

**Files:**
- Modify: `src/health.rs` (add `probe_all_playlist_cors`; call it from `check_all`)
- Modify: `docs/IDEAS.md` (mark idea 12 done)

- [ ] **Step 1: Add the sweep function in `src/health.rs`**

After `check_all`, add:

```rust
async fn probe_all_playlist_cors(
    pool: &SqlitePool,
    client: &reqwest::Client,
    cors_cache: &CorsCache,
) {
    let items = match crate::model::playlist_item::list_all(pool).await {
        Ok(i) => i,
        Err(e) => {
            tracing::error!("health: failed to fetch playlist items: {e}");
            return;
        }
    };
    // Dedupe by host so each CDN is probed at most once per cycle.
    let mut probed_hosts = std::collections::HashSet::new();
    for item in items {
        if !item.url.starts_with("https://") || crate::media::resolver::needs_resolution(&item.url) {
            continue;
        }
        let host = crate::media::hls::extract_manifest_host(&item.url);
        if !probed_hosts.insert(host) {
            continue;
        }
        probe_and_cache_cors(client, cors_cache, &item.url).await;
    }
}
```

- [ ] **Step 2: Call it from `check_all`**

In `src/health.rs`, at the end of `check_all`, after the `for src in sources { ... }` loop, add:

```rust
    probe_all_playlist_cors(pool, client, cors_cache).await;
```

- [ ] **Step 3: Verify the full suite passes**

Run: `cargo test`
Expected: PASS — all unit + integration tests (previously 117 + the new ones).

- [ ] **Step 4: Verify clippy and fmt are clean**

Run: `cargo clippy -- -D warnings && cargo fmt --check`
Expected: no warnings, no diff.

- [ ] **Step 5: Mark idea 12 done in `docs/IDEAS.md`**

Replace line 23 (the idea 12 bullet) so its title is struck through and a `done:` note is appended, matching the style of ideas 10/11. Replace:

```markdown
12. **CORS budget badge for VOD channels** — VOD channels store URLs in `playlist_items`, not `sources`, so `build_guide_data` (which derives budget only from the `sources` table) always yields `Unknown` → no badge for VOD. Extend budget derivation to cover VOD playlist-item URLs, and add a probe trigger for playlist items (VOD items have no per-source Test button today). Depends on idea 11A's descend-into-master probe.
```

with:

```markdown
12. ~~**CORS budget badge for VOD channels**~~ — done: `build_guide_data` derives a VOD channel's budget badge from its currently-playing playlist item (`vod_budget_url` + `playlist_item::current_position`); the CORS cache for item hosts is warmed by a per-item admin **Test** button (`POST /admin/playlist/:id/test`) and a new background-checker sweep (`probe_all_playlist_cors`, deduped by host). The shared `health::probe_and_cache_cors` helper (skips non-HTTPS and youtube/twitch URLs) backs sources, the Test button, and the sweep. Spec: `docs/superpowers/specs/2026-06-02-vod-budget-badge-design.md`.
```

- [ ] **Step 6: Commit**

```bash
cargo fmt
git add src/health.rs docs/IDEAS.md
git commit -m "feat: background CORS sweep for VOD playlist hosts; mark idea 12 done

Co-Authored-By: Claude Opus 4.8 (1M context) <noreply@anthropic.com>"
```

---

## Self-Review Notes

- **Spec coverage:** Component 1 → Task 1; Component 2 → Task 3; Component 3 → Task 4; Component 4 → Tasks 2 (model) + 5 (sweep). Testing section → unit tests in Tasks 1/2/3, integration tests in Tasks 3/4.
- **Type consistency:** `probe_and_cache_cors(client, cors_cache: &CorsCache, url) -> Option<bool>` is defined in Task 1 and called identically in Tasks 4 and 5. `budget_for_url(Option<&str>, &HashMap)` and `vod_budget_url(&[PlaylistItem], Option<DateTime<Utc>>, DateTime<Utc>) -> Option<String>` defined and used in Task 3. `AdminPlaylistItemRow` fields/`apply_budget` defined in Task 4 Step 1 and used by the handler (Step 5) and templates (Steps 6-7).
- **No background-sweep integration test:** the 15-min detached task isn't directly exercised in tests; its building blocks (`probe_and_cache_cors`, `list_all`) are unit-tested, and `check_all` wiring is a one-line call. This is an accepted gap, consistent with the existing untested `health::start` spawn.
