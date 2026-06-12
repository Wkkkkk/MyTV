# Player Observability Implementation Plan

> **For agentic workers:** REQUIRED SUB-SKILL: Use superpowers:subagent-driven-development (recommended) or superpowers:executing-plans to implement this plan task-by-task. Steps use checkbox (`- [ ]`) syntax for tracking.

**Goal:** Surface which source/item is playing in `TuneResponse`, and add a shareable per-channel `/watch/:id` deep-link.

**Architecture:** Two independent changes. (D) Add three optional fields to `TuneResponse` and set them at the four response sites — additive, no schema change. (E) A public `/watch/:id` route serves the existing guide page pre-targeted to one channel, passing the id through a JS global emitted inside `guide.html` (read by `base.html`'s init) so the admin templates that also extend `base.html` are unaffected.

**Tech Stack:** Rust, Axum 0.7, Askama 0.12 templates, vanilla JS, `tower::ServiceExt::oneshot` integration tests.

**Spec:** `docs/superpowers/specs/2026-06-12-player-observability-design.md`

---

## File Structure

- `src/routes/player.rs` — Task 1: `TuneResponse` struct + `tune_response` helper + 4 call sites.
- `tests/http.rs` — Tasks 1 & 2: new integration tests (append at end of file).
- `src/routes/guide/mod.rs` — Task 2: hand-define `GuidePageTemplate`, add `watch_page` handler.
- `templates/guide.html` — Task 2: emit the auto-tune JS global.
- `templates/base.html` — Task 2: read the global, auto-tune + `replaceState`.
- `src/lib.rs` — Task 2: register `/watch/:id`.

---

## Task 1: Source/item observability in `TuneResponse` (D)

**Files:**
- Modify: `src/routes/player.rs:20-31` (struct), `:75-92` (`tune_response`), `:176-184` (live Play), `:319-324` (`tune_vod_at`), `:341-346` (`next_vod_at`)
- Test: `tests/http.rs` (append)

- [ ] **Step 1: Write the failing tests**

Append to `tests/http.rs`:

```rust
#[tokio::test]
async fn test_tune_live_includes_source_identity() {
    let response = app().await.oneshot(req("/channel/1/tune")).await.unwrap();
    assert_eq!(response.status(), StatusCode::OK);
    let json = body_json(response).await;
    // Channel 1's only active source is seed source id 1 / live.m3u8.
    assert_eq!(json["source_id"].as_i64().unwrap(), 1);
    assert_eq!(
        json["source_url"].as_str().unwrap(),
        "https://stream.example.com/live.m3u8"
    );
    // A live tune has no playlist item.
    assert!(json["playlist_item_id"].is_null());
}

#[tokio::test]
async fn test_tune_vod_includes_playlist_item_id() {
    let response = app().await.oneshot(req("/channel/4/tune")).await.unwrap();
    assert_eq!(response.status(), StatusCode::OK);
    let json = body_json(response).await;
    // VOD playback comes from a playlist item, not a source.
    assert!(!json["playlist_item_id"].is_null());
    assert!(json["source_id"].is_null());
    assert!(json["source_url"].is_null());
}
```

- [ ] **Step 2: Run tests to verify they fail**

Run: `cargo test --test http test_tune_live_includes_source_identity test_tune_vod_includes_playlist_item_id`
Expected: FAIL — compile error / panic on missing JSON keys (`source_id`, `source_url`, `playlist_item_id` not yet in `TuneResponse`).

- [ ] **Step 3: Add the three fields to `TuneResponse`**

In `src/routes/player.rs`, replace the struct (currently lines 20-31):

```rust
#[derive(Debug, Serialize)]
pub struct TuneResponse {
    pub url: String,
    pub start_offset_secs: i64,
    pub name: String,
    pub logo_url: Option<String>,
    pub category: String,
    pub channel_type: String,
    pub skip_proxy: bool,
    pub ended: bool,
    pub waiting: bool,
    pub source_id: Option<i64>,
    pub source_url: Option<String>,
    pub playlist_item_id: Option<i64>,
}
```

- [ ] **Step 4: Thread the new fields through the helpers and call sites**

In `src/routes/player.rs`, change `tune_response` (lines 75-92) to accept the source and item, and set the new fields. The two `ended`/`waiting` helpers set all three to `None`.

Replace `tune_response`:

```rust
fn tune_response(
    ch: &channel::Channel,
    url: String,
    start_offset_secs: i64,
    skip_proxy: bool,
    source: Option<&source::Source>,
    playlist_item_id: Option<i64>,
) -> Json<TuneResponse> {
    Json(TuneResponse {
        url,
        start_offset_secs,
        name: ch.name.clone(),
        logo_url: ch.logo_url.clone(),
        category: ch.category.clone(),
        channel_type: ch.r#type.clone(),
        skip_proxy,
        ended: false,
        waiting: false,
        source_id: source.map(|s| s.id),
        source_url: source.map(|s| s.url.clone()),
        playlist_item_id,
    })
}
```

In `tune_response_ended` (lines 94-106), add the three fields to the `Json(TuneResponse { ... })` literal:

```rust
        ended: true,
        waiting: false,
        source_id: None,
        source_url: None,
        playlist_item_id: None,
```

In `tune_response_waiting` (lines 108-120), add the three fields to its literal:

```rust
        ended: false,
        waiting: true,
        source_id: None,
        source_url: None,
        playlist_item_id: None,
```

Update the three `tune_response` call sites:

Live Play branch (lines 178-183), pass the resolved source and no item:

```rust
                Some(LiveOutcome::Play) => {
                    crate::health::record_source_liveness(&state.pool, src, true).await;
                    return Ok(tune_response(
                        ch,
                        url,
                        0,
                        resolver::needs_resolution(&src.url),
                        Some(src),
                        None,
                    ));
                }
```

`tune_vod_at` (lines 319-324), pass the item id and no source:

```rust
        Ok(url) => Ok(tune_response(
            ch,
            url,
            offset,
            resolver::needs_resolution(&item.url),
            None,
            Some(item.id),
        )),
```

`next_vod_at` (lines 341-346):

```rust
        Ok(url) => Ok(tune_response(
            ch,
            url,
            0,
            resolver::needs_resolution(&item.url),
            None,
            Some(item.id),
        )),
```

(`src` is in scope in the Play branch; `item` is in scope in both VOD functions — both already accessed via `src.url` / `item.url`. `source::Source` and `playlist_item::PlaylistItem` both expose public `id`/`url` fields.)

- [ ] **Step 5: Run the tests to verify they pass**

Run: `cargo test --test http test_tune_live_includes_source_identity test_tune_vod_includes_playlist_item_id`
Expected: PASS (2 passed).

Then confirm no existing test regressed:

Run: `cargo test --test http`
Expected: PASS (all integration tests, including the existing `test_tune_*`).

- [ ] **Step 6: Format, lint, commit**

```bash
cargo fmt
cargo clippy -- -D warnings
git add src/routes/player.rs tests/http.rs
git commit -m "feat: surface source_id/source_url/playlist_item_id in TuneResponse"
```

Expected: `cargo clippy` exits 0 with no warnings.

---

## Task 2: `GET /watch/:id` deep-link (E)

**Files:**
- Modify: `src/routes/guide/mod.rs` (split `GuidePageTemplate` out of the macro; add `watch_page`)
- Modify: `templates/guide.html:2` (emit JS global)
- Modify: `templates/base.html:462` (read global, auto-tune)
- Modify: `src/lib.rs:122` (register route)
- Test: `tests/http.rs` (append)

- [ ] **Step 1: Write the failing tests**

Append to `tests/http.rs`:

```rust
#[tokio::test]
async fn test_watch_known_channel_injects_auto_tune() {
    let response = app().await.oneshot(req("/watch/1")).await.unwrap();
    assert_eq!(response.status(), StatusCode::OK);
    let body = body_text(response).await;
    assert!(body.contains("window.__autoTuneChannelId = 1;"));
}

#[tokio::test]
async fn test_watch_unknown_channel_falls_back_to_guide() {
    let response = app().await.oneshot(req("/watch/999999")).await.unwrap();
    assert_eq!(response.status(), StatusCode::OK);
    let body = body_text(response).await;
    assert!(!body.contains("__autoTuneChannelId"));
}

#[tokio::test]
async fn test_guide_has_no_auto_tune() {
    let response = app().await.oneshot(req("/guide")).await.unwrap();
    assert_eq!(response.status(), StatusCode::OK);
    let body = body_text(response).await;
    assert!(!body.contains("__autoTuneChannelId"));
}
```

- [ ] **Step 2: Run tests to verify they fail**

Run: `cargo test --test http test_watch_known_channel_injects_auto_tune test_watch_unknown_channel_falls_back_to_guide test_guide_has_no_auto_tune`
Expected: FAIL — `/watch/1` and `/watch/999999` return 404 (route not registered), so the body/contains assertions fail.

- [ ] **Step 3: Split `GuidePageTemplate` out of the macro and add the `watch_page` handler**

In `src/routes/guide/mod.rs`, update the imports block (lines 6-13) to add `Path` and the `channel` model:

```rust
use askama::Template;
use axum::{
    extract::{Path, Query, State},
    http::StatusCode,
    response::Html,
};
use serde::Deserialize;

use crate::{model::channel, AppState};
```

Remove the `GuidePageTemplate` macro invocation. The macro line currently reads:

```rust
define_guide_template!(GuidePageTemplate, "guide.html");
define_guide_template!(EpgContentTemplate, "partials/epg_content.html");
```

Change it to keep the macro only for `EpgContentTemplate`:

```rust
define_guide_template!(EpgContentTemplate, "partials/epg_content.html");
```

Then hand-define `GuidePageTemplate` immediately after, with the extra `auto_tune_channel_id` field (this keeps the unused-field off `EpgContentTemplate`, so `-D warnings` stays clean):

```rust
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
    auto_tune_channel_id: Option<i64>,
}

impl From<GuideData> for GuidePageTemplate {
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
            auto_tune_channel_id: None,
        }
    }
}
```

Add the `watch_page` handler after `guide_page` (after line 113):

```rust
pub async fn watch_page(
    State(state): State<AppState>,
    Path(channel_id): Path<i64>,
    Query(params): Query<GuideQuery>,
) -> Result<Html<String>, StatusCode> {
    let exists = channel::get(&state.pool, channel_id)
        .await
        .map_err(|_| StatusCode::INTERNAL_SERVER_ERROR)?
        .is_some();
    let data = load_data(&state, params).await?;
    let mut tpl = GuidePageTemplate::from(data);
    if exists {
        tpl.auto_tune_channel_id = Some(channel_id);
    }
    render_or_500(tpl)
}
```

- [ ] **Step 4: Emit the JS global in `guide.html`**

In `templates/guide.html`, insert immediately after line 2 (`{% block content %}`), before `<div id="player-panel">`:

```html
{% match auto_tune_channel_id %}
{% when Some with (cid) %}
<script>window.__autoTuneChannelId = {{ cid }};</script>
{% when None %}
{% endmatch %}
```

(Project convention: Askama uses `{% match %}`/`{% when %}` for `Option`, not `{% if let %}`.)

- [ ] **Step 5: Read the global in `base.html`'s init**

In `templates/base.html`, immediately after line 462 (`window.tune = tune;`), still inside the `DOMContentLoaded` handler, add:

```js
      if (window.__autoTuneChannelId) {
        var autoCid = window.__autoTuneChannelId;
        history.replaceState(null, '', '/watch/' + autoCid);
        tune(autoCid);
      }
```

- [ ] **Step 6: Register the route**

In `src/lib.rs`, immediately after the `/guide` route (line 122), add:

```rust
        .route("/watch/:id", get(routes::guide::watch_page))
```

- [ ] **Step 7: Run the tests to verify they pass**

Run: `cargo test --test http test_watch_known_channel_injects_auto_tune test_watch_unknown_channel_falls_back_to_guide test_guide_has_no_auto_tune`
Expected: PASS (3 passed).

Then the full suite (catches any template-compile or routing regression):

Run: `cargo test`
Expected: PASS (all tests; new totals = prior + 5 from Tasks 1 & 2).

- [ ] **Step 8: Format, lint, commit**

```bash
cargo fmt
cargo clippy -- -D warnings
git add src/routes/guide/mod.rs templates/guide.html templates/base.html src/lib.rs tests/http.rs
git commit -m "feat: add /watch/:id deep-link that auto-tunes a channel"
```

Expected: `cargo clippy` exits 0 with no warnings.

---

## Final verification

- [ ] Run `cargo fmt --check` → no diff.
- [ ] Run `cargo clippy -- -D warnings` → exits 0.
- [ ] Run `cargo test` → all pass.
- [ ] Update the test count in `CLAUDE.md` (currently "342 tests: 269 unit + 73 integration") to reflect +5 integration tests (→ 78 integration, 347 total), and commit that doc change.
