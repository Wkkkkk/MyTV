# On-Demand VOD Channel Type (#45) Implementation Plan

> **For agentic workers:** REQUIRED SUB-SKILL: Use superpowers:subagent-driven-development (recommended) or superpowers:executing-plans to implement this plan task-by-task. Steps use checkbox (`- [ ]`) syntax for tracking.

**Goal:** Add a third channel type `vod_on_demand` — a viewer-controlled playlist where items play sequentially, the native `<video>` timeline handles seeking, the viewer clicks any item to jump/replay, position is remembered in the browser, and playback stops silently after the last item (no loop).

**Architecture:** A new `ChannelType::VodOnDemand` variant threads through the existing model/guide code (treated like `VodLoop` for guide/health, but with no `loop_anchor`). Two new public player endpoints — `GET /channel/:id/playlist` (item list) and `GET /channel/:id/item/:item_id` (resolve one item) — drive a client-side playlist UI in `templates/base.html`. The client keeps the playback cursor in `localStorage`; there is no server-side position state and no schema migration.

**Tech Stack:** Rust/Axum, SQLx (SQLite), Askama templates, plain CSS + vanilla JS. Tests via `tower::ServiceExt::oneshot` in `tests/http.rs`. Spec: `docs/superpowers/specs/2026-06-15-vod-on-demand-channel-design.md`.

---

## File Structure

**Modify:**
- `tests/fixtures/seed.sql` — add an on-demand channel (ID 6) + 2 items.
- `src/model/channel.rs` — `ChannelType::VodOnDemand` variant + `as_str`/`from_str`/`channel_type()`.
- `src/routes/guide/badges.rs` — `derive_channel_status` arm for the new type.
- `src/routes/guide/data.rs` — guide-row match arm for the new type; add `"type"` to `channels_json`.
- `src/routes/player.rs` — `tune_vod_on_demand` helper; `VodOnDemand` arms in `tune`/`next`; `playlist` + `item` handlers; `PlaylistEntry` struct.
- `src/lib.rs` — register `GET /channel/:id/playlist` and `GET /channel/:id/item/:item_id`.
- `templates/admin/channel_form.html` — add the `vod_on_demand` `<option>`.
- `templates/guide.html` — playlist toolbar button + list container markup.
- `templates/base.html` — playlist CSS + the on-demand client logic.
- `tests/http.rs` — integration tests for the endpoints, `channels_json`, and the admin option.

No new files; no migration (the `channels.type` column is free-form text and `loop_anchor` stays NULL for on-demand).

---

## Task 1: Seed fixture — on-demand channel

**Files:**
- Modify: `tests/fixtures/seed.sql`

- [ ] **Step 1: Add the channel and its items**

In `tests/fixtures/seed.sql`, change the `channels` INSERT to add row 6 (note the trailing comma moves to row 5):

```sql
INSERT INTO channels (id, name, category, logo_url, type, sort_order, loop_anchor) VALUES
  (1, 'Live OK',       'test', NULL, 'live',           1, NULL),
  (2, 'All Down',      'test', NULL, 'live',           2, NULL),
  (3, 'Has Fallback',  'test', NULL, 'live',           3, NULL),
  (4, 'VOD Has Items', 'test', NULL, 'vod_loop',       4, '2020-01-01 00:00:00'),
  (5, 'VOD Empty',     'test', NULL, 'vod_loop',       5, '2020-01-01 00:00:00'),
  (6, 'On Demand',     'test', NULL, 'vod_on_demand',  6, NULL);
```

Then append two on-demand items after the existing `playlist_items` inserts:

```sql
INSERT INTO playlist_items (channel_id, title, url, duration_secs, sort_order) VALUES
  (6, '点播 First',  'https://vod.example.com/od1.mp4', 120, 1),
  (6, 'On-Demand 2', 'https://vod.example.com/od2.mp4', 300, 2);
```

- [ ] **Step 2: Verify the seed still loads**

Run: `cargo test --test http test_guide_partial_returns_200`
Expected: PASS (the fresh in-memory DB seeds without error; existing tests untouched because channel 6 is a new row).

- [ ] **Step 3: Commit**

```bash
git add tests/fixtures/seed.sql
git commit -m "test(fixtures): add on-demand channel 6 for #45"
```

---

## Task 2: Backend — `VodOnDemand` channel type + tune/next

Adding an enum variant breaks every exhaustive `match` on `ChannelType`, so this task updates all of them together to keep the crate compiling.

**Files:**
- Modify: `src/model/channel.rs:22-54`
- Modify: `src/routes/guide/badges.rs:63-64`
- Modify: `src/routes/guide/data.rs:96-120`
- Modify: `src/routes/player.rs:47-73` (match arms) and add `tune_vod_on_demand`
- Test: `src/model/channel.rs` (unit, in the existing `mod tests`), `tests/http.rs`

- [ ] **Step 1: Write the failing unit test (enum round-trip)**

In `src/model/channel.rs`, inside the existing `#[cfg(test)] mod tests`, add:

```rust
#[test]
fn vod_on_demand_round_trips() {
    use std::str::FromStr;
    assert_eq!(ChannelType::VodOnDemand.as_str(), "vod_on_demand");
    assert_eq!(
        ChannelType::from_str("vod_on_demand").unwrap(),
        ChannelType::VodOnDemand
    );
    let ch = Channel {
        r#type: "vod_on_demand".to_string(),
        ..test_channel()
    };
    assert_eq!(ch.channel_type(), ChannelType::VodOnDemand);
}
```

If no `test_channel()` helper exists in that module, build the `Channel` the same way the nearest existing test in this module does (copy its construction); the only field that matters here is `r#type`.

- [ ] **Step 2: Run it to verify it fails**

Run: `cargo test -p mytv channel::tests::vod_on_demand_round_trips`
Expected: FAIL — `no variant named VodOnDemand` (compile error).

- [ ] **Step 3: Add the enum variant and string mappings**

In `src/model/channel.rs`:

```rust
pub enum ChannelType {
    Live,
    VodLoop,
    VodOnDemand,
}
```

In `channel_type()`:

```rust
    pub fn channel_type(&self) -> ChannelType {
        match self.r#type.as_str() {
            "vod_loop" => ChannelType::VodLoop,
            "vod_on_demand" => ChannelType::VodOnDemand,
            _ => ChannelType::Live,
        }
    }
```

In `as_str()`:

```rust
            ChannelType::Live => "live",
            ChannelType::VodLoop => "vod_loop",
            ChannelType::VodOnDemand => "vod_on_demand",
```

In `from_str()`:

```rust
            "live" => Ok(ChannelType::Live),
            "vod_loop" => Ok(ChannelType::VodLoop),
            "vod_on_demand" => Ok(ChannelType::VodOnDemand),
```

(`resolve_anchor` already returns `None` for any non-`VodLoop` type, so on-demand channels get a NULL anchor automatically — no change needed there.)

- [ ] **Step 4: Fix the guide health match (`badges.rs`)**

In `src/routes/guide/badges.rs`, change the `VodLoop` arm of `derive_channel_status` to also cover the new type:

```rust
    match channel_type {
        ChannelType::VodLoop | ChannelType::VodOnDemand => SourceStatus::Ok,
        ChannelType::Live => {
```

- [ ] **Step 5: Fix the guide-row match (`data.rs`)**

In `src/routes/guide/data.rs`, change the `ChannelType::VodLoop => { … }` arm to also match the new type (on-demand has no anchor, so `ch.loop_anchor` is `None` → empty schedule entries, which is correct — it's a playlist, not a broadcast):

```rust
            ChannelType::VodLoop | ChannelType::VodOnDemand => {
                let items = all_playlist_items.get(&ch.id).cloned().unwrap_or_default();
                let entries = match ch.loop_anchor {
                    Some(anchor) => epg::vod_schedule(
                        ch.id,
                        &items,
                        anchor.timestamp(),
                        window_start,
                        window_end,
                    ),
                    None => vec![],
                };
                let budget_url = vod_budget_url(&items, ch.loop_anchor, now);
                (entries, budget_url)
            }
```

- [ ] **Step 6: Add the `tune_vod_on_demand` helper (`player.rs`)**

In `src/routes/player.rs`, add this helper next to `tune_vod_at` (it does NOT use `loop_anchor`, so it cannot reuse `vod_items_and_index`):

```rust
async fn tune_vod_on_demand(
    state: &AppState,
    ch: &channel::Channel,
) -> Result<Json<TuneResponse>, StatusCode> {
    let items = playlist_item::list_active_for_channel(&state.pool, ch.id)
        .await
        .map_err(|_| StatusCode::INTERNAL_SERVER_ERROR)?;
    let item = items.first().ok_or(StatusCode::SERVICE_UNAVAILABLE)?;
    match resolver::resolve_url(&item.url).await {
        Ok(url) => Ok(tune_response(
            ch,
            url,
            0,
            resolver::should_skip_proxy(&item.url),
            None,
            Some(item.id),
        )),
        Err(e) => {
            tracing::warn!(url = %item.url, error = %e, "resolver failed for vod item");
            Err(StatusCode::SERVICE_UNAVAILABLE)
        }
    }
}
```

- [ ] **Step 7: Wire the `VodOnDemand` arm into `tune` and `next`**

In `src/routes/player.rs`, the `match ch.channel_type()` in `tune` becomes:

```rust
    match ch.channel_type() {
        ChannelType::Live => next_live(&state, &ch, None).await,
        ChannelType::VodLoop => {
            let now_secs = chrono::Utc::now().timestamp();
            tune_vod_at(&state, &ch, now_secs).await
        }
        ChannelType::VodOnDemand => tune_vod_on_demand(&state, &ch).await,
    }
```

And in `next` (a direct `/next` hit on an on-demand channel just returns the first item — the JS player path uses `/item/:id`, not `/next`):

```rust
    match ch.channel_type() {
        ChannelType::Live => next_live(&state, &ch, q.failed_url.as_deref()).await,
        ChannelType::VodLoop => {
            let now_secs = chrono::Utc::now().timestamp();
            next_vod_at(&state, &ch, now_secs).await
        }
        ChannelType::VodOnDemand => tune_vod_on_demand(&state, &ch).await,
    }
```

- [ ] **Step 8: Write the failing integration test (tune on-demand)**

In `tests/http.rs`, add near the other tune tests:

```rust
#[tokio::test]
async fn test_tune_on_demand_returns_first_item() {
    let response = app().await.oneshot(req("/channel/6/tune")).await.unwrap();
    assert_eq!(response.status(), StatusCode::OK);
    let body = body_text(response).await;
    assert!(body.contains("od1.mp4"), "should resolve the first active item");
    assert!(body.contains("\"channel_type\":\"vod_on_demand\""));
}
```

- [ ] **Step 9: Run the tests**

Run: `cargo test -p mytv channel::tests::vod_on_demand_round_trips && cargo test --test http test_tune_on_demand_returns_first_item`
Expected: PASS for both.

- [ ] **Step 10: Format, lint, commit**

```bash
cargo fmt
cargo clippy -- -D warnings
git add src/model/channel.rs src/routes/guide/badges.rs src/routes/guide/data.rs src/routes/player.rs tests/http.rs
git commit -m "feat(model): add vod_on_demand channel type + tune handling (#45)"
```

---

## Task 3: `GET /channel/:id/playlist` endpoint

**Files:**
- Modify: `src/routes/player.rs` (add `PlaylistEntry` + `playlist` handler)
- Modify: `src/lib.rs:131-132` (register route)
- Test: `tests/http.rs`

- [ ] **Step 1: Write the failing tests**

In `tests/http.rs`:

```rust
#[tokio::test]
async fn test_playlist_returns_items_in_order() {
    let response = app().await.oneshot(req("/channel/6/playlist")).await.unwrap();
    assert_eq!(response.status(), StatusCode::OK);
    let body = body_text(response).await;
    let i1 = body.find("od1.mp4").or(body.find("First")).unwrap_or(usize::MAX);
    // first item appears before the second
    assert!(body.contains("On-Demand 2"));
    assert!(body.contains("\"duration_secs\":120"));
    let _ = i1;
}

#[tokio::test]
async fn test_playlist_empty_for_channel_without_items() {
    // channel 5 is a vod_loop channel with no playlist items
    let response = app().await.oneshot(req("/channel/5/playlist")).await.unwrap();
    assert_eq!(response.status(), StatusCode::OK);
    assert_eq!(body_text(response).await, "[]");
}

#[tokio::test]
async fn test_playlist_404_for_missing_channel() {
    let response = app().await.oneshot(req("/channel/999/playlist")).await.unwrap();
    assert_eq!(response.status(), StatusCode::NOT_FOUND);
}
```

- [ ] **Step 2: Run to verify they fail**

Run: `cargo test --test http test_playlist_`
Expected: FAIL — 404 from the router (route not registered).

- [ ] **Step 3: Add the handler**

In `src/routes/player.rs`, add the struct (near `TuneResponse`) and handler (near `tune`):

```rust
#[derive(Debug, Serialize)]
pub struct PlaylistEntry {
    pub id: i64,
    pub title: String,
    pub duration_secs: i64,
}

pub async fn playlist(
    State(state): State<AppState>,
    Path(channel_id): Path<i64>,
) -> Result<Json<Vec<PlaylistEntry>>, StatusCode> {
    channel::get(&state.pool, channel_id)
        .await
        .map_err(|_| StatusCode::INTERNAL_SERVER_ERROR)?
        .ok_or(StatusCode::NOT_FOUND)?;

    let items = playlist_item::list_active_for_channel(&state.pool, channel_id)
        .await
        .map_err(|_| StatusCode::INTERNAL_SERVER_ERROR)?;

    Ok(Json(
        items
            .into_iter()
            .map(|i| PlaylistEntry {
                id: i.id,
                title: i.title,
                duration_secs: i.duration_secs,
            })
            .collect(),
    ))
}
```

- [ ] **Step 4: Register the route**

In `src/lib.rs`, after the `/channel/:id/next` line:

```rust
        .route("/channel/:id/next", get(routes::player::next))
        .route("/channel/:id/playlist", get(routes::player::playlist))
```

- [ ] **Step 5: Run the tests**

Run: `cargo test --test http test_playlist_`
Expected: PASS (all three).

- [ ] **Step 6: Format, lint, commit**

```bash
cargo fmt
cargo clippy -- -D warnings
git add src/routes/player.rs src/lib.rs tests/http.rs
git commit -m "feat(player): GET /channel/:id/playlist endpoint (#45)"
```

---

## Task 4: `GET /channel/:id/item/:item_id` endpoint

**Files:**
- Modify: `src/routes/player.rs` (add `item` handler)
- Modify: `src/lib.rs` (register route)
- Test: `tests/http.rs`

- [ ] **Step 1: Write the failing tests**

In `tests/http.rs` (channel 6's first item is seeded with a low id; we look it up via the playlist endpoint to avoid hardcoding):

```rust
#[tokio::test]
async fn test_item_resolves_direct_url() {
    let app = app().await;
    // find channel 6's first item id from the playlist endpoint
    let pl = app.clone().oneshot(req("/channel/6/playlist")).await.unwrap();
    let body = body_text(pl).await;
    let json: serde_json::Value = serde_json::from_str(&body).unwrap();
    let item_id = json[0]["id"].as_i64().unwrap();

    let response = app
        .oneshot(req(&format!("/channel/6/item/{item_id}")))
        .await
        .unwrap();
    assert_eq!(response.status(), StatusCode::OK);
    let body = body_text(response).await;
    assert!(body.contains("od1.mp4"));
    assert!(body.contains("\"start_offset_secs\":0"));
    assert!(body.contains(&format!("\"playlist_item_id\":{item_id}")));
}

#[tokio::test]
async fn test_item_404_for_missing_channel() {
    let response = app().await.oneshot(req("/channel/999/item/1")).await.unwrap();
    assert_eq!(response.status(), StatusCode::NOT_FOUND);
}

#[tokio::test]
async fn test_item_422_when_item_not_on_channel() {
    // channel 6 does not contain item id 999999
    let response = app().await.oneshot(req("/channel/6/item/999999")).await.unwrap();
    assert_eq!(response.status(), StatusCode::UNPROCESSABLE_ENTITY);
}
```

- [ ] **Step 2: Run to verify they fail**

Run: `cargo test --test http test_item_`
Expected: FAIL — 404 from the router (route not registered).

- [ ] **Step 3: Add the handler**

In `src/routes/player.rs`, near the `playlist` handler:

```rust
pub async fn item(
    State(state): State<AppState>,
    Path((channel_id, item_id)): Path<(i64, i64)>,
) -> Result<Json<TuneResponse>, StatusCode> {
    let ch = channel::get(&state.pool, channel_id)
        .await
        .map_err(|_| StatusCode::INTERNAL_SERVER_ERROR)?
        .ok_or(StatusCode::NOT_FOUND)?;

    let items = playlist_item::list_active_for_channel(&state.pool, channel_id)
        .await
        .map_err(|_| StatusCode::INTERNAL_SERVER_ERROR)?;

    let item = items
        .iter()
        .find(|i| i.id == item_id)
        .ok_or(StatusCode::UNPROCESSABLE_ENTITY)?;

    match resolver::resolve_url(&item.url).await {
        Ok(url) => Ok(tune_response(
            &ch,
            url,
            0,
            resolver::should_skip_proxy(&item.url),
            None,
            Some(item.id),
        )),
        Err(e) => {
            tracing::warn!(url = %item.url, error = %e, "resolver failed for vod item");
            Err(StatusCode::SERVICE_UNAVAILABLE)
        }
    }
}
```

- [ ] **Step 4: Register the route**

In `src/lib.rs`, after the `/channel/:id/playlist` line:

```rust
        .route("/channel/:id/playlist", get(routes::player::playlist))
        .route("/channel/:id/item/:item_id", get(routes::player::item))
```

- [ ] **Step 5: Run the tests**

Run: `cargo test --test http test_item_`
Expected: PASS (all three).

- [ ] **Step 6: Format, lint, commit**

```bash
cargo fmt
cargo clippy -- -D warnings
git add src/routes/player.rs src/lib.rs tests/http.rs
git commit -m "feat(player): GET /channel/:id/item/:item_id endpoint (#45)"
```

---

## Task 5: Expose channel `type` in `channels_json`

**Files:**
- Modify: `src/routes/guide/data.rs:57-64`
- Test: `tests/http.rs`

- [ ] **Step 1: Write the failing test**

In `tests/http.rs`:

```rust
#[tokio::test]
async fn test_guide_partial_channels_json_includes_type() {
    let response = app().await.oneshot(req("/guide/partial")).await.unwrap();
    assert_eq!(response.status(), StatusCode::OK);
    let body = body_text(response).await;
    assert!(
        body.contains("\"type\":\"vod_on_demand\""),
        "channels_json must carry channel type so the client can branch"
    );
}
```

- [ ] **Step 2: Run to verify it fails**

Run: `cargo test --test http test_guide_partial_channels_json_includes_type`
Expected: FAIL — `channels_json` currently emits only `id` and `name`.

- [ ] **Step 3: Add `type` to the JSON**

In `src/routes/guide/data.rs`, update the map closure:

```rust
    let channels_json = serde_json::to_string(
        &all_channels
            .iter()
            .map(|c| serde_json::json!({"id": c.id, "name": c.name, "type": c.r#type}))
            .collect::<Vec<_>>(),
    )
    .unwrap_or_else(|_| "[]".to_string())
    .replace("</", r"<\/");
```

- [ ] **Step 4: Run the test**

Run: `cargo test --test http test_guide_partial_channels_json_includes_type`
Expected: PASS.

- [ ] **Step 5: Commit**

```bash
cargo fmt
git add src/routes/guide/data.rs tests/http.rs
git commit -m "feat(guide): expose channel type in channels_json (#45)"
```

---

## Task 6: Admin — `vod_on_demand` dropdown option

**Files:**
- Modify: `templates/admin/channel_form.html:21-23`
- Test: `tests/http.rs`

- [ ] **Step 1: Write the failing test**

In `tests/http.rs`:

```rust
#[tokio::test]
async fn test_channel_new_form_has_on_demand_option() {
    let response = app()
        .await
        .oneshot(authed("/admin/channels/new"))
        .await
        .unwrap();
    assert_eq!(response.status(), StatusCode::OK);
    let body = body_text(response).await;
    assert!(body.contains("value=\"vod_on_demand\""));
}
```

- [ ] **Step 2: Run to verify it fails**

Run: `cargo test --test http test_channel_new_form_has_on_demand_option`
Expected: FAIL — the option is not in the template.

- [ ] **Step 3: Add the option**

In `templates/admin/channel_form.html`, after the `vod_loop` option:

```html
      <option value="live"{% if channel_type.as_str() == "live" %} selected{% endif %}>Live stream</option>
      <option value="vod_loop"{% if channel_type.as_str() == "vod_loop" %} selected{% endif %}>VOD loop</option>
      <option value="vod_on_demand"{% if channel_type.as_str() == "vod_on_demand" %} selected{% endif %}>On-demand playlist</option>
```

- [ ] **Step 4: Run the test**

Run: `cargo test --test http test_channel_new_form_has_on_demand_option`
Expected: PASS.

- [ ] **Step 5: Commit**

```bash
git add templates/admin/channel_form.html tests/http.rs
git commit -m "feat(admin): on-demand option in channel type dropdown (#45)"
```

---

## Task 7: Client — playlist toolbar markup + CSS

**Files:**
- Modify: `templates/guide.html:8-28`
- Modify: `templates/base.html` (CSS block near `#player-help`, ~line 44)
- Test: `tests/http.rs`

- [ ] **Step 1: Write the failing test**

In `tests/http.rs`:

```rust
#[tokio::test]
async fn test_guide_has_playlist_toolbar_markup() {
    let response = app().await.oneshot(req("/guide")).await.unwrap();
    assert_eq!(response.status(), StatusCode::OK);
    let body = body_text(response).await;
    assert!(body.contains("id=\"ov-playlist\""), "playlist toggle button");
    assert!(body.contains("id=\"player-playlist\""), "playlist list container");
}
```

- [ ] **Step 2: Run to verify it fails**

Run: `cargo test --test http test_guide_has_playlist_toolbar_markup`
Expected: FAIL — markup not present.

- [ ] **Step 3: Add the toolbar button + list container**

In `templates/guide.html`, add the playlist toggle button in `#player-toolbar` (before the `?` help button) and a list container after `#player-help`:

```html
    <button type="button" class="ov-btn" id="ov-next" title="Next channel" aria-label="Next channel">↓</button>
    <span class="ov-spacer"></span>
    <button type="button" class="ov-btn" id="ov-playlist" title="Playlist" aria-label="Playlist" hidden>☰</button>
    <button type="button" class="ov-btn" id="ov-help" title="Keyboard shortcuts" aria-label="Keyboard shortcuts">?</button>
  </div>
  <div id="player-help" hidden>
    <strong>Keyboard shortcuts</strong>
    <div>↑ / ↓ — change channel</div>
    <div>Space — play / pause</div>
    <div>← / → — seek 10s (VOD)</div>
    <div>F — fullscreen</div>
  </div>
  <div id="player-playlist" hidden></div>
```

- [ ] **Step 4: Add the CSS**

In `templates/base.html`, after the `#player-help` rules (~line 47), add:

```css
    #player-playlist{position:absolute;bottom:100px;right:12px;z-index:7;
      max-height:50vh;width:min(360px,80vw);overflow-y:auto;
      background:rgba(0,0,0,0.92);border:1px solid var(--border);border-radius:6px;
      opacity:0;pointer-events:none;transition:opacity .15s}
    #player-panel.show-controls #player-playlist:not([hidden]){opacity:1;pointer-events:auto}
    .pl-row{display:flex;align-items:center;gap:8px;padding:8px 10px;cursor:pointer;
      line-height:1.4;border-bottom:1px solid var(--border-subtle)}
    .pl-row:hover{background:rgba(255,255,255,0.08)}
    .pl-row.current{background:var(--accent);color:#fff}
    .pl-mark{flex:0 0 14px;text-align:center}
    .pl-title{flex:1 1 auto;min-width:0;white-space:nowrap;overflow:hidden;text-overflow:ellipsis}
    .pl-dur{flex:0 0 auto;font-variant-numeric:tabular-nums;color:var(--text-muted)}
    .pl-row.current .pl-dur{color:#fff}
```

- [ ] **Step 5: Run the test + build**

Run: `cargo test --test http test_guide_has_playlist_toolbar_markup`
Expected: PASS (Askama compiles the templates as part of the build).

- [ ] **Step 6: Commit**

```bash
git add templates/guide.html templates/base.html tests/http.rs
git commit -m "feat(player): playlist toolbar markup + styles (#45)"
```

---

## Task 8: Client — on-demand playback logic

Vanilla JS in `templates/base.html`. No JS unit tests (project convention); verified manually in Step 4 and by the markup test from Task 7.

**Files:**
- Modify: `templates/base.html` (the `tune` function ~line 468; the overlay-toolbar block ~line 528; the `video 'ended'` handler ~line 496)

- [ ] **Step 1: Add the on-demand module (state, helpers, rendering)**

In `templates/base.html`, inside the main script (place it just before `function tune(channelId) {` at ~line 468), add:

```javascript
      // ── on-demand VOD (#45) ───────────────────────────────────
      var odItems = [];          // [{id, title, duration_secs}]
      var odIndex = -1;          // index into odItems of the current item
      var odChannelId = null;    // channel id the playlist belongs to

      function odKey(id) { return 'mytv:ondemand:' + id; }

      function odLoadCursor(id) {
        try {
          var raw = localStorage.getItem(odKey(id));
          if (!raw) return null;
          var c = JSON.parse(raw);
          if (c && typeof c.itemId === 'number') return c;
        } catch (e) {}
        return null;
      }

      function odSaveCursor() {
        if (odChannelId == null || odIndex < 0 || !odItems[odIndex]) return;
        try {
          localStorage.setItem(odKey(odChannelId), JSON.stringify({
            itemId: odItems[odIndex].id,
            offset: video ? video.currentTime || 0 : 0
          }));
        } catch (e) {}
      }

      function odFmtDur(secs) {
        secs = Math.max(0, Math.floor(secs || 0));
        var m = Math.floor(secs / 60), s = secs % 60;
        return m + ':' + (s < 10 ? '0' : '') + s;
      }

      function odRenderList() {
        var box = document.getElementById('player-playlist');
        if (!box) return;
        box.innerHTML = '';
        odItems.forEach(function(it, i) {
          var row = document.createElement('div');
          row.className = 'pl-row' + (i === odIndex ? ' current' : '');
          row.setAttribute('role', 'button');
          row.setAttribute('tabindex', '0');
          var mark = document.createElement('span');
          mark.className = 'pl-mark';
          mark.textContent = i === odIndex ? '▶' : '';
          var title = document.createElement('span');
          title.className = 'pl-title';
          title.textContent = it.title;
          title.title = it.title;
          var dur = document.createElement('span');
          dur.className = 'pl-dur';
          dur.textContent = odFmtDur(it.duration_secs);
          row.appendChild(mark); row.appendChild(title); row.appendChild(dur);
          row.addEventListener('click', function() { odPlayIndex(i, 0); });
          box.appendChild(row);
        });
      }

      function odPlayIndex(i, offset) {
        if (i < 0 || i >= odItems.length) return;
        odIndex = i;
        odRenderList();
        fetch('/channel/' + odChannelId + '/item/' + odItems[i].id)
          .then(function(r) { if (!r.ok) throw new Error('item ' + r.status); return r.json(); })
          .then(function(d) {
            currentChannel = Object.assign({ channel_id: odChannelId }, d);
            renderInfoBar(currentChannel);
            if (video) video.style.display = '';
            hidePlayerError();
            _loadSource(d.url, offset || 0, d.skip_proxy);
            odSaveCursor();
          })
          .catch(function(err) {
            if (typeof debugLog === 'function') debugLog('error', 'on-demand item: ' + err);
            showPlayerError();
          });
      }

      function odTune(channelId) {
        odChannelId = channelId;
        odItems = [];
        odIndex = -1;
        var btn = document.getElementById('ov-playlist');
        if (btn) btn.hidden = false;
        document.getElementById('player-panel').style.display = 'block';
        fetch('/channel/' + channelId + '/playlist')
          .then(function(r) { if (!r.ok) throw new Error('playlist ' + r.status); return r.json(); })
          .then(function(items) {
            odItems = items || [];
            if (!odItems.length) { showPlayerError(); return; }
            var cursor = odLoadCursor(channelId);
            var start = 0, offset = 0;
            if (cursor) {
              for (var i = 0; i < odItems.length; i++) {
                if (odItems[i].id === cursor.itemId) { start = i; offset = cursor.offset || 0; break; }
              }
            }
            odRenderList();
            odPlayIndex(start, offset);
          })
          .catch(function(err) {
            if (typeof debugLog === 'function') debugLog('error', 'on-demand tune: ' + err);
            showPlayerError();
          });
      }

      function odChannelType(channelId) {
        var channels = window.epgChannels || [];
        for (var i = 0; i < channels.length; i++) {
          if (channels[i].id === channelId) return channels[i].type;
        }
        return null;
      }
```

- [ ] **Step 2: Branch `tune()` to the on-demand path and reset state elsewhere**

In `templates/base.html`, at the very top of `function tune(channelId) {` (right after `currentChannelId = channelId;`), branch out for on-demand and clear the toolbar button for other types:

```javascript
      function tune(channelId) {
        currentChannelId = channelId;
        currentUrl = null;
        if (odChannelType(channelId) === 'vod_on_demand') {
          hidePlayerError();
          odTune(channelId);
          return;
        }
        var plBtn = document.getElementById('ov-playlist');
        if (plBtn) plBtn.hidden = true;
        odChannelId = null; odItems = []; odIndex = -1;
        hidePlayerError();
        // ── existing body continues unchanged below ──
```

(Leave the rest of the existing `tune` body as-is; only the branch + reset lines are inserted after the existing `hidePlayerError();` is removed from its original spot — ensure `hidePlayerError()` is called exactly once on the non-on-demand path.)

- [ ] **Step 3: Wire the playlist toggle button, item-advance on `ended`, and cursor persistence**

In `templates/base.html`, in the overlay-toolbar block (after `ovHelp` wiring, ~line 545), add the playlist toggle:

```javascript
      var ovPlaylist = document.getElementById('ov-playlist');
      var plBox = document.getElementById('player-playlist');
      if (ovPlaylist) ovPlaylist.addEventListener('click', function() {
        if (plBox) plBox.hidden = !plBox.hidden;
      });
```

Replace the existing `video 'ended'` handler (~line 496-505) so on-demand advances by item and stops silently at the end:

```javascript
      if (video) {
        video.addEventListener('ended', function() {
          if (!currentChannelId) return;
          if (odChannelId === currentChannelId) {
            if (odIndex + 1 < odItems.length) odPlayIndex(odIndex + 1, 0);
            // else: last item — stop silently (no auto-advance)
            return;
          }
          fetch('/channel/' + currentChannelId + '/next')
            .then(function(r) { if (!r.ok) throw new Error('next failed: ' + r.status); return r.json(); })
            .then(function(d) { applyTuneResponse(d); })
            .catch(function(err) { if (typeof debugLog === 'function') debugLog('error', 'next: ' + err); console.error('next error:', err); showPlayerError(); });
        });
      }
```

Add cursor persistence near the other `video` listeners (after the buffering listeners, ~line 563):

```javascript
      if (video) {
        var odSaveT = null;
        video.addEventListener('timeupdate', function() {
          if (odChannelId == null) return;
          if (odSaveT) return;
          odSaveT = setTimeout(function() { odSaveT = null; odSaveCursor(); }, 2000);
        });
        video.addEventListener('pause', odSaveCursor);
      }
      window.addEventListener('beforeunload', odSaveCursor);
```

- [ ] **Step 4: Manual verification**

Run: `cargo run` (server on :3000). Then:
1. In the admin UI, create a channel of type **On-demand playlist** (or reuse the seeded channel by running against the test DB), add 2–3 MP4 items.
2. Open `/watch/<id>` for that channel. Confirm: first item plays from 0; the `☰` toolbar button appears; clicking it shows the item list with titles (try a Chinese title) aligned and durations right-aligned.
3. Click a different item → it plays from its start and the highlight moves.
4. Let an item play to the end → the next item auto-plays; let the last item end → playback stops, no channel hop.
5. Drag the native `<video>` timeline → seeking works.
6. Reload the page → playback resumes on the same item near where you left off.

Expected: all six behaviors hold. If `cargo run` against the seeded test data is impractical, verify against a real on-demand channel created in admin.

- [ ] **Step 5: Build, format, commit**

```bash
cargo build
cargo fmt
git add templates/base.html
git commit -m "feat(player): on-demand VOD playback + playlist navigation (#45)"
```

---

## Task 9: Docs — mark idea done

**Files:**
- Modify: `docs/IDEAS.md`
- Modify: `docs/CHANGELOG.md`

- [ ] **Step 1: Move #45 from Open to Done**

In `docs/IDEAS.md`, remove the `#45` block from `## Open` and bump the count in the `## Done` line (44 → 45 completed ideas, and note #45 in the parenthetical).

- [ ] **Step 2: Add a CHANGELOG entry**

In `docs/CHANGELOG.md`, add a `### #45 — On-demand VOD channel type` section summarizing: new `vod_on_demand` type; `/channel/:id/playlist` + `/channel/:id/item/:item_id` endpoints; browser-side cursor (localStorage); clickable playlist toolbar with CJK-safe row layout; no loop / stop-silently; native timeline for seeking. Reference the spec and this plan.

- [ ] **Step 3: Run the full suite**

Run: `cargo fmt --check && cargo clippy -- -D warnings && cargo test`
Expected: all green.

- [ ] **Step 4: Commit**

```bash
git add docs/IDEAS.md docs/CHANGELOG.md
git commit -m "docs(ideas): mark #45 done — on-demand VOD channel type"
```

---

## Self-Review Notes

- **Spec coverage:** new channel type (Task 2), no migration (Task 2 — string column + NULL anchor), `/playlist` (Task 3), `/item/:id` with 404/422/503 (Task 4), `channels_json.type` (Task 5), admin dropdown (Task 6), playlist UI w/ CJK-safe rows (Task 7), client cursor + sequential play + stop-silently + native seek + keyboard-unchanged (Task 8), tests + fixtures (Tasks 1–7), docs (Task 9). All spec sections map to a task.
- **503 path:** `/item/:id` returns 503 on resolve failure (Task 4 handler), but there is no offline integration test for it — forcing a resolve failure needs yt-dlp/network, which the suite avoids. It shares the exact resolve+`tune_response` path as `tune_vod_at`, and the 404/422/happy-path are covered. Documented here rather than silently skipped.
- **Type consistency:** `tune_response(ch, url, offset, skip_proxy, source, playlist_item_id)` signature matches existing usage; `PlaylistEntry` fields (`id`,`title`,`duration_secs`) match the client's `odItems` access and the row renderer; localStorage cursor shape `{itemId, offset}` is written by `odSaveCursor` and read by `odLoadCursor`/`odTune` consistently.
