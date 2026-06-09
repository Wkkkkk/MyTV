# Auto-convert ended YouTube live streams to VOD — Implementation Plan

> **For agentic workers:** REQUIRED SUB-SKILL: Use superpowers:subagent-driven-development (recommended) or superpowers:executing-plans to implement this plan task-by-task. Steps use checkbox (`- [ ]`) syntax for tracking.

**Goal:** When a tuned YouTube-live broadcast has ended, skip the viewer to the next channel instead of black-screening, and convert the dead channel into a looping VOD of the recording.

**Architecture:** At tune time, `next_live` detects `force_finished/1` in the resolved manifest URL, returns an `ended: true` response (no broken URL), and spawns a background task that builds the canonical `watch?v=` URL, fetches the duration, and rewrites the channel into a `vod_loop` (new playlist_item + `channel.type` flip + sources deactivated). The frontend shows a brief overlay and auto-tunes the next channel in the lineup.

**Tech Stack:** Rust / Axum / SQLx (SQLite), yt-dlp via `tokio::process::Command`, vanilla JS in an Askama template.

---

## File Structure

- `src/media/resolver.rs` — add pure helpers `is_finished_live`, `live_url_to_watch_url`, and the yt-dlp glue `fetch_video_id`.
- `src/model/source.rs` — add `deactivate_all_for_channel`.
- `src/model/channel.rs` — add focused setter `set_type_and_anchor`.
- `src/routes/player.rs` — add `ended` to `TuneResponse`, the `tune_response_ended` helper, the DB orchestration `convert_channel_to_vod_loop`, the background wrapper `live_to_vod_conversion` + `spawn_live_to_vod_conversion`, and the detection branch in `next_live`.
- `templates/base.html` — shared `applyTuneResponse` handler, `nextChannelId` helper, `advanceEndedChannel` + overlay markup, and routing all tune/next callbacks through the shared handler.
- `tests/http.rs` — `app_with_pool` helper + integration test for the ended path.

---

## Task 1: Resolver pure helpers (`is_finished_live`, `live_url_to_watch_url`)

**Files:**
- Modify: `src/media/resolver.rs` (add functions after `needs_resolution`, tests in the existing `#[cfg(test)] mod tests`)

- [ ] **Step 1: Write the failing tests**

Add to `mod tests` in `src/media/resolver.rs`:

```rust
#[test]
fn test_is_finished_live() {
    assert!(is_finished_live(
        "https://r5---sn-x.googlevideo.com/a/force_finished/1/b/index.m3u8"
    ));
    assert!(!is_finished_live(
        "https://r5---sn-x.googlevideo.com/a/id/abc/b/index.m3u8"
    ));
}

#[test]
fn test_live_url_to_watch_url() {
    assert_eq!(
        live_url_to_watch_url("https://www.youtube.com/live/abc123"),
        Some("https://www.youtube.com/watch?v=abc123".to_string())
    );
    assert_eq!(
        live_url_to_watch_url("https://youtu.be/abc123?feature=share"),
        Some("https://www.youtube.com/watch?v=abc123".to_string())
    );
    assert_eq!(
        live_url_to_watch_url("https://www.youtube.com/@somechannel/live"),
        None
    );
    assert_eq!(
        live_url_to_watch_url("https://www.youtube.com/watch?v=abc123"),
        None
    );
}
```

- [ ] **Step 2: Run tests to verify they fail**

Run: `cargo test --lib media::resolver::tests::test_is_finished_live media::resolver::tests::test_live_url_to_watch_url`
Expected: FAIL — `cannot find function is_finished_live` / `live_url_to_watch_url`.

- [ ] **Step 3: Write the implementation**

Add after `needs_resolution` (around line 9) in `src/media/resolver.rs`:

```rust
/// Returns true if a resolved YouTube manifest URL belongs to an ended live
/// broadcast. yt-dlp marks finished live HLS manifests with `force_finished/1`,
/// which leaves the player on a frozen playlist (black screen).
pub fn is_finished_live(resolved_url: &str) -> bool {
    resolved_url.contains("force_finished/1")
}

/// Rewrites a YouTube *live* URL that embeds a video id into the canonical
/// `watch?v=<id>` form, which yt-dlp resolves to the recorded MP4 once the
/// broadcast ends. Returns `None` for forms with no id in the path
/// (channel/handle `/live`) and for URLs already in `watch?v=` form.
pub fn live_url_to_watch_url(source_url: &str) -> Option<String> {
    let tail = source_url
        .split("youtube.com/live/")
        .nth(1)
        .or_else(|| source_url.split("youtu.be/").nth(1))?;
    let id = tail.split(['?', '&', '/']).next().unwrap_or("").trim();
    if id.is_empty() {
        return None;
    }
    Some(format!("https://www.youtube.com/watch?v={id}"))
}
```

- [ ] **Step 4: Run tests to verify they pass**

Run: `cargo test --lib media::resolver::tests::test_is_finished_live media::resolver::tests::test_live_url_to_watch_url`
Expected: PASS (2 tests).

- [ ] **Step 5: Format and commit**

```bash
cargo fmt
git add src/media/resolver.rs
git commit -m "feat: resolver helpers to detect ended YouTube live and build watch URL"
```

---

## Task 2: `source::deactivate_all_for_channel`

**Files:**
- Modify: `src/model/source.rs` (function after `set_active`, test in `mod tests`)

- [ ] **Step 1: Write the failing test**

Add to `mod tests` in `src/model/source.rs` (helpers `test_pool`, `make_channel`, `hls` already exist):

```rust
#[tokio::test]
async fn test_deactivate_all_for_channel() {
    let pool = test_pool().await;
    let ch = make_channel(&pool).await;
    create(&pool, hls(ch.id, "https://a.example.com/s.m3u8", 1))
        .await
        .unwrap();
    create(&pool, hls(ch.id, "https://b.example.com/s.m3u8", 2))
        .await
        .unwrap();

    deactivate_all_for_channel(&pool, ch.id).await.unwrap();

    assert!(list_active_for_channel(&pool, ch.id).await.unwrap().is_empty());
    assert_eq!(
        list_for_channel(&pool, ch.id).await.unwrap().len(),
        2,
        "rows are kept, only is_active flips"
    );
}
```

- [ ] **Step 2: Run test to verify it fails**

Run: `cargo test --lib model::source::tests::test_deactivate_all_for_channel`
Expected: FAIL — `cannot find function deactivate_all_for_channel`.

- [ ] **Step 3: Write the implementation**

Add after `set_active` (around line 148) in `src/model/source.rs`:

```rust
/// Deactivate every source for a channel. Used when an ended YouTube live is
/// converted to a VOD loop; rows are kept for reference, only is_active flips.
pub async fn deactivate_all_for_channel(pool: &SqlitePool, channel_id: i64) -> Result<()> {
    sqlx::query("UPDATE sources SET is_active = 0 WHERE channel_id = ?")
        .bind(channel_id)
        .execute(pool)
        .await?;
    Ok(())
}
```

- [ ] **Step 4: Run test to verify it passes**

Run: `cargo test --lib model::source::tests::test_deactivate_all_for_channel`
Expected: PASS.

- [ ] **Step 5: Format and commit**

```bash
cargo fmt
git add src/model/source.rs
git commit -m "feat: source::deactivate_all_for_channel"
```

---

## Task 3: `channel::set_type_and_anchor`

**Files:**
- Modify: `src/model/channel.rs` (function after `update`, test in `mod tests`)

- [ ] **Step 1: Write the failing test**

Add to `mod tests` in `src/model/channel.rs`:

```rust
#[tokio::test]
async fn test_set_type_and_anchor_flips_to_vod_loop() {
    use chrono::TimeZone;
    let pool = test_pool().await;
    let ch = create(
        &pool,
        NewChannel {
            name: "X".into(),
            category: "c".into(),
            logo_url: None,
            channel_type: ChannelType::Live,
            sort_order: 0,
            loop_anchor: None,
        },
    )
    .await
    .unwrap();
    assert_eq!(ch.channel_type(), ChannelType::Live);

    let anchor = Utc.timestamp_opt(1_700_000_000, 0).unwrap();
    set_type_and_anchor(&pool, ch.id, ChannelType::VodLoop, Some(anchor))
        .await
        .unwrap();

    let updated = get(&pool, ch.id).await.unwrap().unwrap();
    assert_eq!(updated.channel_type(), ChannelType::VodLoop);
    assert_eq!(updated.loop_anchor, Some(anchor));
}
```

Note: if `test_pool` does not already exist in this module, add it alongside the test:

```rust
async fn test_pool() -> SqlitePool {
    crate::db::connect("sqlite::memory:").await.unwrap()
}
```

(Check the top of `mod tests` first — only add it if missing, to avoid a duplicate-definition error.)

- [ ] **Step 2: Run test to verify it fails**

Run: `cargo test --lib model::channel::tests::test_set_type_and_anchor_flips_to_vod_loop`
Expected: FAIL — `cannot find function set_type_and_anchor`.

- [ ] **Step 3: Write the implementation**

Add after `update` (around line 122) in `src/model/channel.rs`:

```rust
/// Set a channel's playback type and loop anchor. Used by the ended-live → VOD
/// conversion to flip a `live` channel into a `vod_loop`.
pub async fn set_type_and_anchor(
    pool: &SqlitePool,
    id: i64,
    channel_type: ChannelType,
    loop_anchor: Option<DateTime<Utc>>,
) -> Result<()> {
    sqlx::query("UPDATE channels SET type = ?, loop_anchor = ? WHERE id = ?")
        .bind(channel_type.as_str())
        .bind(loop_anchor)
        .bind(id)
        .execute(pool)
        .await?;
    Ok(())
}
```

- [ ] **Step 4: Run test to verify it passes**

Run: `cargo test --lib model::channel::tests::test_set_type_and_anchor_flips_to_vod_loop`
Expected: PASS.

- [ ] **Step 5: Format and commit**

```bash
cargo fmt
git add src/model/channel.rs
git commit -m "feat: channel::set_type_and_anchor"
```

---

## Task 4: DB orchestration `convert_channel_to_vod_loop`

**Files:**
- Modify: `src/routes/player.rs` (module-private fn near `next_live`; test in `mod tests` using existing `test_state` / `make_live_channel` helpers)

- [ ] **Step 1: Write the failing test**

Add to `mod tests` in `src/routes/player.rs`:

```rust
#[tokio::test]
async fn test_convert_channel_to_vod_loop() {
    let state = test_state().await;
    let ch = make_live_channel(&state).await;
    source::create(
        &state.pool,
        source::NewSource {
            channel_id: ch.id,
            kind: source::SourceKind::YoutubeLive,
            url: "https://www.youtube.com/live/abc123".into(),
            priority: 1,
        },
    )
    .await
    .unwrap();

    let anchor = DateTime::from_timestamp(1_700_000_000, 0).unwrap();
    convert_channel_to_vod_loop(
        &state.pool,
        ch.id,
        "Live Test",
        "https://www.youtube.com/watch?v=abc123",
        212,
        anchor,
    )
    .await
    .unwrap();

    let updated = channel::get(&state.pool, ch.id).await.unwrap().unwrap();
    assert_eq!(updated.channel_type(), channel::ChannelType::VodLoop);
    assert_eq!(updated.loop_anchor, Some(anchor));

    let items = playlist_item::list_active_for_channel(&state.pool, ch.id)
        .await
        .unwrap();
    assert_eq!(items.len(), 1);
    assert_eq!(items[0].url, "https://www.youtube.com/watch?v=abc123");
    assert_eq!(items[0].duration_secs, 212);
    assert_eq!(items[0].title, "Live Test");

    assert!(source::list_active_for_channel(&state.pool, ch.id)
        .await
        .unwrap()
        .is_empty());

    // Idempotent: a second run on an already-converted channel is a no-op.
    convert_channel_to_vod_loop(
        &state.pool,
        ch.id,
        "Live Test",
        "https://www.youtube.com/watch?v=abc123",
        212,
        anchor,
    )
    .await
    .unwrap();
    assert_eq!(
        playlist_item::list_active_for_channel(&state.pool, ch.id)
            .await
            .unwrap()
            .len(),
        1,
        "second conversion must not append a duplicate item"
    );
}
```

- [ ] **Step 2: Run test to verify it fails**

Run: `cargo test --lib routes::player::tests::test_convert_channel_to_vod_loop`
Expected: FAIL — `cannot find function convert_channel_to_vod_loop`.

- [ ] **Step 3: Write the implementation**

Add near `next_live` (after line 118) in `src/routes/player.rs`. `channel`, `source`, `playlist_item`, and `ChannelType` are already imported at the top of the file:

```rust
/// DB-only conversion of an ended live channel into a VOD loop: append the
/// recording as a playlist item, flip the channel to vod_loop anchored at
/// `anchor`, and deactivate the (now-finished) live sources. Idempotent: a
/// channel already in vod_loop is left untouched.
async fn convert_channel_to_vod_loop(
    pool: &sqlx::SqlitePool,
    channel_id: i64,
    title: &str,
    watch_url: &str,
    duration_secs: i64,
    anchor: chrono::DateTime<chrono::Utc>,
) -> anyhow::Result<()> {
    let Some(ch) = channel::get(pool, channel_id).await? else {
        anyhow::bail!("channel {channel_id} not found");
    };
    if ch.channel_type() == ChannelType::VodLoop {
        return Ok(());
    }
    playlist_item::create(
        pool,
        playlist_item::NewPlaylistItem {
            channel_id,
            title: title.to_string(),
            url: watch_url.to_string(),
            duration_secs,
            sort_order: 0,
        },
    )
    .await?;
    channel::set_type_and_anchor(pool, channel_id, ChannelType::VodLoop, Some(anchor)).await?;
    source::deactivate_all_for_channel(pool, channel_id).await?;
    Ok(())
}
```

- [ ] **Step 4: Run test to verify it passes**

Run: `cargo test --lib routes::player::tests::test_convert_channel_to_vod_loop`
Expected: PASS.

- [ ] **Step 5: Format and commit**

```bash
cargo fmt
git add src/routes/player.rs
git commit -m "feat: convert_channel_to_vod_loop DB orchestration"
```

---

## Task 5: yt-dlp glue `resolver::fetch_video_id`

**Files:**
- Modify: `src/media/resolver.rs` (function after `fetch_duration_secs`; `#[ignore]` network test in `mod tests`)

- [ ] **Step 1: Write the failing test**

Add to `mod tests` in `src/media/resolver.rs`:

```rust
#[tokio::test]
#[ignore = "requires yt-dlp installed and network access — run manually"]
async fn test_fetch_video_id_returns_id() {
    let url = "https://www.youtube.com/watch?v=dQw4w9WgXcQ";
    let id = fetch_video_id(url).await.unwrap();
    assert_eq!(id, "dQw4w9WgXcQ");
}
```

- [ ] **Step 2: Run test to verify it fails to compile**

Run: `cargo test --lib media::resolver`
Expected: FAIL — `cannot find function fetch_video_id` (compile error; the `#[ignore]` test still must compile).

- [ ] **Step 3: Write the implementation**

Add after `fetch_duration_secs` (around line 104) in `src/media/resolver.rs`:

```rust
/// Fetches the canonical video id of a YouTube URL via yt-dlp. Used to build a
/// `watch?v=<id>` URL when an ended live source carries no id in its path
/// (channel/handle live URLs).
pub async fn fetch_video_id(url: &str) -> Result<String> {
    if !url.starts_with("http://") && !url.starts_with("https://") {
        bail!("invalid URL scheme: {}", url);
    }
    let output = tokio::time::timeout(
        Duration::from_secs(30),
        Command::new("yt-dlp")
            .args(["--print", "id", "--no-playlist", "--", url])
            .output(),
    )
    .await
    .map_err(|_| anyhow::anyhow!("yt-dlp timed out after 30s for {}", url))??;

    if !output.status.success() {
        bail!(
            "yt-dlp failed for {}: {}",
            url,
            String::from_utf8_lossy(&output.stderr).trim()
        );
    }
    let id = String::from_utf8_lossy(&output.stdout).trim().to_string();
    if id.is_empty() {
        bail!("yt-dlp returned empty id for {}", url);
    }
    Ok(id)
}
```

- [ ] **Step 4: Run test to verify it compiles (and is ignored)**

Run: `cargo test --lib media::resolver`
Expected: PASS — existing resolver tests pass; `test_fetch_video_id_returns_id` shows as `ignored`.

- [ ] **Step 5: Format and commit**

```bash
cargo fmt
git add src/media/resolver.rs
git commit -m "feat: resolver::fetch_video_id via yt-dlp"
```

---

## Task 6: Wire detection into `next_live` + background task + `TuneResponse.ended`

**Files:**
- Modify: `src/routes/player.rs` (`TuneResponse`, `tune_response`, new `tune_response_ended`, `next_live`, new `spawn_live_to_vod_conversion` + `live_to_vod_conversion`)
- Modify: `tests/http.rs` (add `app_with_pool` helper, integration test)

- [ ] **Step 1: Write the failing integration test**

In `tests/http.rs`, first add a pool-returning helper. Replace the existing `app()` function body so it delegates (keeps every existing caller working):

```rust
async fn app_with_pool() -> (axum::Router, sqlx::SqlitePool) {
    let pool = db::connect("sqlite::memory:").await.unwrap();
    sqlx::query(include_str!("fixtures/seed.sql"))
        .execute(&pool)
        .await
        .unwrap();
    let state = AppState {
        pool: pool.clone(),
        config: Arc::new(Config {
            database_url: "sqlite::memory:".to_string(),
            admin_password: "test".to_string(),
            youtube_api_key: None,
            port: 0,
        }),
        http_client: test_client(),
        proxy_client: reqwest::Client::builder()
            .redirect(reqwest::redirect::Policy::none())
            .connect_timeout(std::time::Duration::from_millis(500))
            .timeout(std::time::Duration::from_millis(500))
            .build()
            .unwrap(),
        cors_cache: Arc::new(tokio::sync::RwLock::new(std::collections::HashMap::new())),
        ssrf_cache: Arc::new(tokio::sync::RwLock::new(std::collections::HashMap::new())),
        metrics: Arc::new(metrics::Metrics::new()),
    };
    (build_router(state), pool)
}

async fn app() -> axum::Router {
    app_with_pool().await.0
}
```

Then delete the old standalone `app()` body that built its own pool/state (the new `app()` above replaces it). Add the test:

```rust
#[tokio::test]
async fn test_tune_finished_live_returns_ended_and_no_url() {
    let (router, pool) = app_with_pool().await;
    // A resolved URL containing force_finished/1 marks an ended YouTube live.
    // Seed it as a plain HLS source so resolve_url passes it through unchanged
    // (no yt-dlp needed), exercising the ended-detection wiring deterministically.
    // priority 0 so it is tried before channel 1's existing live source.
    sqlx::query(
        "INSERT INTO sources (channel_id, kind, url, priority, is_active) \
         VALUES (1, 'hls', 'https://stream.example.com/ended.m3u8?force_finished/1', 0, 1)",
    )
    .execute(&pool)
    .await
    .unwrap();

    let response = router.oneshot(req("/channel/1/tune")).await.unwrap();
    assert_eq!(response.status(), StatusCode::OK);
    let json = body_json(response).await;
    assert_eq!(json["ended"], serde_json::json!(true));
    assert_eq!(json["url"], serde_json::json!(""));
}
```

- [ ] **Step 2: Run test to verify it fails**

Run: `cargo test --test http test_tune_finished_live_returns_ended_and_no_url`
Expected: FAIL — `ended` field missing (compile error on `TuneResponse`) or assertion fails.

- [ ] **Step 3: Add the `ended` field and helpers**

In `src/routes/player.rs`, add `ended` to `TuneResponse` (after `skip_proxy`, line 28):

```rust
    pub skip_proxy: bool,
    pub ended: bool,
}
```

Set it `false` in the existing `tune_response` helper (after `skip_proxy,`, around line 86):

```rust
        skip_proxy,
        ended: false,
    })
}
```

Add the ended-response helper right after `tune_response`:

```rust
fn tune_response_ended(ch: &channel::Channel) -> Json<TuneResponse> {
    Json(TuneResponse {
        url: String::new(),
        start_offset_secs: 0,
        name: ch.name.clone(),
        logo_url: ch.logo_url.clone(),
        category: ch.category.clone(),
        channel_type: ch.r#type.clone(),
        skip_proxy: false,
        ended: true,
    })
}
```

- [ ] **Step 4: Add the background task and wire detection into `next_live`**

In `src/routes/player.rs`, add the spawn wrapper + async body after `convert_channel_to_vod_loop`:

```rust
fn spawn_live_to_vod_conversion(
    state: &AppState,
    channel_id: i64,
    channel_name: String,
    source_url: String,
) {
    let pool = state.pool.clone();
    tokio::spawn(async move {
        if let Err(e) = live_to_vod_conversion(&pool, channel_id, &channel_name, &source_url).await
        {
            tracing::warn!(channel_id, error = %e, "ended-live → VOD conversion failed");
        }
    });
}

async fn live_to_vod_conversion(
    pool: &sqlx::SqlitePool,
    channel_id: i64,
    channel_name: &str,
    source_url: &str,
) -> anyhow::Result<()> {
    let watch_url = match resolver::live_url_to_watch_url(source_url) {
        Some(u) => u,
        None => {
            let id = resolver::fetch_video_id(source_url).await?;
            format!("https://www.youtube.com/watch?v={id}")
        }
    };
    let duration = resolver::fetch_duration_secs(&watch_url).await?;
    convert_channel_to_vod_loop(
        pool,
        channel_id,
        channel_name,
        &watch_url,
        duration,
        chrono::Utc::now(),
    )
    .await
}
```

Then change the `Ok(url)` arm of the `resolve_url` match inside `next_live` (currently lines 104-111) to:

```rust
            Ok(url) => {
                if resolver::is_finished_live(&url) {
                    spawn_live_to_vod_conversion(state, ch.id, ch.name.clone(), src.url.clone());
                    return Ok(tune_response_ended(ch));
                }
                return Ok(tune_response(ch, url, 0, resolver::needs_resolution(&src.url)));
            }
```

- [ ] **Step 5: Run the integration test + full suite**

Run: `cargo test --test http test_tune_finished_live_returns_ended_and_no_url`
Expected: PASS.

Run: `cargo test`
Expected: PASS — all existing tests still green (the new `ended` field is additive; `app()` still returns a router).

- [ ] **Step 6: Format and commit**

```bash
cargo fmt
git add src/routes/player.rs tests/http.rs
git commit -m "feat: return ended response and spawn live→VOD conversion on finished live"
```

---

## Task 7: Frontend — auto-advance to next channel with a brief notice

**Files:**
- Modify: `templates/base.html` (CSS rule near line 25, overlay markup near `#player-error`, JS handlers in the script block)

- [ ] **Step 1: Add the overlay markup and style**

Find the `#player-error` style rule (line 25) and add an overlay rule after it:

```css
    #player-error{display:none;padding:32px;text-align:center;color:#e94560;background:#000;font-size:1rem}
    #player-ended{display:none;padding:32px;text-align:center;color:#fff;background:#000;font-size:1rem}
```

Find the `#player-error` element in the markup (search for `id="player-error"`) and add a sibling right after it:

```html
        <div id="player-ended">Stream ended — switching to next channel…</div>
```

- [ ] **Step 2: Add the shared response handler + navigation helpers**

In the `<script>` block, add a module-level counter near the other player state vars (where `currentChannelId` / `currentUrl` are declared):

```js
      var endedHops = 0;
```

Add these functions just above `function tune(channelId)` (line 334):

```js
      function nextChannelId(dir) {
        var channels = window.epgChannels || [];
        if (!channels.length || !currentChannelId) return null;
        var idx = -1;
        for (var i = 0; i < channels.length; i++) {
          if (channels[i].id === currentChannelId) { idx = i; break; }
        }
        if (idx < 0) idx = 0;
        var n = dir === 'up'
          ? (idx - 1 + channels.length) % channels.length
          : (idx + 1) % channels.length;
        return channels[n].id;
      }

      function advanceEndedChannel() {
        var channels = window.epgChannels || [];
        endedHops++;
        if (!channels.length || endedHops > channels.length) {
          endedHops = 0;
          showPlayerError();
          return;
        }
        var notice = document.getElementById('player-ended');
        if (notice) notice.style.display = 'block';
        var nextId = nextChannelId('down');
        setTimeout(function() {
          if (notice) notice.style.display = 'none';
          if (nextId) tune(nextId);
          else showPlayerError();
        }, 1500);
      }

      function applyTuneResponse(d) {
        if (!d) return;
        if (d.ended) { advanceEndedChannel(); return; }
        endedHops = 0;
        if (d.url) _loadSource(d.url, d.start_offset_secs, d.skip_proxy);
      }
```

- [ ] **Step 3: Route every tune/next callback through `applyTuneResponse`**

Replace each response callback body that currently calls `_loadSource` directly. There are six sites:

- Line ~213 (HLS fatal failover): `.then(function(d) { if (d && d.url) _loadSource(d.url, d.start_offset_secs, d.skip_proxy); })` → `.then(function(d) { applyTuneResponse(d); })`
- Line ~245 (DASH failover): same replacement.
- Line ~278 (native HLS onerror failover): same replacement.
- Line ~305 (direct MP4 onerror failover): same replacement.
- Line ~349 (in `tune()`): the `.then(function(d) { currentChannel = ...; renderInfoBar(...); _loadSource(...); })` — keep the info-bar lines but route playback through the handler:

```js
          .then(function(d) {
            currentChannel = Object.assign({ channel_id: channelId }, d);
            renderInfoBar(currentChannel);
            applyTuneResponse(d);
          })
```

- Line ~363 (`video 'ended'` → `/next`): `.then(function(d) { if (d.url) _loadSource(d.url, d.start_offset_secs, d.skip_proxy); })` → `.then(function(d) { applyTuneResponse(d); })`

- [ ] **Step 4: Reuse `nextChannelId` in the arrow-key handler**

Replace the channel-stepping block in the `ArrowUp`/`ArrowDown` handler (lines ~393-406) with:

```js
        if (e.key === 'ArrowUp' || e.key === 'ArrowDown') {
          e.preventDefault();
          var nextId = nextChannelId(e.key === 'ArrowUp' ? 'up' : 'down');
          if (nextId) tune(nextId);
          return;
        }
```

- [ ] **Step 5: Verify the template still compiles**

Run: `cargo build`
Expected: SUCCESS — Askama compiles `base.html` at build time; a template syntax error would fail here.

- [ ] **Step 6: Manual smoke check (optional, documented for the reviewer)**

Run: `cargo run` then open `http://localhost:3000`. The JS change is exercised when a tune response has `ended:true`; without a real ended-live source this is best verified by the integration test from Task 6. Confirm no JS console errors on normal tuning and that ArrowUp/ArrowDown still change channels.

- [ ] **Step 7: Format and commit**

```bash
cargo fmt
git add templates/base.html
git commit -m "feat: auto-advance to next channel with overlay when stream ended"
```

---

## Final verification

- [ ] **Run the full suite, formatter, and linter**

```bash
cargo fmt --check
cargo clippy -- -D warnings
cargo test
```

Expected: formatter clean, no clippy warnings, all tests pass (the existing 226 + the 4 new unit/integration tests; the 1 new `#[ignore]` resolver test is skipped).

- [ ] **Update idea status**

Mark idea 36 done in `docs/IDEAS.md` (strike through the title and append `— done: <one-line summary>. Spec: docs/superpowers/specs/2026-06-09-ended-live-to-vod-design.md`), matching the format of ideas 34 and 37. Commit:

```bash
git add docs/IDEAS.md
git commit -m "docs: mark idea 36 done — auto-convert ended YouTube live to VOD"
```
