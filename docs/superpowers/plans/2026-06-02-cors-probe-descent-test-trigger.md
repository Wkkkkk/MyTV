# CORS Probe Descent + Manual Test-Button Trigger Implementation Plan

> **For agentic workers:** REQUIRED SUB-SKILL: Use superpowers:subagent-driven-development (recommended) or superpowers:executing-plans to implement this plan task-by-task. Steps use checkbox (`- [ ]`) syntax for tracking.

**Goal:** Make the CORS probe descend one level (master → variant → segment) and key the cache by source-URL host everywhere, so HTTPS+CORS channels show the ⚡ badge; turn the admin Test button into a manual per-source health+CORS check that updates the Health dot and a new Budget column.

**Architecture:** A shared `probe_source_cors` in `media/hls.rs` does the 1-level descent and feeds an in-memory `cors_cache` keyed by source-URL host. A unified `health::check_source` runs the HTTP health check + CORS probe and is called by both the 15-min background loop and the Test button. The guide and admin source table share a new `budget` module that renders ⚡ / ☁ / blank from the cache.

**Tech Stack:** Rust, Axum 0.7, SQLx (SQLite), Askama templates, HTMX. Toolchain pinned to 1.96.

**Spec:** `docs/superpowers/specs/2026-06-02-cors-probe-descent-test-trigger-design.md`

---

## File Structure

| File | Responsibility | Change |
|---|---|---|
| `src/media/hls.rs` | HLS parsing + probe primitives | Add `extract_manifest_host`, `find_first_variant_url`, `find_segment_with_descent`, `probe_source_cors`, `fetch_text` |
| `src/budget.rs` (new) | Network-budget status shared by guide + admin | `BudgetStatus`, `status_for_url`, `budget_badge` |
| `src/lib.rs` | Crate root | Declare `mod budget;` |
| `src/routes/guide.rs` | EPG guide | Import budget items from `crate::budget`; remove local copies |
| `src/health.rs` | Background health checker | `check_one` → `pub check_source` (+ cache + probe); drop `probe_cors_for_source` + local host fn |
| `src/routes/player.rs` | Stream proxy | `resolve_direct_segments` uses descent + shared host fn |
| `src/routes/admin/mod.rs` | Admin row types | `AdminSourceRow` gains budget badge fields |
| `src/routes/admin/channels.rs` | Admin channel detail | `channel_detail` derives per-source budget |
| `src/routes/admin/sources.rs` | Source CRUD/test | `source_test` runs `check_source`, renders row partial |
| `templates/admin/partials/source_row.html` (new) | One source `<tr>` | Health + Budget cells, actions |
| `templates/admin/channel_detail.html` | Channel detail page | Budget header, `{% include %}`, Test targets row |
| `tests/http.rs` | Integration tests | Test-button persistence + guide badge |

---

## Task 1: Probe primitives + 1-level descent in `hls.rs`

**Files:**
- Modify: `src/media/hls.rs` (add functions after `find_first_segment_url`, before `has_cors_wildcard`; reuses existing private `resolve_uri`)
- Test: `src/media/hls.rs` (`#[cfg(test)] mod tests`)

- [ ] **Step 1: Write the failing tests**

Add to the `tests` module in `src/media/hls.rs`:

```rust
    #[test]
    fn test_extract_manifest_host_strips_path() {
        assert_eq!(
            extract_manifest_host("https://cdn.example.com/live/index.m3u8"),
            "https://cdn.example.com"
        );
    }

    #[test]
    fn test_extract_manifest_host_no_path() {
        assert_eq!(
            extract_manifest_host("https://cdn.example.com"),
            "https://cdn.example.com"
        );
    }

    #[test]
    fn test_find_first_variant_url_resolves_relative() {
        let master = "#EXTM3U\n#EXT-X-STREAM-INF:BANDWIDTH=1\nvariant/720.m3u8\n";
        assert_eq!(
            find_first_variant_url(master, "https://h.com/live/master.m3u8"),
            Some("https://h.com/live/variant/720.m3u8".to_string())
        );
    }

    #[test]
    fn test_find_first_variant_url_none_for_media_playlist() {
        let media = "#EXTM3U\n#EXTINF:6,\nseg1.ts\n";
        assert_eq!(find_first_variant_url(media, "https://h.com/v.m3u8"), None);
    }

    #[tokio::test]
    async fn test_find_segment_with_descent_depth_zero() {
        // base already a variant: segment found without any network call
        let client = reqwest::Client::new();
        let media = "#EXTM3U\n#EXTINF:6,\nhttps://cdn.com/seg1.ts\n";
        let seg = find_segment_with_descent(&client, media, "https://h.com/v.m3u8").await;
        assert_eq!(seg.as_deref(), Some("https://cdn.com/seg1.ts"));
    }
```

- [ ] **Step 2: Run tests to verify they fail**

Run: `cargo test --lib media::hls`
Expected: FAIL — `extract_manifest_host`, `find_first_variant_url`, `find_segment_with_descent` not found.

- [ ] **Step 3: Implement the functions**

Insert into `src/media/hls.rs` immediately after `find_first_segment_url` (ends at the line with `None\n}` around line 151) and before `has_cors_wildcard`:

```rust
/// Extracts `scheme://host` from a URL, stripping any path/query.
/// This is the canonical CORS-cache key (the source-URL host).
pub fn extract_manifest_host(url: &str) -> String {
    let after = url.find("://").map(|i| i + 3).unwrap_or(0);
    let host_end = url[after..].find('/').unwrap_or(url[after..].len());
    url[..after + host_end].to_string()
}

/// Returns the first sub-playlist (`.m3u8`/`.m3u`) line in a master playlist,
/// resolved to an absolute URL. `None` if there is no sub-playlist line.
pub fn find_first_variant_url(content: &str, base_url: &str) -> Option<String> {
    for line in content.lines() {
        if line.starts_with('#') || line.is_empty() {
            continue;
        }
        let lower = line.to_lowercase();
        let path = lower.split('?').next().unwrap_or(&lower);
        if path.ends_with(".m3u8") || path.ends_with(".m3u") {
            return Some(resolve_uri(line, base_url));
        }
    }
    None
}

/// Finds a segment URL to CORS-probe, descending one level if `content` is a master playlist.
/// Returns `None` if no segment can be found within one descent.
pub async fn find_segment_with_descent(
    client: &reqwest::Client,
    content: &str,
    base_url: &str,
) -> Option<String> {
    if let Some(seg) = find_first_segment_url(content, base_url) {
        return Some(seg);
    }
    let variant = find_first_variant_url(content, base_url)?;
    let body = fetch_text(client, &variant).await?;
    find_first_segment_url(&body, &variant)
}

/// Determines whether segments for `source_url` can be fetched directly by the browser.
/// `Some(true)` = direct (HTTPS segment with `Access-Control-Allow-Origin: *`),
/// `Some(false)` = must proxy (HTTP segment, or HTTPS without CORS),
/// `None` = could not determine (network error, or no segment after one descent).
pub async fn probe_source_cors(client: &reqwest::Client, source_url: &str) -> Option<bool> {
    let body = fetch_text(client, source_url).await?;
    let segment = find_segment_with_descent(client, &body, source_url).await?;
    if segment.starts_with("http://") {
        return Some(false);
    }
    Some(probe_cors(client, &segment).await)
}

async fn fetch_text(client: &reqwest::Client, url: &str) -> Option<String> {
    client
        .get(url)
        .timeout(Duration::from_secs(10))
        .send()
        .await
        .ok()?
        .text()
        .await
        .ok()
}
```

`Duration` is already imported at the top of the file (`use std::time::Duration;`). `resolve_uri` and `find_first_segment_url` already exist in this module.

- [ ] **Step 4: Run tests to verify they pass**

Run: `cargo test --lib media::hls`
Expected: PASS (all hls tests, including the 5 new ones).

- [ ] **Step 5: Format and commit**

```bash
cargo fmt
git add src/media/hls.rs
git commit -m "feat: add CORS probe with one-level master-playlist descent"
```

---

## Task 2: Shared `budget` module

**Files:**
- Create: `src/budget.rs`
- Modify: `src/lib.rs` (declare module)
- Test: `src/budget.rs` (`#[cfg(test)] mod tests`)

- [ ] **Step 1: Create `src/budget.rs` with implementation + failing tests**

```rust
use std::collections::HashMap;

use crate::media::hls::extract_manifest_host;

#[derive(Debug, Clone, Copy, PartialEq)]
pub enum BudgetStatus {
    Direct,
    Proxied,
    Unknown,
}

/// Derives the network-budget status for a single source URL from the CORS cache.
/// HTTP URLs are always `Proxied` (mixed content) without a cache lookup.
pub fn status_for_url(url: &str, cors_cache: &HashMap<String, bool>) -> BudgetStatus {
    if url.starts_with("http://") {
        return BudgetStatus::Proxied;
    }
    match cors_cache.get(&extract_manifest_host(url)) {
        Some(&true) => BudgetStatus::Direct,
        Some(&false) => BudgetStatus::Proxied,
        None => BudgetStatus::Unknown,
    }
}

/// Maps a budget status to a (CSS class, glyph) pair. Unknown renders an empty glyph.
pub fn budget_badge(status: BudgetStatus) -> (&'static str, &'static str) {
    match status {
        BudgetStatus::Direct => ("budget-direct", "⚡"),
        BudgetStatus::Proxied => ("budget-proxied", "☁"),
        BudgetStatus::Unknown => ("budget-unknown", ""),
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn test_status_for_url_http_always_proxied() {
        assert_eq!(
            status_for_url("http://example.com/stream.m3u8", &HashMap::new()),
            BudgetStatus::Proxied
        );
    }

    #[test]
    fn test_status_for_url_https_cache_hit_direct() {
        let mut cache = HashMap::new();
        cache.insert("https://example.com".to_string(), true);
        assert_eq!(
            status_for_url("https://example.com/stream.m3u8", &cache),
            BudgetStatus::Direct
        );
    }

    #[test]
    fn test_status_for_url_https_cache_hit_proxied() {
        let mut cache = HashMap::new();
        cache.insert("https://example.com".to_string(), false);
        assert_eq!(
            status_for_url("https://example.com/stream.m3u8", &cache),
            BudgetStatus::Proxied
        );
    }

    #[test]
    fn test_status_for_url_https_cache_miss_unknown() {
        assert_eq!(
            status_for_url("https://example.com/stream.m3u8", &HashMap::new()),
            BudgetStatus::Unknown
        );
    }
}
```

- [ ] **Step 2: Declare the module in `src/lib.rs`**

In `src/lib.rs`, add `mod budget;` to the module list at the top (after line 3 `mod epg;`). Result:

```rust
pub mod config;
pub mod db;
mod budget;
mod epg;
pub mod health;
mod media;
mod model;
mod routes;
```

- [ ] **Step 3: Run tests to verify they pass**

Run: `cargo test --lib budget`
Expected: PASS (4 tests).

- [ ] **Step 4: Format and commit**

```bash
cargo fmt
git add src/budget.rs src/lib.rs
git commit -m "feat: add shared budget module for network-budget status"
```

---

## Task 3: Point `guide.rs` at the shared budget module

**Files:**
- Modify: `src/routes/guide.rs` (lines 43-48 enum, 116-136 derive, 146-152 badge, tests 712-744)

- [ ] **Step 1: Remove the local `BudgetStatus` enum**

Delete lines 43-48 in `src/routes/guide.rs`:

```rust
#[derive(Debug, Clone, Copy, PartialEq)]
pub enum BudgetStatus {
    Direct,
    Proxied,
    Unknown,
}
```

- [ ] **Step 2: Add the import**

At the top of `src/routes/guide.rs`, add (next to the other `use` statements):

```rust
use crate::budget::{budget_badge, status_for_url, BudgetStatus};
```

- [ ] **Step 3: Delete the local `budget_badge` fn**

Delete lines 146-152 (the `fn budget_badge(...)` block) in `src/routes/guide.rs`. The `health_badge` fn stays.

- [ ] **Step 4: Simplify `derive_budget_status` to delegate**

Replace the body of `derive_budget_status` (lines 116-136) with:

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

- [ ] **Step 5: Remove the migrated budget tests**

In the `tests` module of `src/routes/guide.rs`, delete these four tests (now covered by `budget.rs`): `test_derive_budget_status_http_always_proxied`, `test_derive_budget_status_https_cache_hit_direct`, `test_derive_budget_status_https_cache_hit_proxied`, `test_derive_budget_status_https_cache_miss_unknown`. **Keep** `test_derive_budget_status_no_source_unknown` (it tests the no-URL wrapper path).

- [ ] **Step 6: Build and test**

Run: `cargo test --lib routes::guide`
Expected: PASS. No unused-import or dead-code warnings.

Run: `cargo clippy --all-targets -- -D warnings`
Expected: clean.

- [ ] **Step 7: Format and commit**

```bash
cargo fmt
git add src/routes/guide.rs
git commit -m "refactor: guide uses shared budget module"
```

---

## Task 4: Unify per-source check in `health.rs`

**Files:**
- Modify: `src/health.rs` (lines 30-70: `check_all`, `extract_manifest_host`, `probe_cors_for_source`, `check_one`)

- [ ] **Step 1: Rewrite `check_all` to call `check_source`**

Replace `check_all` (lines 30-44) in `src/health.rs` with:

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
}
```

- [ ] **Step 2: Delete `extract_manifest_host` and `probe_cors_for_source`**

Delete lines 46-70 in `src/health.rs` (the local `fn extract_manifest_host` and `async fn probe_cors_for_source`). The host fn now lives in `hls.rs`; the probe is folded into `check_source`.

- [ ] **Step 3: Rename `check_one` → `check_source` and fold in the probe**

Replace the `async fn check_one(...)` signature and add the CORS probe at the end. The function becomes (replacing lines 72-108):

```rust
pub async fn check_source(
    pool: &SqlitePool,
    client: &reqwest::Client,
    cors_cache: &CorsCache,
    src: &Source,
) {
    let (ok, reason) = do_http_check(client, src).await;
    let (new_failures, action) = process_result(src, ok);

    let is_active = match action {
        HealthAction::Disable => Some(false),
        HealthAction::Reenable => Some(true),
        HealthAction::None => None,
    };

    if let Err(e) = source::update_health(
        pool,
        src.id,
        if ok { "ok" } else { "error" },
        reason.as_deref(),
        new_failures,
        is_active,
    )
    .await
    {
        tracing::error!("health: failed to update source {}: {e}", src.id);
        return;
    }

    match action {
        HealthAction::Disable => tracing::warn!(
            "health: source {} auto-disabled after {} consecutive failures",
            src.id,
            new_failures
        ),
        HealthAction::Reenable => tracing::info!(
            "health: source {} auto-re-enabled after passing health check",
            src.id
        ),
        HealthAction::None => {}
    }

    if src.url.starts_with("https://") {
        if let Some(result) = crate::media::hls::probe_source_cors(client, &src.url).await {
            let host_key = crate::media::hls::extract_manifest_host(&src.url);
            cors_cache.write().await.insert(host_key.clone(), result);
            tracing::debug!(source_id = src.id, host = %host_key, cors = result, "CORS probe cached");
        }
    }
}
```

- [ ] **Step 4: Build and run existing health tests**

Run: `cargo test --lib health`
Expected: PASS — `process_result` unit tests are unchanged and still compile.

Run: `cargo clippy --all-targets -- -D warnings`
Expected: clean (no dead `extract_manifest_host`).

- [ ] **Step 5: Format and commit**

```bash
cargo fmt
git add src/health.rs
git commit -m "refactor: unify per-source health + CORS probe into check_source"
```

---

## Task 5: Player cache-key fix via descent

**Files:**
- Modify: `src/routes/player.rs` (lines 214-239: `extract_manifest_host`, `resolve_direct_segments`)

- [ ] **Step 1: Delete the local `extract_manifest_host`**

Delete lines 214-218 in `src/routes/player.rs` (the `fn extract_manifest_host`). It now comes from `hls`.

- [ ] **Step 2: Rewrite `resolve_direct_segments` to descend and use the shared host fn**

Replace `resolve_direct_segments` (lines 220-239) with:

```rust
async fn resolve_direct_segments(state: &AppState, content: &str, base_url: &str) -> bool {
    let host_key = hls::extract_manifest_host(base_url);
    {
        let cache = state.cors_cache.read().await;
        if let Some(&cached) = cache.get(&host_key) {
            return cached;
        }
    }
    let segment_url = match hls::find_segment_with_descent(&state.http_client, content, base_url).await
    {
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

This checks the cache first (no extra fetch on a hit), descends one level on a master, and always keys by the source/master host (`base_url` on the first tune). `hls` is already imported in `player.rs`.

- [ ] **Step 3: Build and test**

Run: `cargo test --lib routes::player`
Expected: PASS (existing player tests unaffected).

Run: `cargo clippy --all-targets -- -D warnings`
Expected: clean.

- [ ] **Step 4: Format and commit**

```bash
cargo fmt
git add src/routes/player.rs
git commit -m "fix: key CORS cache by source host and descend on master playlists"
```

---

## Task 6: `AdminSourceRow` gains budget badge fields

**Files:**
- Modify: `src/routes/admin/mod.rs` (struct at lines 41-50, `From` impl at lines 74-87)

- [ ] **Step 1: Add the fields to the struct**

In `src/routes/admin/mod.rs`, change `AdminSourceRow` (lines 41-50) to add two fields:

```rust
pub struct AdminSourceRow {
    pub id: i64,
    pub kind: String,
    pub url: String,
    pub priority: i64,
    pub is_active: bool,
    pub last_status: Option<String>,
    pub consecutive_failures: i64,
    pub failure_reason: Option<String>,
    pub budget_badge_class: &'static str,
    pub budget_badge_char: &'static str,
}
```

- [ ] **Step 2: Default the fields in `From<Source>`**

In the `From<source::Source> for AdminSourceRow` impl (lines 74-87), add the two fields with the Unknown badge default:

```rust
impl From<source::Source> for AdminSourceRow {
    fn from(s: source::Source) -> Self {
        Self {
            id: s.id,
            kind: s.kind,
            url: s.url,
            priority: s.priority,
            is_active: s.is_active,
            last_status: s.last_status,
            consecutive_failures: s.consecutive_failures,
            failure_reason: s.failure_reason,
            budget_badge_class: "budget-unknown",
            budget_badge_char: "",
        }
    }
}
```

- [ ] **Step 3: Build**

Run: `cargo build`
Expected: compiles (the template referencing the new fields comes in Task 7, so a build here is fine).

- [ ] **Step 4: Format and commit**

```bash
cargo fmt
git add src/routes/admin/mod.rs
git commit -m "feat: add budget badge fields to AdminSourceRow"
```

---

## Task 7: Source row partial + Budget column in the template

**Files:**
- Create: `templates/admin/partials/source_row.html`
- Modify: `templates/admin/channel_detail.html` (header line 29; loop body lines 32-80)

- [ ] **Step 1: Create the row partial**

Create `templates/admin/partials/source_row.html` with the full `<tr>` (ported from `channel_detail.html` lines 33-79, plus a Budget cell, with the Test form retargeted and the OK/Failed span removed):

```html
<tr id="src-row-{{ src.id }}">
  <td>{{ src.kind }}</td>
  <td style="word-break:break-all;max-width:400px;font-size:0.78rem">{{ src.url }}</td>
  <td>{{ src.priority }}</td>
  <td>
    {% if src.is_active %}
    <span class="badge badge-on">on</span>
    {% else %}
    <span class="badge badge-off">off</span>
    {% endif %}
  </td>
  <td>
    {% match src.last_status %}
    {% when None %}
    <span style="color:#888" title="Never checked">○</span>
    {% when Some(status) %}
    {% if status == "ok" %}
    <span style="color:#4caf50" title="Healthy">●</span>
    {% else %}
    <span style="color:#e94560" title="Last check failed">●</span>
    {% if let Some(reason) = src.failure_reason.as_ref() %}
    <div style="font-size:0.7rem;color:#e94560;margin-top:2px">{{ reason }}</div>
    {% endif %}
    {% if !src.is_active && src.consecutive_failures >= 3 %}
    <div style="font-size:0.7rem;color:#888">[auto-disabled]</div>
    {% endif %}
    {% endif %}
    {% endmatch %}
  </td>
  <td>
    {% if src.budget_badge_char.is_empty() %}
    <span style="color:#888" title="Network budget not yet probed">·</span>
    {% else %}
    <span class="{{ src.budget_badge_class }}" title="Network budget">{{ src.budget_badge_char }}</span>
    {% endif %}
  </td>
  <td style="white-space:nowrap">
    <form action="/admin/sources/{{ src.id }}/toggle" method="post" style="display:inline-block">
      <button class="btn btn-sm" type="submit">
        {% if src.is_active %}Disable{% else %}Enable{% endif %}
      </button>
    </form>
    <form action="/admin/sources/{{ src.id }}/delete" method="post" style="display:inline-block;margin-left:4px">
      <button class="btn btn-sm btn-danger" type="submit"
              onclick="return confirm('Remove this source?')">Delete</button>
    </form>
    <form hx-post="/admin/sources/{{ src.id }}/test"
          hx-target="#src-row-{{ src.id }}"
          hx-swap="outerHTML"
          style="display:inline-block;margin-left:4px">
      <button class="btn btn-sm" type="submit">Test</button>
    </form>
  </td>
</tr>
```

- [ ] **Step 2: Add the Budget header column**

In `templates/admin/channel_detail.html`, change the source table header (line 29) from:

```html
<tr><th>Kind</th><th>URL</th><th>Priority</th><th>Active</th><th>Health</th><th></th></tr>
```

to:

```html
<tr><th>Kind</th><th>URL</th><th>Priority</th><th>Active</th><th>Health</th><th>Budget</th><th></th></tr>
```

- [ ] **Step 3: Replace the loop body with the include**

In `templates/admin/channel_detail.html`, replace the entire `{% for src in sources %}` … `{% endfor %}` block (lines 32-81 — the `<tr>` … `</tr>` and the surrounding loop) with:

```html
      {% for src in sources %}
      {% include "admin/partials/source_row.html" %}
      {% endfor %}
```

The loop variable `src` is in scope for the included partial.

- [ ] **Step 4: Build (Askama validates templates at compile time)**

Run: `cargo build`
Expected: compiles. If Askama complains the partial template is not found, confirm the path is `templates/admin/partials/source_row.html` (Askama resolves include paths relative to the `templates/` root).

- [ ] **Step 5: Format and commit**

```bash
cargo fmt
git add templates/admin/partials/source_row.html templates/admin/channel_detail.html
git commit -m "feat: source row partial with Budget column; Test targets the row"
```

---

## Task 8: Derive per-source budget in `channel_detail`

**Files:**
- Modify: `src/routes/admin/channels.rs` (`channel_detail`, lines 230-282)

- [ ] **Step 1: Compute budget per source before rendering**

In `src/routes/admin/channels.rs`, replace the `sources:` line in the final `render(ChannelDetailTemplate { ... })` (line 278) so the rows get their budget filled. Replace:

```rust
    render(ChannelDetailTemplate {
        channel_id: ch.id,
        channel_name: ch.name,
        channel_type: ch.r#type,
        sources: srcs.into_iter().map(Into::into).collect(),
        playlist_items: items.into_iter().map(Into::into).collect(),
        vod_schedule,
    })
```

with:

```rust
    let cors = state.cors_cache.read().await.clone();
    let sources: Vec<AdminSourceRow> = srcs
        .into_iter()
        .map(|s| {
            let (cls, ch_glyph) =
                crate::budget::budget_badge(crate::budget::status_for_url(&s.url, &cors));
            let mut row: AdminSourceRow = s.into();
            row.budget_badge_class = cls;
            row.budget_badge_char = ch_glyph;
            row
        })
        .collect();

    render(ChannelDetailTemplate {
        channel_id: ch.id,
        channel_name: ch.name,
        channel_type: ch.r#type,
        sources,
        playlist_items: items.into_iter().map(Into::into).collect(),
        vod_schedule,
    })
```

`AdminSourceRow` is already imported in `channels.rs` (used by `ChannelDetailTemplate`). `status_for_url` borrows `s.url` before `s.into()` moves `s`.

- [ ] **Step 2: Build and test**

Run: `cargo test --lib routes::admin`
Expected: PASS.

Run: `cargo clippy --all-targets -- -D warnings`
Expected: clean.

- [ ] **Step 3: Format and commit**

```bash
cargo fmt
git add src/routes/admin/channels.rs
git commit -m "feat: derive per-source budget badge in channel detail"
```

---

## Task 9: Rewrite `source_test` to run the unified check + render the row

**Files:**
- Modify: `src/routes/admin/sources.rs` (imports lines 1-10; `source_test` lines 79-116)

- [ ] **Step 1: Update imports and add the row template struct**

In `src/routes/admin/sources.rs`, replace the import block (lines 1-10) with:

```rust
use askama::Template;
use axum::{
    extract::{Path, State},
    http::StatusCode,
    response::{Html, IntoResponse, Redirect},
};
use serde::Deserialize;

use crate::routes::admin::AdminSourceRow;
use crate::routes::{internal_error, render};
use crate::{model::source, AppState};
```

`crate::media::resolver` is removed — it was used **only** in the old `source_test` (verified: lines 91-92 are its sole uses in this file), and the rewritten handler no longer resolves yt-dlp URLs (`check_source`'s HTTP check already handles `youtube_live`). `render` is the existing `crate::routes::render<T: askama::Template>(t) -> Result<Html<String>, StatusCode>` helper. `use askama::Template;` is needed for the `#[derive(Template)]` macro.

Add the template struct near the top (after the `SourceForm` struct, around line 19):

```rust
#[derive(Template)]
#[template(path = "admin/partials/source_row.html")]
struct SourceRowTemplate {
    src: AdminSourceRow,
}
```

- [ ] **Step 2: Replace the `source_test` body**

Replace `source_test` (lines 79-116) with:

```rust
pub async fn source_test(
    State(state): State<AppState>,
    Path(source_id): Path<i64>,
) -> Result<Html<String>, StatusCode> {
    let src = source::get(&state.pool, source_id)
        .await
        .map_err(internal_error)?
        .ok_or(StatusCode::NOT_FOUND)?;

    crate::health::check_source(&state.pool, &state.http_client, &state.cors_cache, &src).await;

    let updated = source::get(&state.pool, source_id)
        .await
        .map_err(internal_error)?
        .ok_or(StatusCode::NOT_FOUND)?;

    let cors = state.cors_cache.read().await.clone();
    let (cls, glyph) = crate::budget::budget_badge(crate::budget::status_for_url(&updated.url, &cors));
    let mut row: AdminSourceRow = updated.into();
    row.budget_badge_class = cls;
    row.budget_badge_char = glyph;

    render(SourceRowTemplate { src: row })
}
```

`render` returns `Result<Html<String>, StatusCode>`, matching `source_test`'s return type. `internal_error` is still used for the two `source::get` calls; `Html` is still used in the signature.

- [ ] **Step 3: Build and check for unused imports**

Run: `cargo build`
Expected: compiles with no unused-import warnings (the `resolver` import is gone, `render` is now used).

Run: `cargo clippy --all-targets -- -D warnings`
Expected: clean.

- [ ] **Step 4: Format and commit**

```bash
cargo fmt
git add src/routes/admin/sources.rs
git commit -m "feat: Test button runs unified health+CORS check and returns the row"
```

---

## Task 10: Integration tests

**Files:**
- Modify: `tests/http.rs` (add a POST helper + two tests; add `app_with_cors` helper)

- [ ] **Step 1: Write the failing tests**

Add to `tests/http.rs` (after the existing helpers, e.g. after `body_json`):

```rust
fn authed_post(uri: &str) -> Request<Body> {
    Request::builder()
        .method("POST")
        .uri(uri)
        .header("Authorization", "Basic dXNlcjp0ZXN0")
        .body(Body::empty())
        .unwrap()
}

async fn body_text(response: axum::response::Response) -> String {
    use http_body_util::BodyExt;
    let bytes = response.into_body().collect().await.unwrap().to_bytes();
    String::from_utf8_lossy(&bytes).to_string()
}

async fn app_with_cors(host: &str, direct: bool) -> axum::Router {
    let pool = db::connect("sqlite::memory:").await.unwrap();
    sqlx::query(include_str!("fixtures/seed.sql"))
        .execute(&pool)
        .await
        .unwrap();
    let cors_cache = Arc::new(tokio::sync::RwLock::new(std::collections::HashMap::new()));
    cors_cache.write().await.insert(host.to_string(), direct);
    let state = AppState {
        pool,
        config: Arc::new(Config {
            database_url: "sqlite::memory:".to_string(),
            admin_password: "test".to_string(),
            youtube_api_key: None,
            port: 0,
        }),
        http_client: reqwest::Client::new(),
        cors_cache,
    };
    build_router(state)
}

#[tokio::test]
async fn test_source_test_returns_row_partial_not_ok_badge() {
    // Source 1 (https, unreachable in tests) → check writes an "error" status.
    let response = app()
        .await
        .oneshot(authed_post("/admin/sources/1/test"))
        .await
        .unwrap();
    assert_eq!(response.status(), StatusCode::OK);
    let body = body_text(response).await;
    assert!(body.contains("src-row-1"), "response should be the row partial");
    assert!(body.contains("●"), "row should render a health dot");
    assert!(
        !body.contains(">OK<"),
        "old OK badge text must be gone, got: {body}"
    );
}

#[tokio::test]
async fn test_guide_renders_direct_budget_badge_from_cache() {
    // Channel 1's first active source host is https://stream.example.com.
    let response = app_with_cors("https://stream.example.com", true)
        .await
        .oneshot(req("/guide"))
        .await
        .unwrap();
    assert_eq!(response.status(), StatusCode::OK);
    let body = body_text(response).await;
    assert!(body.contains("⚡"), "guide should show the direct budget badge");
}
```

- [ ] **Step 2: Run tests to verify they fail (before this task's deps) / pass now**

Run: `cargo test --test http test_source_test_returns_row_partial_not_ok_badge test_guide_renders_direct_budget_badge_from_cache`
Expected: PASS (all prior tasks are implemented). If `test_source_test_...` fails because source 1's status stays `None`, confirm `check_source` writes `"error"` on a failed GET — an unreachable host yields `do_http_check → (false, ...)` → `update_health(..., "error", ...)`, so `last_status` becomes `Some("error")` and the red `●` renders.

- [ ] **Step 3: Run the full suite**

Run: `cargo test`
Expected: PASS — the original 117 tests minus the 4 budget tests moved to `budget.rs` (still counted, just relocated) plus the 5 new hls tests, 4 budget tests, and 2 integration tests.

Run: `cargo fmt --check && cargo clippy --all-targets -- -D warnings`
Expected: both clean.

- [ ] **Step 4: Commit**

```bash
cargo fmt
git add tests/http.rs
git commit -m "test: source Test button persistence and guide budget badge"
```

---

## Manual verification (after all tasks)

Run the app and confirm the end-to-end behaviour the spec targets:

```bash
cargo run
```

1. Open `http://localhost:3000/admin/channels/<id>` for a live channel with an HTTPS source. The source table now has a **Budget** column.
2. Click **Test** on a source. The row swaps in place: the **Health** dot updates (green ● / red ●) and persists across a page reload; the **Budget** cell shows ⚡ (direct) for a CORS-enabled HTTPS CDN, ☁ for proxied, or `·` if undetermined.
3. Open `http://localhost:3000/guide`. Channels whose HTTPS source CDN sends `Access-Control-Allow-Origin: *` now show the ⚡ badge (previously blank) once probed — either by clicking Test, tuning, or after a background health cycle.

---

## Self-Review Notes

- **Spec coverage:** §1 probe descent → Task 1; §2 unified check → Task 4; §3 player key fix → Task 5; §4 budget module → Tasks 2–3; §5 admin row/column → Tasks 6–9; testing → Tasks 1, 2, 10. All "Files Changed" rows in the spec map to a task.
- **VOD (idea 12 / C):** intentionally untouched — no VOD task here.
- **Cache key consistency:** `extract_manifest_host` (Task 1) is the single host-derivation used by `health::check_source` (Task 4), `player::resolve_direct_segments` (Task 5), and `budget::status_for_url` (Task 2) — read and write sides agree on the source-URL host.
