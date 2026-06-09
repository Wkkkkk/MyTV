# Skip Stream-Proxy for YouTube VOD Implementation Plan

> **For agentic workers:** REQUIRED SUB-SKILL: Use superpowers:subagent-driven-development (recommended) or superpowers:executing-plans to implement this plan task-by-task. Steps use checkbox (`- [ ]`) syntax for tracking.

**Goal:** Eliminate unnecessary server egress for YouTube VOD by telling the client to skip `/stream-proxy` for URLs resolved via yt-dlp.

**Architecture:** Add `skip_proxy: bool` to `TuneResponse` on the server; the server sets it using the already-public `resolver::needs_resolution()` check. The client reads the flag and uses the original unproxied URL as `video.src` in the direct-MP4 branch; HLS and DASH branches are untouched.

**Tech Stack:** Rust/Axum (`src/routes/player.rs`), Askama template JS (`templates/base.html`), SQLx integration tests (`tests/http.rs`).

---

### Task 1: Add `skip_proxy` to `TuneResponse` and wire it through the server

**Files:**
- Modify: `src/routes/player.rs:20-28` — `TuneResponse` struct
- Modify: `src/routes/player.rs:72-81` — `tune_response()` helper
- Modify: `src/routes/player.rs:83-103` — `next_live()` call site
- Modify: `src/routes/player.rs:129-143` — `tune_vod_at()` call site
- Modify: `src/routes/player.rs:145-160` — `next_vod_at()` call site
- Test: `tests/http.rs`

- [ ] **Step 1: Write the failing test**

Add at the bottom of `tests/http.rs` (before the final `}`):

```rust
#[tokio::test]
async fn test_tune_skip_proxy_false_for_plain_hls() {
    let response = app().await.oneshot(req("/channel/1/tune")).await.unwrap();
    assert_eq!(response.status(), StatusCode::OK);
    let json = body_json(response).await;
    assert_eq!(json["skip_proxy"], false);
}
```

Channel 1's source URL is `https://stream.example.com/live.m3u8` — a plain HLS URL, so `needs_resolution()` returns `false` and `skip_proxy` must be `false`.

- [ ] **Step 2: Run test to verify it fails**

```bash
cargo test test_tune_skip_proxy_false_for_plain_hls -- --nocapture
```

Expected: FAIL — the field `skip_proxy` does not exist on `TuneResponse` yet, so the JSON object won't contain it and `json["skip_proxy"]` will be `null`, not `false`.

- [ ] **Step 3: Add `skip_proxy` to `TuneResponse` struct**

In `src/routes/player.rs`, replace the struct definition:

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
}
```

- [ ] **Step 4: Add `skip_proxy` param to `tune_response()` and populate it**

Replace the `tune_response` function:

```rust
fn tune_response(
    ch: &channel::Channel,
    url: String,
    start_offset_secs: i64,
    skip_proxy: bool,
) -> Json<TuneResponse> {
    Json(TuneResponse {
        url,
        start_offset_secs,
        name: ch.name.clone(),
        logo_url: ch.logo_url.clone(),
        category: ch.category.clone(),
        channel_type: ch.r#type.clone(),
        skip_proxy,
    })
}
```

- [ ] **Step 5: Update the three `tune_response` call sites**

In `next_live()`, replace:
```rust
Ok(url) => return Ok(tune_response(ch, url, 0)),
```
with:
```rust
Ok(url) => return Ok(tune_response(ch, url, 0, resolver::needs_resolution(&src.url))),
```

In `tune_vod_at()`, replace:
```rust
Ok(url) => Ok(tune_response(ch, url, offset)),
```
with:
```rust
Ok(url) => Ok(tune_response(ch, url, offset, resolver::needs_resolution(&item.url))),
```

In `next_vod_at()`, replace:
```rust
Ok(url) => Ok(tune_response(ch, url, 0)),
```
with:
```rust
Ok(url) => Ok(tune_response(ch, url, 0, resolver::needs_resolution(&item.url))),
```

- [ ] **Step 6: Run the new test to verify it passes**

```bash
cargo test test_tune_skip_proxy_false_for_plain_hls -- --nocapture
```

Expected: PASS

- [ ] **Step 7: Run the full test suite**

```bash
cargo test
```

Expected: all tests pass. Existing tune tests (`test_tune_live_ok_returns_stream_url`, `tune_response_includes_channel_metadata`, etc.) continue to pass because `skip_proxy` is an additive JSON field.

- [ ] **Step 8: Format and commit**

```bash
cargo fmt
git add src/routes/player.rs tests/http.rs
git commit -m "feat: add skip_proxy field to TuneResponse for yt-dlp resolved URLs"
```

---

### Task 2: Update `_loadSource` in `base.html` to honour `skip_proxy`

**Files:**
- Modify: `templates/base.html:226` — `_loadSource` signature
- Modify: `templates/base.html:308` — `video.src` assignment in the direct-MP4 else branch
- Modify: `templates/base.html:213,245,278,305,349,363` — 6 call sites

- [ ] **Step 1: Add `skipProxy` parameter to `_loadSource`**

In `templates/base.html`, replace:
```javascript
      function _loadSource(url, offset) {
```
with:
```javascript
      function _loadSource(url, offset, skipProxy) {
```

`skipProxy` is `undefined` (falsy) at any call site that doesn't pass it yet, so existing behaviour is unchanged while you update call sites in the next step.

- [ ] **Step 2: Use `currentUrl` instead of the proxied `url` when `skipProxy` is true**

`currentUrl` is assigned at line 227 (`currentUrl = url;`) before `url = proxyUrl(url)` mutates the variable at line 231. It therefore always holds the original, unproxied URL.

In the `else` branch, replace:
```javascript
          video.src = url;
```
with:
```javascript
          video.src = skipProxy ? currentUrl : url;
```

- [ ] **Step 3: Update all 6 `_loadSource` call sites to pass `d.skip_proxy`**

Each call site fetches a `TuneResponse` JSON object `d` and calls `_loadSource(d.url, d.start_offset_secs)`. Update every one to:

```javascript
_loadSource(d.url, d.start_offset_secs, d.skip_proxy)
```

The six locations (grep with `grep -n "_loadSource" templates/base.html` to confirm line numbers haven't shifted):

| Approx. line | Context |
|---|---|
| 213 | initial tune on page load, in the `.then()` after fetch |
| 245 | DASH `ERROR` event handler, after `/next` fetch |
| 278 | native HLS `video.onerror` handler, after `/next` fetch |
| 305 | direct-MP4 `video.onerror` handler, after `/next` fetch |
| 349 | a second tune call site |
| 363 | another tune/next call site |

- [ ] **Step 4: Verify with `cargo test` (server-side tests still pass)**

```bash
cargo test
```

Expected: all tests pass. The template change is JS-only and not covered by server-side tests.

- [ ] **Step 5: Format and commit**

```bash
cargo fmt
git add templates/base.html
git commit -m "feat: skip stream-proxy in _loadSource for yt-dlp resolved direct MP4 URLs"
```

---

### Task 3: Mark yt-dlp playlist items as Direct (⚡) in the CORS cache when Test is clicked

**Files:**
- Modify: `src/routes/admin/playlist.rs:119-141` — `playlist_item_test` handler

- [ ] **Step 1: Add a YouTube VOD seed item and write the failing test**

Append to `tests/fixtures/seed.sql` (after the existing `playlist_items` insert block):

```sql
-- YouTube VOD item for skip-proxy budget test; is_active=0 so VOD loop tests are unaffected
INSERT INTO playlist_items (channel_id, title, url, duration_secs, sort_order, is_active)
VALUES (4, 'YT Episode', 'https://www.youtube.com/watch?v=dQw4w9WgXcQ', 212, 3, 0);
```

The existing `playlist_items` rows have ids 1 and 2. This new row gets id 3. Setting `is_active = 0` keeps it out of the VOD loop so existing channel-4 tune tests are unaffected.

Add the test in `tests/http.rs`:

```rust
#[tokio::test]
async fn test_playlist_item_test_marks_youtube_as_direct_budget() {
    // seed.sql inserts playlist items with ids 1 (ep1) and 2 (ep2) for channel 4,
    // then the YouTube item as id 3 (is_active=0, excluded from VOD loop).
    let resp = app()
        .await
        .oneshot(authed_post("/admin/playlist/3/test"))
        .await
        .unwrap();
    assert_eq!(resp.status(), StatusCode::OK);
    let bytes = resp.into_body().collect().await.unwrap().to_bytes();
    let html = String::from_utf8_lossy(&bytes);
    assert!(html.contains('⚡'), "YouTube VOD item must show Direct (⚡) budget after Test");
}
```

- [ ] **Step 2: Run test to verify it fails**

```bash
cargo test test_playlist_item_test_marks_youtube_as_direct_budget -- --nocapture
```

Expected: FAIL — the response HTML contains blank budget (no ⚡) because the CORS cache is not populated for YouTube items.

- [ ] **Step 3: Add the CORS cache write to `playlist_item_test`**

In `src/routes/admin/playlist.rs`, update `playlist_item_test` — add the CORS cache write between the probe call and the re-fetch:

```rust
pub async fn playlist_item_test(
    State(state): State<AppState>,
    Path(item_id): Path<i64>,
) -> Result<Html<String>, StatusCode> {
    let item = playlist_item::get(&state.pool, item_id)
        .await
        .map_err(internal_error)?
        .ok_or(StatusCode::NOT_FOUND)?;

    crate::health::probe_playlist_item(&state.pool, &state.http_client, &state.cors_cache, &item)
        .await;

    if media::resolver::needs_resolution(&item.url) {
        let host = media::hls::extract_manifest_host(&item.url);
        state
            .cors_cache
            .write()
            .await
            .insert(host, std::time::Instant::now());
    }

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

`media` is already imported via `use crate::media;` at the top of `playlist.rs` — no import changes needed.

- [ ] **Step 4: Run the new test to verify it passes**

```bash
cargo test test_playlist_item_test_marks_youtube_as_direct_budget -- --nocapture
```

Expected: PASS — the returned HTML row contains ⚡.

- [ ] **Step 5: Run the full test suite**

```bash
cargo test
```

Expected: all tests pass. The new seed row (id 3) does not affect any existing test since no existing test references playlist item id 3.

- [ ] **Step 6: Format and commit**

```bash
cargo fmt
git add src/routes/admin/playlist.rs tests/fixtures/seed.sql tests/http.rs
git commit -m "feat: mark yt-dlp VOD playlist items as Direct budget when Test is clicked"
```
