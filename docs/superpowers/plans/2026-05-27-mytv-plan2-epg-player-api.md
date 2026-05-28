# MyTV Plan 2: EPG Engine + Player API Implementation Plan

> **For agentic workers:** REQUIRED SUB-SKILL: Use superpowers:subagent-driven-development (recommended) or superpowers:executing-plans to implement this plan task-by-task. Steps use checkbox (`- [ ]`) syntax for tracking.

**Goal:** Build the URL resolver, EPG schedule engine, and player API endpoints (`/channel/:id/tune` and `/channel/:id/next`) that the browser player uses to get a playable URL and advance through VOD loops.

**Architecture:** Three focused modules: `resolver.rs` wraps yt-dlp as a subprocess for URL resolution (HLS URLs pass through unchanged); `epg.rs` contains pure functions that compute program schedules from playlist data; `routes/player.rs` implements the two player endpoints as thin async handlers that call into the resolver and playlist logic. Internal business-logic helpers take an injectable `now_secs: i64` so tests are deterministic without mocking time.

**Tech Stack:** Rust, Axum 0.7, sqlx 0.7 (SQLite), chrono, serde, yt-dlp (system binary, must be installed)

**Prerequisite:** `yt-dlp` must be installed on the host system (`brew install yt-dlp` on macOS). Tests that require yt-dlp and network access are marked `#[ignore]` — all other tests run without it.

---

## File Structure

```
src/
  resolver.rs              — yt-dlp subprocess wrapper + HLS passthrough
  epg.rs                   — ProgramEntry struct + live_entry + vod_schedule
  routes/
    mod.rs                 — (modify) add pub mod player
    player.rs              — TuneResponse, tune handler, next handler, internal helpers
  main.rs                  — (modify) declare epg + resolver modules, add player routes
```

---

## Task 1: URL Resolver

**Files:**
- Create: `src/resolver.rs`
- Modify: `src/main.rs`

- [ ] **Step 1: Write the failing tests in src/resolver.rs**

```rust
use anyhow::{bail, Result};
use std::process::Command;

/// Returns true if the URL requires yt-dlp to obtain a playable stream.
/// Direct HLS and plain IPTV stream URLs are used as-is.
pub fn needs_resolution(url: &str) -> bool {
    url.contains("youtube.com")
        || url.contains("youtu.be")
        || url.contains("twitch.tv")
}

/// Returns a directly playable URL.
/// HLS/IPTV URLs are returned unchanged. YouTube/Twitch are resolved via yt-dlp.
pub fn resolve_url(url: &str) -> Result<String> {
    if !needs_resolution(url) {
        return Ok(url.to_string());
    }
    let output = Command::new("yt-dlp")
        .args(["-g", "--no-playlist", url])
        .output()?;
    if !output.status.success() {
        bail!(
            "yt-dlp failed for {}: {}",
            url,
            String::from_utf8_lossy(&output.stderr).trim()
        );
    }
    let resolved = String::from_utf8(output.stdout)?;
    let first_line = resolved.lines().next().unwrap_or("").trim().to_string();
    if first_line.is_empty() {
        bail!("yt-dlp returned empty output for {}", url);
    }
    Ok(first_line)
}

/// Fetches the duration of a video in seconds via yt-dlp.
/// Called once when an admin adds a VOD asset so duration is stored in the DB.
pub fn fetch_duration_secs(url: &str) -> Result<i64> {
    let output = Command::new("yt-dlp")
        .args(["--print", "duration", "--no-playlist", url])
        .output()?;
    if !output.status.success() {
        bail!(
            "yt-dlp failed for {}: {}",
            url,
            String::from_utf8_lossy(&output.stderr).trim()
        );
    }
    let raw = String::from_utf8(output.stdout)?;
    let trimmed = raw.trim();
    let duration: f64 = trimmed
        .parse()
        .map_err(|_| anyhow::anyhow!("could not parse yt-dlp duration: {:?}", trimmed))?;
    Ok(duration as i64)
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn test_youtube_needs_resolution() {
        assert!(needs_resolution("https://www.youtube.com/watch?v=dQw4w9WgXcQ"));
        assert!(needs_resolution("https://youtu.be/dQw4w9WgXcQ"));
        assert!(needs_resolution("https://www.youtube.com/channel/UCXXXXXX/live"));
        assert!(needs_resolution("https://www.twitch.tv/somestream"));
    }

    #[test]
    fn test_hls_does_not_need_resolution() {
        assert!(!needs_resolution("https://example.com/stream.m3u8"));
        assert!(!needs_resolution("https://live.example.com/hls/index.m3u8"));
        assert!(!needs_resolution("https://iptv.example.com/channel/1"));
        assert!(!needs_resolution("https://vimeo.com/123456789")); // not in list
    }

    #[test]
    fn test_resolve_url_passthrough_for_hls() {
        let url = "https://example.com/live/stream.m3u8";
        let result = resolve_url(url).unwrap();
        assert_eq!(result, url);
    }

    #[test]
    fn test_resolve_url_passthrough_for_plain_iptv() {
        let url = "https://iptv.example.com/channel/999/index";
        let result = resolve_url(url).unwrap();
        assert_eq!(result, url);
    }

    #[test]
    #[ignore = "requires yt-dlp installed and network access — run manually"]
    fn test_resolve_youtube_live_returns_hls_url() {
        let url = "https://www.youtube.com/watch?v=jfKfPfyJRdk"; // lofi beats, usually live
        let result = resolve_url(url);
        assert!(result.is_ok(), "expected resolved URL, got: {:?}", result);
        let resolved = result.unwrap();
        assert!(
            resolved.starts_with("https://"),
            "expected HTTPS URL, got: {}",
            resolved
        );
    }

    #[test]
    #[ignore = "requires yt-dlp installed and network access — run manually"]
    fn test_fetch_duration_returns_seconds() {
        // A short public YouTube video
        let url = "https://www.youtube.com/watch?v=dQw4w9WgXcQ";
        let result = fetch_duration_secs(url);
        assert!(result.is_ok(), "expected duration, got: {:?}", result);
        let secs = result.unwrap();
        assert!(secs > 0, "duration should be positive");
    }
}
```

- [ ] **Step 2: Run tests to confirm they fail with a compile error**

```bash
cargo test resolver 2>&1 | head -5
```

Expected: `error[E0433]: failed to resolve: use of undeclared crate or module 'resolver'`

- [ ] **Step 3: Declare the module in src/main.rs**

Add `mod resolver;` to the module declarations in src/main.rs (keep alphabetical order):

```rust
mod channel;
mod config;
mod db;
mod playlist_item;
mod resolver;
mod routes;
mod source;
```

The rest of main.rs stays unchanged.

- [ ] **Step 4: Run tests to confirm they pass**

```bash
cargo test resolver 2>&1
```

Expected:
```
test resolver::tests::test_hls_does_not_need_resolution ... ok
test resolver::tests::test_resolve_url_passthrough_for_hls ... ok
test resolver::tests::test_resolve_url_passthrough_for_plain_iptv ... ok
test resolver::tests::test_youtube_needs_resolution ... ok
```

(The two `#[ignore]` tests are skipped — that is correct.)

- [ ] **Step 5: Commit**

```bash
git add src/resolver.rs src/main.rs
git commit -m "feat: add url resolver with yt-dlp and hls passthrough"
```

---

## Task 2: EPG Schedule Engine

**Files:**
- Create: `src/epg.rs`
- Modify: `src/main.rs`

- [ ] **Step 1: Write the failing tests in src/epg.rs**

```rust
use chrono::{DateTime, Duration, Utc};
use serde::Serialize;

use crate::playlist_item::{self, PlaylistItem};

#[derive(Debug, Clone, Serialize)]
pub struct ProgramEntry {
    pub channel_id: i64,
    pub title: String,
    pub url: String,
    pub start_time: DateTime<Utc>,
    pub end_time: DateTime<Utc>,
    pub is_live: bool,
    pub start_offset_secs: i64,
}

/// Returns the single EPG entry for a live channel spanning the given window.
pub fn live_entry(
    channel_id: i64,
    name: &str,
    window_start: DateTime<Utc>,
    window_end: DateTime<Utc>,
) -> ProgramEntry {
    ProgramEntry {
        channel_id,
        title: format!("{} — Live", name),
        url: String::new(),
        start_time: window_start,
        end_time: window_end,
        is_live: true,
        start_offset_secs: 0,
    }
}

/// Computes program entries for a VOD loop channel over the given time window.
/// Entries are in chronological order. The first entry may start with a non-zero
/// offset if the window starts mid-item. The last entry is clipped to window_end.
pub fn vod_schedule(
    channel_id: i64,
    items: &[PlaylistItem],
    anchor_secs: i64,
    window_start: DateTime<Utc>,
    window_end: DateTime<Utc>,
) -> Vec<ProgramEntry> {
    if items.is_empty() || window_start >= window_end {
        return vec![];
    }

    let (mut idx, mut offset) =
        match playlist_item::current_position(items, window_start.timestamp(), anchor_secs) {
            Some(pos) => pos,
            None => return vec![],
        };

    let mut entries = Vec::new();
    let mut cursor = window_start;

    while cursor < window_end {
        let item = &items[idx];
        let remaining_secs = item.duration_secs - offset;
        let entry_end = (cursor + Duration::seconds(remaining_secs)).min(window_end);

        entries.push(ProgramEntry {
            channel_id,
            title: item.title.clone(),
            url: item.url.clone(),
            start_time: cursor,
            end_time: entry_end,
            is_live: false,
            start_offset_secs: offset,
        });

        cursor = entry_end;
        offset = 0;
        idx = (idx + 1) % items.len();
    }

    entries
}

#[cfg(test)]
mod tests {
    use super::*;
    use chrono::TimeZone;

    fn item(id: i64, title: &str, duration_secs: i64) -> PlaylistItem {
        PlaylistItem {
            id,
            channel_id: 1,
            title: title.to_string(),
            url: format!("https://example.com/{}.mp4", title),
            duration_secs,
            sort_order: id - 1,
        }
    }

    #[test]
    fn test_live_entry_spans_full_window() {
        let start = Utc.with_ymd_and_hms(2026, 1, 1, 0, 0, 0).unwrap();
        let end = start + Duration::hours(24);
        let entry = live_entry(1, "CNN", start, end);
        assert!(entry.is_live);
        assert_eq!(entry.start_time, start);
        assert_eq!(entry.end_time, end);
        assert_eq!(entry.channel_id, 1);
        assert!(entry.title.contains("CNN"));
    }

    #[test]
    fn test_vod_schedule_empty_playlist_returns_empty() {
        let start = Utc.with_ymd_and_hms(2026, 1, 1, 0, 0, 0).unwrap();
        let end = start + Duration::hours(1);
        let entries = vod_schedule(1, &[], 0, start, end);
        assert!(entries.is_empty());
    }

    #[test]
    fn test_vod_schedule_window_start_equals_end_returns_empty() {
        let t = Utc.with_ymd_and_hms(2026, 1, 1, 0, 0, 0).unwrap();
        let entries = vod_schedule(1, &[item(1, "A", 3600)], 0, t, t);
        assert!(entries.is_empty());
    }

    #[test]
    fn test_vod_schedule_two_full_loops() {
        // Playlist: A (3600s) + B (1800s) = 5400s total. Window = 3h = two full loops.
        let items = vec![item(1, "A", 3600), item(2, "B", 1800)];
        let anchor_secs = 0;
        let start = Utc.with_ymd_and_hms(2026, 1, 1, 0, 0, 0).unwrap(); // exactly at anchor
        let end = start + Duration::hours(3);

        let entries = vod_schedule(1, &items, anchor_secs, start, end);

        assert_eq!(entries.len(), 4); // A, B, A, B
        assert_eq!(entries[0].title, "A");
        assert_eq!(entries[0].start_offset_secs, 0);
        assert_eq!(entries[1].title, "B");
        assert_eq!(entries[1].start_offset_secs, 0);
        assert_eq!(entries[2].title, "A");
        assert_eq!(entries[2].start_offset_secs, 0);
        assert_eq!(entries[3].title, "B");
        assert_eq!(entries[3].start_offset_secs, 0);
    }

    #[test]
    fn test_vod_schedule_starts_mid_first_item() {
        // Playlist: A (3600s). Window starts 1800s into A. Window = 3600s.
        // Expected: first entry is tail of A (1800s remaining, offset 1800),
        //           second entry is head of next-loop A (1800s, offset 0).
        let items = vec![item(1, "A", 3600)];
        let anchor_secs = 0;
        let start = Utc.timestamp_opt(1800, 0).unwrap();
        let end = start + Duration::seconds(3600);

        let entries = vod_schedule(1, &items, anchor_secs, start, end);

        assert_eq!(entries.len(), 2);
        assert_eq!(entries[0].start_offset_secs, 1800);
        assert_eq!(entries[0].end_time, start + Duration::seconds(1800));
        assert_eq!(entries[1].start_offset_secs, 0);
        assert_eq!(entries[1].end_time, end);
    }

    #[test]
    fn test_vod_schedule_last_entry_clipped_to_window_end() {
        // Playlist: A (3600s). Window = 30 min. Should produce one entry ending at window_end.
        let items = vec![item(1, "A", 3600)];
        let anchor_secs = 0;
        let start = Utc.timestamp_opt(0, 0).unwrap();
        let end = start + Duration::minutes(30);

        let entries = vod_schedule(1, &items, anchor_secs, start, end);

        assert_eq!(entries.len(), 1);
        assert_eq!(entries[0].title, "A");
        assert_eq!(entries[0].start_offset_secs, 0);
        assert_eq!(entries[0].end_time, end); // clipped, not 3600s
    }

    #[test]
    fn test_vod_schedule_mid_second_item() {
        // Playlist: A (3600s) + B (1800s). Window starts 4000s into anchor (400s into B).
        let items = vec![item(1, "A", 3600), item(2, "B", 1800)];
        let anchor_secs = 0;
        let start = Utc.timestamp_opt(4000, 0).unwrap(); // 4000s = 3600 (A) + 400s into B
        let end = start + Duration::minutes(30);

        let entries = vod_schedule(1, &items, anchor_secs, start, end);

        assert_eq!(entries[0].title, "B");
        assert_eq!(entries[0].start_offset_secs, 400); // 400s into B
    }
}
```

- [ ] **Step 2: Run tests to confirm they fail with a compile error**

```bash
cargo test epg 2>&1 | head -5
```

Expected: `error[E0433]: failed to resolve: use of undeclared crate or module 'epg'`

- [ ] **Step 3: Declare the module in src/main.rs**

Add `mod epg;` (keep alphabetical order):

```rust
mod channel;
mod config;
mod db;
mod epg;
mod playlist_item;
mod resolver;
mod routes;
mod source;
```

- [ ] **Step 4: Run tests to confirm they pass**

```bash
cargo test epg 2>&1
```

Expected: all 6 epg tests pass

- [ ] **Step 5: Commit**

```bash
git add src/epg.rs src/main.rs
git commit -m "feat: add epg schedule engine for live and vod loop channels"
```

---

## Task 3: Player Routes

**Files:**
- Create: `src/routes/player.rs`
- Modify: `src/routes/mod.rs`
- Modify: `src/main.rs`

### About the internal helper pattern

The handlers delegate to private async helpers that accept `now_secs: i64` as a parameter instead of calling `Utc::now()` directly. This makes them deterministically testable within `#[cfg(test)]` without any mocking framework.

- [ ] **Step 1: Write the failing tests in src/routes/player.rs**

```rust
use axum::{
    extract::{Path, Query, State},
    http::StatusCode,
    response::Json,
};
use chrono::DateTime;
use serde::{Deserialize, Serialize};

use crate::{
    channel::{self, ChannelType},
    playlist_item, resolver, source, AppState,
};

#[derive(Debug, Serialize)]
pub struct TuneResponse {
    pub url: String,
    pub start_offset_secs: i64,
}

#[derive(Debug, Deserialize)]
pub struct NextQuery {
    pub failed_url: Option<String>,
}

pub async fn tune(
    State(state): State<AppState>,
    Path(channel_id): Path<i64>,
) -> Result<Json<TuneResponse>, StatusCode> {
    let ch = channel::get(&state.pool, channel_id)
        .await
        .map_err(|_| StatusCode::INTERNAL_SERVER_ERROR)?
        .ok_or(StatusCode::NOT_FOUND)?;

    match ch.channel_type() {
        ChannelType::Live => tune_live(&state, &ch).await,
        ChannelType::VodLoop => {
            let now_secs = chrono::Utc::now().timestamp();
            tune_vod_at(&state, &ch, now_secs).await
        }
    }
}

pub async fn next(
    State(state): State<AppState>,
    Path(channel_id): Path<i64>,
    Query(q): Query<NextQuery>,
) -> Result<Json<TuneResponse>, StatusCode> {
    let ch = channel::get(&state.pool, channel_id)
        .await
        .map_err(|_| StatusCode::INTERNAL_SERVER_ERROR)?
        .ok_or(StatusCode::NOT_FOUND)?;

    match ch.channel_type() {
        ChannelType::Live => next_live(&state, &ch, q.failed_url.as_deref()).await,
        ChannelType::VodLoop => {
            let now_secs = chrono::Utc::now().timestamp();
            next_vod_at(&state, &ch, now_secs).await
        }
    }
}

async fn tune_live(
    state: &AppState,
    ch: &channel::Channel,
) -> Result<Json<TuneResponse>, StatusCode> {
    let sources = source::list_active_for_channel(&state.pool, ch.id)
        .await
        .map_err(|_| StatusCode::INTERNAL_SERVER_ERROR)?;

    for src in &sources {
        if let Ok(url) = resolver::resolve_url(&src.url) {
            return Ok(Json(TuneResponse { url, start_offset_secs: 0 }));
        }
    }
    Err(StatusCode::SERVICE_UNAVAILABLE)
}

async fn tune_vod_at(
    state: &AppState,
    ch: &channel::Channel,
    now_secs: i64,
) -> Result<Json<TuneResponse>, StatusCode> {
    let anchor_secs = ch
        .loop_anchor
        .ok_or(StatusCode::SERVICE_UNAVAILABLE)?
        .timestamp();

    let items = playlist_item::list_for_channel(&state.pool, ch.id)
        .await
        .map_err(|_| StatusCode::INTERNAL_SERVER_ERROR)?;

    if items.is_empty() {
        return Err(StatusCode::SERVICE_UNAVAILABLE);
    }

    let (idx, offset) =
        playlist_item::current_position(&items, now_secs, anchor_secs)
            .ok_or(StatusCode::SERVICE_UNAVAILABLE)?;

    let item = &items[idx];
    match resolver::resolve_url(&item.url) {
        Ok(url) => Ok(Json(TuneResponse { url, start_offset_secs: offset })),
        Err(_) => Err(StatusCode::SERVICE_UNAVAILABLE),
    }
}

async fn next_live(
    state: &AppState,
    ch: &channel::Channel,
    failed_url: Option<&str>,
) -> Result<Json<TuneResponse>, StatusCode> {
    let sources = source::list_active_for_channel(&state.pool, ch.id)
        .await
        .map_err(|_| StatusCode::INTERNAL_SERVER_ERROR)?;

    for src in sources.iter().filter(|s| Some(s.url.as_str()) != failed_url) {
        if let Ok(url) = resolver::resolve_url(&src.url) {
            return Ok(Json(TuneResponse { url, start_offset_secs: 0 }));
        }
    }
    Err(StatusCode::SERVICE_UNAVAILABLE)
}

async fn next_vod_at(
    state: &AppState,
    ch: &channel::Channel,
    now_secs: i64,
) -> Result<Json<TuneResponse>, StatusCode> {
    let anchor_secs = ch
        .loop_anchor
        .ok_or(StatusCode::SERVICE_UNAVAILABLE)?
        .timestamp();

    let items = playlist_item::list_for_channel(&state.pool, ch.id)
        .await
        .map_err(|_| StatusCode::INTERNAL_SERVER_ERROR)?;

    if items.is_empty() {
        return Err(StatusCode::SERVICE_UNAVAILABLE);
    }

    let (idx, _) = playlist_item::current_position(&items, now_secs, anchor_secs)
        .ok_or(StatusCode::SERVICE_UNAVAILABLE)?;

    let next_idx = (idx + 1) % items.len();
    let item = &items[next_idx];

    match resolver::resolve_url(&item.url) {
        Ok(url) => Ok(Json(TuneResponse { url, start_offset_secs: 0 })),
        Err(_) => Err(StatusCode::SERVICE_UNAVAILABLE),
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::{config, db};

    async fn test_state() -> AppState {
        let pool = db::connect("sqlite::memory:").await.unwrap();
        let config = std::sync::Arc::new(config::Config::from_env().unwrap());
        AppState { pool, config }
    }

    async fn make_live_channel(state: &AppState) -> channel::Channel {
        channel::create(
            &state.pool,
            channel::NewChannel {
                name: "Live Test".into(),
                category: "test".into(),
                logo_url: None,
                channel_type: "live".into(),
                sort_order: 0,
                loop_anchor: None,
            },
        )
        .await
        .unwrap()
    }

    /// anchor_secs: unix timestamp the VOD loop started
    async fn make_vod_channel(state: &AppState, anchor_secs: i64) -> channel::Channel {
        channel::create(
            &state.pool,
            channel::NewChannel {
                name: "VOD Test".into(),
                category: "test".into(),
                logo_url: None,
                channel_type: "vod_loop".into(),
                sort_order: 0,
                loop_anchor: Some(DateTime::from_timestamp(anchor_secs, 0).unwrap()),
            },
        )
        .await
        .unwrap()
    }

    // ── tune_live ────────────────────────────────────────────────────────────

    #[tokio::test]
    async fn test_tune_live_returns_primary_hls_source() {
        let state = test_state().await;
        let ch = make_live_channel(&state).await;

        source::create(
            &state.pool,
            source::NewSource {
                channel_id: ch.id,
                kind: "hls".into(),
                url: "https://primary.example.com/stream.m3u8".into(),
                priority: 1,
            },
        )
        .await
        .unwrap();

        let result = tune_live(&state, &ch).await.unwrap();
        assert_eq!(result.url, "https://primary.example.com/stream.m3u8");
        assert_eq!(result.start_offset_secs, 0);
    }

    #[tokio::test]
    async fn test_tune_live_skips_youtube_source_when_ytdlp_unavailable_and_returns_hls_backup() {
        let state = test_state().await;
        let ch = make_live_channel(&state).await;

        // Priority 1: YouTube (resolve_url will fail without yt-dlp or network)
        source::create(
            &state.pool,
            source::NewSource {
                channel_id: ch.id,
                kind: "youtube_live".into(),
                url: "https://www.youtube.com/watch?v=FAIL_YTDLP_NOT_INSTALLED".into(),
                priority: 1,
            },
        )
        .await
        .unwrap();

        // Priority 2: HLS passthrough (always succeeds)
        source::create(
            &state.pool,
            source::NewSource {
                channel_id: ch.id,
                kind: "hls".into(),
                url: "https://backup.example.com/stream.m3u8".into(),
                priority: 2,
            },
        )
        .await
        .unwrap();

        let result = tune_live(&state, &ch).await.unwrap();
        assert_eq!(result.url, "https://backup.example.com/stream.m3u8");
    }

    #[tokio::test]
    async fn test_tune_live_returns_503_when_all_sources_fail() {
        let state = test_state().await;
        let ch = make_live_channel(&state).await;
        // No sources added

        let err = tune_live(&state, &ch).await.unwrap_err();
        assert_eq!(err, StatusCode::SERVICE_UNAVAILABLE);
    }

    // ── tune_vod_at ─────────────────────────────────────────────────────────

    #[tokio::test]
    async fn test_tune_vod_returns_correct_url_and_offset() {
        // anchor=0, now=1000, item A=3600s → idx=0, offset=1000
        let state = test_state().await;
        let ch = make_vod_channel(&state, 0).await;

        playlist_item::create(
            &state.pool,
            playlist_item::NewPlaylistItem {
                channel_id: ch.id,
                title: "Episode 1".into(),
                url: "https://example.com/ep1.m3u8".into(),
                duration_secs: 3600,
                sort_order: 0,
            },
        )
        .await
        .unwrap();

        let result = tune_vod_at(&state, &ch, 1000).await.unwrap();
        assert_eq!(result.url, "https://example.com/ep1.m3u8");
        assert_eq!(result.start_offset_secs, 1000);
    }

    #[tokio::test]
    async fn test_tune_vod_wraps_to_second_item() {
        // anchor=0, now=4000, A=3600s, B=1800s → idx=1 (in B), offset=400
        let state = test_state().await;
        let ch = make_vod_channel(&state, 0).await;

        playlist_item::create(
            &state.pool,
            playlist_item::NewPlaylistItem {
                channel_id: ch.id,
                title: "A".into(),
                url: "https://example.com/a.m3u8".into(),
                duration_secs: 3600,
                sort_order: 0,
            },
        )
        .await
        .unwrap();

        playlist_item::create(
            &state.pool,
            playlist_item::NewPlaylistItem {
                channel_id: ch.id,
                title: "B".into(),
                url: "https://example.com/b.m3u8".into(),
                duration_secs: 1800,
                sort_order: 1,
            },
        )
        .await
        .unwrap();

        let result = tune_vod_at(&state, &ch, 4000).await.unwrap();
        assert_eq!(result.url, "https://example.com/b.m3u8");
        assert_eq!(result.start_offset_secs, 400); // 4000 - 3600 = 400s into B
    }

    #[tokio::test]
    async fn test_tune_vod_returns_503_when_no_playlist() {
        let state = test_state().await;
        let ch = make_vod_channel(&state, 0).await;
        // No playlist items

        let err = tune_vod_at(&state, &ch, 1000).await.unwrap_err();
        assert_eq!(err, StatusCode::SERVICE_UNAVAILABLE);
    }

    // ── next_live ────────────────────────────────────────────────────────────

    #[tokio::test]
    async fn test_next_live_skips_failed_url_and_returns_backup() {
        let state = test_state().await;
        let ch = make_live_channel(&state).await;

        source::create(
            &state.pool,
            source::NewSource {
                channel_id: ch.id,
                kind: "hls".into(),
                url: "https://primary.example.com/stream.m3u8".into(),
                priority: 1,
            },
        )
        .await
        .unwrap();

        source::create(
            &state.pool,
            source::NewSource {
                channel_id: ch.id,
                kind: "hls".into(),
                url: "https://backup.example.com/stream.m3u8".into(),
                priority: 2,
            },
        )
        .await
        .unwrap();

        let result = next_live(
            &state,
            &ch,
            Some("https://primary.example.com/stream.m3u8"),
        )
        .await
        .unwrap();
        assert_eq!(result.url, "https://backup.example.com/stream.m3u8");
    }

    #[tokio::test]
    async fn test_next_live_returns_primary_when_no_failed_url() {
        let state = test_state().await;
        let ch = make_live_channel(&state).await;

        source::create(
            &state.pool,
            source::NewSource {
                channel_id: ch.id,
                kind: "hls".into(),
                url: "https://primary.example.com/stream.m3u8".into(),
                priority: 1,
            },
        )
        .await
        .unwrap();

        let result = next_live(&state, &ch, None).await.unwrap();
        assert_eq!(result.url, "https://primary.example.com/stream.m3u8");
    }

    // ── next_vod_at ──────────────────────────────────────────────────────────

    #[tokio::test]
    async fn test_next_vod_returns_following_item() {
        // anchor=0, now=4000, A=3600s, B=1800s → currently in B (idx=1) → next = A (idx=0)
        let state = test_state().await;
        let ch = make_vod_channel(&state, 0).await;

        playlist_item::create(
            &state.pool,
            playlist_item::NewPlaylistItem {
                channel_id: ch.id,
                title: "A".into(),
                url: "https://example.com/a.m3u8".into(),
                duration_secs: 3600,
                sort_order: 0,
            },
        )
        .await
        .unwrap();

        playlist_item::create(
            &state.pool,
            playlist_item::NewPlaylistItem {
                channel_id: ch.id,
                title: "B".into(),
                url: "https://example.com/b.m3u8".into(),
                duration_secs: 1800,
                sort_order: 1,
            },
        )
        .await
        .unwrap();

        // now=4000: elapsed=4000 % 5400=4000, in B (after 3600s of A)
        // next_idx = (1+1) % 2 = 0 → A
        let result = next_vod_at(&state, &ch, 4000).await.unwrap();
        assert_eq!(result.url, "https://example.com/a.m3u8");
        assert_eq!(result.start_offset_secs, 0);
    }

    #[tokio::test]
    async fn test_next_vod_wraps_around_at_end_of_playlist() {
        // anchor=0, now=500, A=3600s → currently in A (idx=0) → next = A (idx=0, wraps)
        let state = test_state().await;
        let ch = make_vod_channel(&state, 0).await;

        playlist_item::create(
            &state.pool,
            playlist_item::NewPlaylistItem {
                channel_id: ch.id,
                title: "A".into(),
                url: "https://example.com/a.m3u8".into(),
                duration_secs: 3600,
                sort_order: 0,
            },
        )
        .await
        .unwrap();

        // Single-item playlist: next always wraps back to A
        let result = next_vod_at(&state, &ch, 500).await.unwrap();
        assert_eq!(result.url, "https://example.com/a.m3u8");
        assert_eq!(result.start_offset_secs, 0);
    }
}
```

- [ ] **Step 2: Run tests to confirm they fail with a compile error**

```bash
cargo test player 2>&1 | head -5
```

Expected: `error[E0433]: failed to resolve: use of undeclared crate or module 'player'`

- [ ] **Step 3: Add pub mod player to src/routes/mod.rs**

Replace `src/routes/mod.rs` with:

```rust
pub mod health;
pub mod player;
```

- [ ] **Step 4: Run tests again — expect a different error (file not found)**

```bash
cargo test player 2>&1 | head -5
```

Expected: `error[E0583]: file not found for module 'player'` (the module is declared but the file doesn't exist yet)

- [ ] **Step 5: Create src/routes/player.rs with the content from Step 1**

The full file content is identical to what was written in Step 1.

- [ ] **Step 6: Run tests to confirm they pass**

```bash
cargo test player -- --test-threads=1 2>&1
```

Expected: all 11 player tests pass

- [ ] **Step 7: Wire player routes into src/main.rs**

Replace the `let app = Router::new()...` block in `src/main.rs` with:

```rust
    let app = Router::new()
        .route("/health", get(routes::health::health_check))
        .route("/channel/:id/tune", get(routes::player::tune))
        .route("/channel/:id/next", get(routes::player::next))
        .with_state(state);
```

The full updated `src/main.rs`:

```rust
mod channel;
mod config;
mod db;
mod epg;
mod playlist_item;
mod resolver;
mod routes;
mod source;

use anyhow::Result;
use axum::{routing::get, Router};
use sqlx::SqlitePool;
use std::sync::Arc;
use tracing_subscriber::EnvFilter;

#[derive(Clone)]
pub struct AppState {
    pub pool: SqlitePool,
    pub config: Arc<config::Config>,
}

#[tokio::main]
async fn main() -> Result<()> {
    dotenvy::dotenv().ok();
    tracing_subscriber::fmt()
        .with_env_filter(EnvFilter::from_default_env())
        .init();

    let config = Arc::new(config::Config::from_env()?);
    let pool = db::connect(&config.database_url).await?;

    let state = AppState {
        pool,
        config: config.clone(),
    };

    let app = Router::new()
        .route("/health", get(routes::health::health_check))
        .route("/channel/:id/tune", get(routes::player::tune))
        .route("/channel/:id/next", get(routes::player::next))
        .with_state(state);

    let addr = format!("0.0.0.0:{}", config.port);
    tracing::info!("listening on {addr}");
    let listener = tokio::net::TcpListener::bind(&addr).await?;
    axum::serve(listener, app).await?;

    Ok(())
}
```

- [ ] **Step 8: Build to confirm no compile errors**

```bash
cargo build 2>&1 | grep -E "^error"
```

Expected: no output (no errors)

- [ ] **Step 9: Run the full test suite**

```bash
cargo test -- --test-threads=1 2>&1 | tail -5
```

Expected: all tests pass (channel + config + db + epg + playlist_item + player + resolver + source)

- [ ] **Step 10: Smoke test the endpoints with a running server**

In one terminal:
```bash
cargo run
```

In a second terminal (after server starts):
```bash
# Channel 999 doesn't exist — expect 404
curl -s -o /dev/null -w "%{http_code}" http://localhost:3000/channel/999/tune
```
Expected: `404`

```bash
# Health still works
curl -s http://localhost:3000/health
```
Expected: `{"status":"ok"}`

- [ ] **Step 11: Commit**

```bash
git add src/routes/player.rs src/routes/mod.rs src/main.rs
git commit -m "feat: add player api with tune and next endpoints"
```

---

## Self-Review

**Spec coverage:**

| Spec requirement | Covered |
|---|---|
| yt-dlp URL resolution for YouTube | ✅ Task 1 `resolve_url` |
| HLS passthrough without yt-dlp | ✅ Task 1 `needs_resolution` + `resolve_url` |
| Duration fetching for VOD assets | ✅ Task 1 `fetch_duration_secs` |
| EPG for live channels (single "Live" block) | ✅ Task 2 `live_entry` |
| EPG for VOD loop channels (24h computed schedule) | ✅ Task 2 `vod_schedule` |
| `/channel/:id/tune` endpoint | ✅ Task 3 `tune` handler |
| `/channel/:id/next` endpoint | ✅ Task 3 `next` handler |
| Live channel source failover by priority | ✅ Task 3 `tune_live` + `next_live` |
| VOD loop position computed from anchor | ✅ Task 3 `tune_vod_at` + `next_vod_at` |
| Start offset returned for mid-item VOD | ✅ Task 3 `tune_vod_at` |
| EPG grid UI (Askama templates, HTMX) | ⬜ Plan 3 |
| Admin CRUD UI | ⬜ Plan 4 |
| Discovery tools | ⬜ Plan 4 |

**Placeholder scan:** No TBDs. Every step has complete code. All test assertions are specific.

**Type consistency:**
- `ProgramEntry.start_offset_secs: i64` ↔ `TuneResponse.start_offset_secs: i64` — same type, same name
- `playlist_item::current_position` returns `Option<(usize, i64)>` — used correctly in `tune_vod_at` and `next_vod_at`
- `channel::ChannelType::Live` and `ChannelType::VodLoop` — match arms are exhaustive
- `AppState` in `routes/player.rs` accessed as `crate::AppState` via `use crate::AppState` — correct for binary crate where `AppState` is defined in `main.rs`

---

## Next Plans

- **Plan 3:** EPG Grid UI + Player — Askama HTML templates, HTMX category tab switching and 24h time navigation, hls.js player panel, mobile-responsive layout
- **Plan 4:** Admin UI + Discovery — Admin CRUD pages for channels/sources/playlists, YouTube Data API search, iptv-org M3U import, manual URL entry
