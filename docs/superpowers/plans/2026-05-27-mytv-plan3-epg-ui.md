# MyTV Plan 3: EPG Grid UI + Player Implementation Plan

> **For agentic workers:** REQUIRED SUB-SKILL: Use superpowers:subagent-driven-development (recommended) or superpowers:executing-plans to implement this plan task-by-task. Steps use checkbox (`- [ ]`) syntax for tracking.

**Goal:** Build the EPG grid view at `/guide` with category tabs, time navigation (4-hour sliding window), and an integrated hls.js player panel that tunes to any channel via the existing `/channel/:id/tune` and `/channel/:id/next` API.

**Architecture:** Three Askama templates (base shell, full guide page, HTMX-swappable EPG content partial). Pure helper functions compute window positions and convert `ProgramEntry` to percentage-positioned `ProgramSlot` values. Two route handlers — `/guide` (full page) and `/guide/partial` (HTMX fragment) — share a `build_guide_data` helper. The player is a `<video>` element controlled by ~30 lines of JavaScript that call the existing tune/next API.

**Tech Stack:** Rust, Axum 0.7, Askama 0.12 (server-rendered templates), HTMX 1.9 (CDN, tab and time nav swaps), hls.js (CDN, HLS playback), axum::response::Html

---

## File Structure

```
templates/
  base.html                     — HTML shell: CSS, CDN scripts (HTMX + hls.js), player JS
  guide.html                    — full guide page (extends base.html, includes epg_content)
  partials/
    epg_content.html            — HTMX swap target: tabs + time nav + EPG grid rows
src/
  routes/
    guide.rs                    — ProgramSlot/ChannelRow/TimeLabel types, pure helpers,
                                   build_rows, guide_page + guide_partial handlers
    mod.rs                      — (modify) add pub mod guide
  main.rs                       — (modify) add GET /guide and GET /guide/partial routes
Cargo.toml                      — (modify) add askama = "0.12"
```

---

## Task 1: Askama dependency + template scaffold

**Files:**
- Modify: `Cargo.toml`
- Create: `templates/base.html`
- Create: `templates/guide.html` (scaffold only — full content added in Task 3)
- Create: `templates/partials/epg_content.html` (scaffold only — full content added in Task 3)

- [ ] **Step 1: Add askama to Cargo.toml**

Replace `[dependencies]` section in `Cargo.toml` with:

```toml
[dependencies]
axum = "0.7"
tokio = { version = "1", features = ["full"] }
sqlx = { version = "0.7", features = ["runtime-tokio-rustls", "sqlite", "migrate", "chrono"] }
tower-http = { version = "0.5", features = ["trace"] }
serde = { version = "1", features = ["derive"] }
serde_json = "1"
chrono = { version = "0.4", features = ["serde"] }
dotenvy = "0.15"
tracing = "0.1"
tracing-subscriber = { version = "0.3", features = ["env-filter"] }
anyhow = "1"
askama = "0.12"
```

- [ ] **Step 2: Create the templates directory**

```bash
mkdir -p /Users/kunwu/Workspace/playground/MyTV/templates/partials
```

- [ ] **Step 3: Create templates/base.html**

Create `/Users/kunwu/Workspace/playground/MyTV/templates/base.html`:

```html
<!DOCTYPE html>
<html lang="en">
<head>
  <meta charset="utf-8">
  <meta name="viewport" content="width=device-width, initial-scale=1">
  <title>MyTV</title>
  <script src="https://unpkg.com/htmx.org@1.9.10" defer></script>
  <script src="https://cdn.jsdelivr.net/npm/hls.js@latest"></script>
  <style>
    *{box-sizing:border-box;margin:0;padding:0}
    body{background:#0f0f0f;color:#e0e0e0;font-family:system-ui,sans-serif;min-height:100vh}
    a{color:inherit;text-decoration:none}

    /* ── header ───────────────────────────────────────────────── */
    .site-header{background:#111;padding:10px 16px;display:flex;align-items:center;gap:12px;border-bottom:1px solid #222}
    .site-header h1{font-size:1.1rem;color:#e94560;letter-spacing:1px}

    /* ── player panel ─────────────────────────────────────────── */
    #player-panel{display:none;background:#000;border-bottom:2px solid #e94560}
    #player-panel video{width:100%;max-height:50vh;display:block}

    /* ── epg nav (tabs + time nav) ────────────────────────────── */
    .epg-nav{display:flex;align-items:center;justify-content:space-between;
             flex-wrap:wrap;gap:8px;padding:8px 12px;background:#141420;border-bottom:1px solid #222}
    .tabs{display:flex;gap:6px;flex-wrap:wrap}
    .tab{padding:3px 12px;border-radius:3px;cursor:pointer;border:1px solid #333;
         background:#1a1a1a;color:#999;font-size:0.82rem}
    .tab.active,.tab:hover{background:#e94560;color:#fff;border-color:#e94560}
    .time-nav{display:flex;align-items:center;gap:10px}
    .nav-btn{padding:3px 12px;background:#222;color:#bbb;border:1px solid #333;
             border-radius:3px;cursor:pointer;font-size:0.82rem}
    .nav-btn:hover{background:#333}
    .time-range{font-size:0.8rem;color:#777;min-width:120px;text-align:center}

    /* ── epg grid ─────────────────────────────────────────────── */
    .epg-wrapper{overflow-x:auto}
    .epg-grid{min-width:700px}
    .epg-row{display:flex;border-bottom:1px solid #1c1c1c;min-height:44px}
    .channel-col{width:140px;flex-shrink:0;padding:0 10px;display:flex;align-items:center;
                 font-size:0.8rem;border-right:1px solid #222;background:#111;overflow:hidden}
    .programs-col{flex:1;position:relative;height:44px;overflow:hidden}

    /* time header row */
    .time-header .programs-col{height:26px;background:#0d0d1a;border-bottom:1px solid #1e2030}
    .time-label{position:absolute;transform:translateX(-50%);font-size:0.7rem;color:#555;top:5px;white-space:nowrap}

    /* program blocks */
    .program{position:absolute;top:1px;height:42px;overflow:hidden;cursor:pointer;
             display:flex;align-items:center;padding:0 7px;font-size:0.78rem;
             background:#1a2235;border-right:1px solid #0f0f0f;
             transition:background 0.1s}
    .program:hover{background:#22304a;z-index:2}
    .program.live{background:#1a2a1a}
    .program.live:hover{background:#203220}
    .live-badge{background:#c0001a;color:#fff;font-size:0.6rem;font-weight:700;
                padding:1px 4px;border-radius:2px;margin-right:5px;flex-shrink:0;letter-spacing:0.5px}
    .program-title{white-space:nowrap;overflow:hidden;text-overflow:ellipsis}

    /* "now" line */
    .now-line{position:absolute;top:0;bottom:0;width:2px;background:#e94560;
              z-index:5;pointer-events:none}

    /* ── mobile ───────────────────────────────────────────────── */
    @media(max-width:600px){
      .channel-col{width:80px;font-size:0.7rem;padding:0 5px}
      .epg-grid{min-width:400px}
      .time-range{display:none}
    }
  </style>
</head>
<body>
  <header class="site-header">
    <h1>MyTV</h1>
  </header>

  {% block content %}{% endblock %}

  <script>
    // ── player ──────────────────────────────────────────────────
    const video = document.getElementById('video');
    let hls = null;
    let currentChannelId = null;

    if (video && Hls.isSupported()) {
      hls = new Hls();
      hls.attachMedia(video);
    }

    function _loadSource(url, offset) {
      if (hls) {
        hls.loadSource(url);
        hls.once(Hls.Events.MANIFEST_PARSED, function() {
          if (offset > 0) video.currentTime = offset;
          video.play().catch(function(){});
        });
      } else if (video && video.canPlayType('application/vnd.apple.mpegurl')) {
        video.src = url;
        video.currentTime = offset;
        video.play().catch(function(){});
      }
    }

    function tune(channelId) {
      currentChannelId = channelId;
      document.getElementById('player-panel').style.display = 'block';
      fetch('/channel/' + channelId + '/tune')
        .then(function(r){ return r.json(); })
        .then(function(d){ _loadSource(d.url, d.start_offset_secs); });
    }

    if (video) {
      video.addEventListener('ended', function() {
        if (!currentChannelId) return;
        fetch('/channel/' + currentChannelId + '/next')
          .then(function(r){ return r.json(); })
          .then(function(d){ _loadSource(d.url, d.start_offset_secs); });
      });
    }
  </script>
</body>
</html>
```

- [ ] **Step 4: Create scaffold templates**

Create `/Users/kunwu/Workspace/playground/MyTV/templates/guide.html`:

```html
{% extends "base.html" %}
{% block content %}
<p>guide placeholder</p>
{% endblock %}
```

Create `/Users/kunwu/Workspace/playground/MyTV/templates/partials/epg_content.html`:

```html
<p>epg_content placeholder</p>
```

- [ ] **Step 5: Create a minimal src/routes/guide.rs so it compiles**

Create `/Users/kunwu/Workspace/playground/MyTV/src/routes/guide.rs`:

```rust
use askama::Template;
use axum::{
    extract::{Query, State},
    http::StatusCode,
    response::Html,
};
use serde::Deserialize;

use crate::AppState;

#[derive(Template)]
#[template(path = "guide.html")]
struct GuidePageTemplate {}

#[derive(Debug, Deserialize)]
pub struct GuideQuery {
    pub category: Option<String>,
    pub offset: Option<i64>,
}

pub async fn guide_page(
    State(_state): State<AppState>,
    Query(_params): Query<GuideQuery>,
) -> Result<Html<String>, StatusCode> {
    let html = GuidePageTemplate {}
        .render()
        .map_err(|_| StatusCode::INTERNAL_SERVER_ERROR)?;
    Ok(Html(html))
}

pub async fn guide_partial(
    State(_state): State<AppState>,
    Query(_params): Query<GuideQuery>,
) -> Result<Html<String>, StatusCode> {
    Ok(Html("<p>partial placeholder</p>".into()))
}
```

- [ ] **Step 6: Add pub mod guide to src/routes/mod.rs**

Replace `src/routes/mod.rs` with:

```rust
pub mod guide;
pub mod health;
pub mod player;
```

- [ ] **Step 7: Wire routes in src/main.rs**

Read `src/main.rs`. The current `let app = Router::new()` block should become:

```rust
    let app = Router::new()
        .route("/health", get(routes::health::health_check))
        .route("/guide", get(routes::guide::guide_page))
        .route("/guide/partial", get(routes::guide::guide_partial))
        .route("/channel/:id/tune", get(routes::player::tune))
        .route("/channel/:id/next", get(routes::player::next))
        .with_state(state);
```

- [ ] **Step 8: Verify compile**

```bash
cd /Users/kunwu/Workspace/playground/MyTV && cargo build 2>&1 | grep -E "^error"
```

Expected: no output (no errors)

- [ ] **Step 9: Commit**

```bash
cd /Users/kunwu/Workspace/playground/MyTV && \
git add Cargo.toml Cargo.lock templates/ src/routes/guide.rs src/routes/mod.rs src/main.rs && \
git commit -m "feat: scaffold askama templates and guide routes"
```

---

## Task 2: Guide data layer — types + pure helpers + tests

**Files:**
- Modify: `src/routes/guide.rs` (add structs and pure functions, no handlers yet)

- [ ] **Step 1: Write the failing tests by replacing src/routes/guide.rs**

Replace the entire `src/routes/guide.rs` with:

```rust
use askama::Template;
use axum::{
    extract::{Query, State},
    http::StatusCode,
    response::Html,
};
use chrono::{DateTime, Utc};
use serde::Deserialize;

use crate::{
    channel::{self, Channel, ChannelType},
    epg, playlist_item, AppState,
};

// ── display types ──────────────────────────────────────────────────────────

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

pub struct ChannelRow {
    pub id: i64,
    pub name: String,
    pub programs: Vec<ProgramSlot>,
}

// ── template types (filled in Task 3) ─────────────────────────────────────

#[derive(Template)]
#[template(path = "guide.html")]
struct GuidePageTemplate {}

#[derive(Debug, Deserialize)]
pub struct GuideQuery {
    pub category: Option<String>,
    pub offset: Option<i64>,
}

// ── pure helpers ───────────────────────────────────────────────────────────

/// Returns (window_start, window_end) for the EPG grid.
/// offset_hours: hours from now to window start (default -2 centers "now" in a 4-hour window).
/// Window is always 4 hours wide.
pub fn compute_window(now_secs: i64, offset_hours: i64) -> (DateTime<Utc>, DateTime<Utc>) {
    let start_secs = now_secs + offset_hours * 3600;
    let end_secs = start_secs + 4 * 3600;
    let window_start = DateTime::from_timestamp(start_secs, 0).expect("valid timestamp");
    let window_end = DateTime::from_timestamp(end_secs, 0).expect("valid timestamp");
    (window_start, window_end)
}

/// Converts a ProgramEntry to a ProgramSlot with percentage positioning within the window.
/// Returns None if the entry is completely outside [window_start, window_end].
pub fn entry_to_slot(
    entry: &epg::ProgramEntry,
    window_start: DateTime<Utc>,
    window_end: DateTime<Utc>,
) -> Option<ProgramSlot> {
    if entry.end_time <= window_start || entry.start_time >= window_end {
        return None;
    }
    let window_secs = (window_end - window_start).num_seconds() as f64;
    let visible_start = entry.start_time.max(window_start);
    let visible_end = entry.end_time.min(window_end);
    let left_secs = (visible_start - window_start).num_seconds() as f64;
    let width_secs = (visible_end - visible_start).num_seconds() as f64;
    Some(ProgramSlot {
        title: entry.title.clone(),
        is_live: entry.is_live,
        left_pct: (left_secs / window_secs * 100.0).clamp(0.0, 100.0),
        width_pct: (width_secs / window_secs * 100.0).clamp(0.0, 100.0),
        channel_id: entry.channel_id,
    })
}

/// Returns the "now" line position as a percentage of the window, or None if outside.
pub fn now_line_pct(
    now: DateTime<Utc>,
    window_start: DateTime<Utc>,
    window_end: DateTime<Utc>,
) -> Option<f64> {
    if now < window_start || now > window_end {
        return None;
    }
    let window_secs = (window_end - window_start).num_seconds() as f64;
    let elapsed = (now - window_start).num_seconds() as f64;
    Some((elapsed / window_secs * 100.0).clamp(0.0, 100.0))
}

/// Returns hourly time labels for the visible window, each with a left percentage.
pub fn time_labels(window_start: DateTime<Utc>, window_end: DateTime<Utc>) -> Vec<TimeLabel> {
    let window_secs = (window_end - window_start).num_seconds() as f64;
    let start_ts = window_start.timestamp();
    let end_ts = window_end.timestamp();
    let rem = start_ts.rem_euclid(3600);
    let first_tick = if rem == 0 { start_ts } else { start_ts + (3600 - rem) };
    let mut labels = Vec::new();
    let mut ts = first_tick;
    while ts <= end_ts {
        let dt = DateTime::from_timestamp(ts, 0).expect("valid ts");
        let elapsed = (dt - window_start).num_seconds() as f64;
        labels.push(TimeLabel {
            label: dt.format("%H:%M").to_string(),
            left_pct: (elapsed / window_secs * 100.0).clamp(0.0, 100.0),
        });
        ts += 3600;
    }
    labels
}

// ── stub handlers (replaced in Task 3) ────────────────────────────────────

pub async fn guide_page(
    State(_state): State<AppState>,
    Query(_params): Query<GuideQuery>,
) -> Result<Html<String>, StatusCode> {
    let html = GuidePageTemplate {}
        .render()
        .map_err(|_| StatusCode::INTERNAL_SERVER_ERROR)?;
    Ok(Html(html))
}

pub async fn guide_partial(
    State(_state): State<AppState>,
    Query(_params): Query<GuideQuery>,
) -> Result<Html<String>, StatusCode> {
    Ok(Html("<p>partial placeholder</p>".into()))
}

// ── tests ──────────────────────────────────────────────────────────────────

#[cfg(test)]
mod tests {
    use super::*;
    use chrono::Utc;

    fn dt(secs: i64) -> DateTime<Utc> {
        DateTime::from_timestamp(secs, 0).unwrap()
    }

    fn make_entry(channel_id: i64, start: i64, end: i64, is_live: bool) -> epg::ProgramEntry {
        epg::ProgramEntry {
            channel_id,
            title: "Test".to_string(),
            url: String::new(),
            start_time: dt(start),
            end_time: dt(end),
            is_live,
            start_offset_secs: 0,
        }
    }

    // window: ts 0–14400 (4 hours)
    fn w() -> (DateTime<Utc>, DateTime<Utc>) {
        (dt(0), dt(14400))
    }

    // ── compute_window ─────────────────────────────────────────

    #[test]
    fn test_compute_window_default_offset() {
        // offset=-2: window starts 2h before now, window is 4h wide
        let now = 100_000i64;
        let (start, end) = compute_window(now, -2);
        assert_eq!(start.timestamp(), now - 7200);
        assert_eq!(end.timestamp(), now + 7200);
        assert_eq!((end - start).num_hours(), 4);
    }

    #[test]
    fn test_compute_window_positive_offset() {
        // offset=4: window starts 4h after now
        let now = 100_000i64;
        let (start, end) = compute_window(now, 4);
        assert_eq!(start.timestamp(), now + 4 * 3600);
        assert_eq!(end.timestamp(), now + 8 * 3600);
    }

    // ── entry_to_slot ──────────────────────────────────────────

    #[test]
    fn test_entry_to_slot_fully_within_window() {
        let (ws, we) = w();
        // 1h–2h in a 0–4h window
        let e = make_entry(1, 3600, 7200, false);
        let slot = entry_to_slot(&e, ws, we).unwrap();
        assert!((slot.left_pct - 25.0).abs() < 0.01, "left={}", slot.left_pct);
        assert!((slot.width_pct - 25.0).abs() < 0.01, "width={}", slot.width_pct);
        assert!(!slot.is_live);
    }

    #[test]
    fn test_entry_to_slot_live_flag_preserved() {
        let (ws, we) = w();
        let e = make_entry(1, 0, 14400, true);
        let slot = entry_to_slot(&e, ws, we).unwrap();
        assert!(slot.is_live);
    }

    #[test]
    fn test_entry_to_slot_clipped_left() {
        let (ws, we) = w();
        // starts -1h before window, ends at 1h inside
        let e = make_entry(1, -3600, 3600, false);
        let slot = entry_to_slot(&e, ws, we).unwrap();
        assert!((slot.left_pct - 0.0).abs() < 0.01, "left={}", slot.left_pct);
        // visible: 0 to 3600 = 3600s out of 14400 = 25%
        assert!((slot.width_pct - 25.0).abs() < 0.01, "width={}", slot.width_pct);
    }

    #[test]
    fn test_entry_to_slot_clipped_right() {
        let (ws, we) = w();
        // starts at 3h, ends at 5h (clips at 4h)
        let e = make_entry(1, 10800, 18000, false);
        let slot = entry_to_slot(&e, ws, we).unwrap();
        assert!((slot.left_pct - 75.0).abs() < 0.01, "left={}", slot.left_pct);
        // visible: 10800 to 14400 = 3600s = 25%
        assert!((slot.width_pct - 25.0).abs() < 0.01, "width={}", slot.width_pct);
    }

    #[test]
    fn test_entry_to_slot_entirely_before_window() {
        let (ws, we) = w();
        let e = make_entry(1, -7200, -3600, false);
        assert!(entry_to_slot(&e, ws, we).is_none());
    }

    #[test]
    fn test_entry_to_slot_entirely_after_window() {
        let (ws, we) = w();
        let e = make_entry(1, 18000, 21600, false);
        assert!(entry_to_slot(&e, ws, we).is_none());
    }

    // ── now_line_pct ───────────────────────────────────────────

    #[test]
    fn test_now_line_pct_at_midpoint() {
        let (ws, we) = w(); // 0 to 14400
        let pct = now_line_pct(dt(7200), ws, we).unwrap();
        assert!((pct - 50.0).abs() < 0.01, "pct={}", pct);
    }

    #[test]
    fn test_now_line_pct_outside_window() {
        let (ws, we) = w();
        assert!(now_line_pct(dt(-1), ws, we).is_none());
        assert!(now_line_pct(dt(14401), ws, we).is_none());
    }

    // ── time_labels ────────────────────────────────────────────

    #[test]
    fn test_time_labels_aligned_4h_window() {
        // window 0:00–4:00 UTC (ts 0–14400, both exactly on hour boundaries)
        let (ws, we) = w();
        let labels = time_labels(ws, we);
        // ticks at 0, 3600, 7200, 10800, 14400 → 5 labels
        assert_eq!(labels.len(), 5, "expected 5 labels, got {}", labels.len());
        assert_eq!(labels[0].label, "00:00");
        assert!((labels[0].left_pct - 0.0).abs() < 0.01);
        assert_eq!(labels[4].label, "04:00");
        assert!((labels[4].left_pct - 100.0).abs() < 0.01);
    }

    #[test]
    fn test_time_labels_non_aligned_start() {
        // window starts at ts=1800 (00:30), ends at ts=16200 (04:30)
        let ws = dt(1800);
        let we = dt(16200);
        let labels = time_labels(ws, we);
        // ticks: 3600 (01:00), 7200 (02:00), 10800 (03:00), 14400 (04:00) → 4 labels
        assert_eq!(labels.len(), 4, "expected 4 labels, got {}", labels.len());
        assert_eq!(labels[0].label, "01:00");
        // left_pct for 01:00 = (3600-1800)/(16200-1800)*100 = 1800/14400*100 = 12.5%
        assert!((labels[0].left_pct - 12.5).abs() < 0.01);
    }
}
```

- [ ] **Step 2: Run tests to confirm compile error**

```bash
cd /Users/kunwu/Workspace/playground/MyTV && cargo test guide 2>&1 | head -10
```

Expected: compile error — `epg::ProgramEntry` fields are private or there's a visibility issue, OR it compiles fine if `ProgramEntry` fields are public (they are). In either case, if it compiles, proceed to Step 3. If there's a genuine compile error about struct field visibility, note it and fix by reading `src/epg.rs` — all fields are `pub`.

- [ ] **Step 3: Run tests to confirm they pass**

```bash
cd /Users/kunwu/Workspace/playground/MyTV && cargo test guide 2>&1
```

Expected: all 12 tests pass (guide module)

- [ ] **Step 4: Run full suite to confirm no regressions**

```bash
cd /Users/kunwu/Workspace/playground/MyTV && cargo test -- --test-threads=1 2>&1 | tail -5
```

Expected: all tests pass

- [ ] **Step 5: Commit**

```bash
cd /Users/kunwu/Workspace/playground/MyTV && \
git add src/routes/guide.rs && \
git commit -m "feat: add epg guide data layer with pure helper functions"
```

---

## Task 3: Guide routes + full templates

**Files:**
- Modify: `src/routes/guide.rs` (replace stub handlers with real ones; add template structs)
- Modify: `templates/guide.html` (full page content)
- Modify: `templates/partials/epg_content.html` (full EPG grid partial)

### About the template structs

`GuidePageTemplate` extends `base.html` and includes `partials/epg_content.html` via `{% include %}`. The `{% include %}` shares the parent template's variable scope, so both templates see the same fields.

`EpgContentTemplate` uses `partials/epg_content.html` directly as its template (no extension), allowing the `/guide/partial` handler to return just the partial HTML fragment without the page shell.

Both structs have identical fields — a shared `build_guide_data` helper populates them.

- [ ] **Step 1: Replace src/routes/guide.rs with the full implementation**

Replace the entire file with:

```rust
use askama::Template;
use axum::{
    extract::{Query, State},
    http::StatusCode,
    response::Html,
};
use chrono::{DateTime, Utc};
use serde::Deserialize;
use sqlx::SqlitePool;

use crate::{
    channel::{self, Channel, ChannelType},
    epg, playlist_item, AppState,
};

// ── display types ──────────────────────────────────────────────────────────

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

pub struct ChannelRow {
    pub id: i64,
    pub name: String,
    pub programs: Vec<ProgramSlot>,
}

// ── template structs ───────────────────────────────────────────────────────

#[derive(Template)]
#[template(path = "guide.html")]
struct GuidePageTemplate {
    categories: Vec<String>,
    active_category: String,
    offset_hours: i64,
    window_label: String,
    labels: Vec<TimeLabel>,
    now_pct: Option<f64>,
    rows: Vec<ChannelRow>,
}

#[derive(Template)]
#[template(path = "partials/epg_content.html")]
struct EpgContentTemplate {
    categories: Vec<String>,
    active_category: String,
    offset_hours: i64,
    window_label: String,
    labels: Vec<TimeLabel>,
    now_pct: Option<f64>,
    rows: Vec<ChannelRow>,
}

// ── query params ───────────────────────────────────────────────────────────

#[derive(Debug, Deserialize)]
pub struct GuideQuery {
    pub category: Option<String>,
    pub offset: Option<i64>,
}

// ── pure helpers ───────────────────────────────────────────────────────────

pub fn compute_window(now_secs: i64, offset_hours: i64) -> (DateTime<Utc>, DateTime<Utc>) {
    let start_secs = now_secs + offset_hours * 3600;
    let end_secs = start_secs + 4 * 3600;
    let window_start = DateTime::from_timestamp(start_secs, 0).expect("valid timestamp");
    let window_end = DateTime::from_timestamp(end_secs, 0).expect("valid timestamp");
    (window_start, window_end)
}

pub fn entry_to_slot(
    entry: &epg::ProgramEntry,
    window_start: DateTime<Utc>,
    window_end: DateTime<Utc>,
) -> Option<ProgramSlot> {
    if entry.end_time <= window_start || entry.start_time >= window_end {
        return None;
    }
    let window_secs = (window_end - window_start).num_seconds() as f64;
    let visible_start = entry.start_time.max(window_start);
    let visible_end = entry.end_time.min(window_end);
    let left_secs = (visible_start - window_start).num_seconds() as f64;
    let width_secs = (visible_end - visible_start).num_seconds() as f64;
    Some(ProgramSlot {
        title: entry.title.clone(),
        is_live: entry.is_live,
        left_pct: (left_secs / window_secs * 100.0).clamp(0.0, 100.0),
        width_pct: (width_secs / window_secs * 100.0).clamp(0.0, 100.0),
        channel_id: entry.channel_id,
    })
}

pub fn now_line_pct(
    now: DateTime<Utc>,
    window_start: DateTime<Utc>,
    window_end: DateTime<Utc>,
) -> Option<f64> {
    if now < window_start || now > window_end {
        return None;
    }
    let window_secs = (window_end - window_start).num_seconds() as f64;
    let elapsed = (now - window_start).num_seconds() as f64;
    Some((elapsed / window_secs * 100.0).clamp(0.0, 100.0))
}

pub fn time_labels(window_start: DateTime<Utc>, window_end: DateTime<Utc>) -> Vec<TimeLabel> {
    let window_secs = (window_end - window_start).num_seconds() as f64;
    let start_ts = window_start.timestamp();
    let end_ts = window_end.timestamp();
    let rem = start_ts.rem_euclid(3600);
    let first_tick = if rem == 0 { start_ts } else { start_ts + (3600 - rem) };
    let mut labels = Vec::new();
    let mut ts = first_tick;
    while ts <= end_ts {
        let dt = DateTime::from_timestamp(ts, 0).expect("valid ts");
        let elapsed = (dt - window_start).num_seconds() as f64;
        labels.push(TimeLabel {
            label: dt.format("%H:%M").to_string(),
            left_pct: (elapsed / window_secs * 100.0).clamp(0.0, 100.0),
        });
        ts += 3600;
    }
    labels
}

// ── data builder ───────────────────────────────────────────────────────────

struct GuideData {
    categories: Vec<String>,
    active_category: String,
    offset_hours: i64,
    window_label: String,
    labels: Vec<TimeLabel>,
    now_pct: Option<f64>,
    rows: Vec<ChannelRow>,
}

async fn build_guide_data(
    pool: &SqlitePool,
    category: &str,
    offset_hours: i64,
) -> anyhow::Result<GuideData> {
    let now = Utc::now();
    let (window_start, window_end) = compute_window(now.timestamp(), offset_hours);

    let all_channels = channel::list(pool).await?;
    let categories = channel::distinct_categories(&all_channels);

    let channels: Vec<Channel> = if category == "all" {
        all_channels
    } else {
        channel::list_by_category(pool, category).await?
    };

    let mut rows = Vec::new();
    for ch in &channels {
        let entries = match ch.channel_type() {
            ChannelType::Live => vec![epg::live_entry(ch.id, &ch.name, window_start, window_end)],
            ChannelType::VodLoop => {
                if let Some(anchor) = ch.loop_anchor {
                    let items = playlist_item::list_for_channel(pool, ch.id).await?;
                    epg::vod_schedule(ch.id, &items, anchor.timestamp(), window_start, window_end)
                } else {
                    vec![]
                }
            }
        };
        let programs: Vec<ProgramSlot> = entries
            .iter()
            .filter_map(|e| entry_to_slot(e, window_start, window_end))
            .collect();
        rows.push(ChannelRow {
            id: ch.id,
            name: ch.name.clone(),
            programs,
        });
    }

    Ok(GuideData {
        categories,
        active_category: category.to_string(),
        offset_hours,
        window_label: format!(
            "{} – {}",
            window_start.format("%H:%M"),
            window_end.format("%H:%M")
        ),
        labels: time_labels(window_start, window_end),
        now_pct: now_line_pct(now, window_start, window_end),
        rows,
    })
}

// ── handlers ───────────────────────────────────────────────────────────────

pub async fn guide_page(
    State(state): State<AppState>,
    Query(params): Query<GuideQuery>,
) -> Result<Html<String>, StatusCode> {
    let category = params.category.unwrap_or_else(|| "all".to_string());
    let offset_hours = params.offset.unwrap_or(-2);

    let data = build_guide_data(&state.pool, &category, offset_hours)
        .await
        .map_err(|_| StatusCode::INTERNAL_SERVER_ERROR)?;

    let html = GuidePageTemplate {
        categories: data.categories,
        active_category: data.active_category,
        offset_hours: data.offset_hours,
        window_label: data.window_label,
        labels: data.labels,
        now_pct: data.now_pct,
        rows: data.rows,
    }
    .render()
    .map_err(|_| StatusCode::INTERNAL_SERVER_ERROR)?;

    Ok(Html(html))
}

pub async fn guide_partial(
    State(state): State<AppState>,
    Query(params): Query<GuideQuery>,
) -> Result<Html<String>, StatusCode> {
    let category = params.category.unwrap_or_else(|| "all".to_string());
    let offset_hours = params.offset.unwrap_or(-2);

    let data = build_guide_data(&state.pool, &category, offset_hours)
        .await
        .map_err(|_| StatusCode::INTERNAL_SERVER_ERROR)?;

    let html = EpgContentTemplate {
        categories: data.categories,
        active_category: data.active_category,
        offset_hours: data.offset_hours,
        window_label: data.window_label,
        labels: data.labels,
        now_pct: data.now_pct,
        rows: data.rows,
    }
    .render()
    .map_err(|_| StatusCode::INTERNAL_SERVER_ERROR)?;

    Ok(Html(html))
}

// ── tests ──────────────────────────────────────────────────────────────────

#[cfg(test)]
mod tests {
    use super::*;
    use chrono::Utc;

    fn dt(secs: i64) -> DateTime<Utc> {
        DateTime::from_timestamp(secs, 0).unwrap()
    }

    fn make_entry(channel_id: i64, start: i64, end: i64, is_live: bool) -> epg::ProgramEntry {
        epg::ProgramEntry {
            channel_id,
            title: "Test".to_string(),
            url: String::new(),
            start_time: dt(start),
            end_time: dt(end),
            is_live,
            start_offset_secs: 0,
        }
    }

    fn w() -> (DateTime<Utc>, DateTime<Utc>) {
        (dt(0), dt(14400))
    }

    #[test]
    fn test_compute_window_default_offset() {
        let now = 100_000i64;
        let (start, end) = compute_window(now, -2);
        assert_eq!(start.timestamp(), now - 7200);
        assert_eq!(end.timestamp(), now + 7200);
        assert_eq!((end - start).num_hours(), 4);
    }

    #[test]
    fn test_compute_window_positive_offset() {
        let now = 100_000i64;
        let (start, end) = compute_window(now, 4);
        assert_eq!(start.timestamp(), now + 4 * 3600);
        assert_eq!(end.timestamp(), now + 8 * 3600);
    }

    #[test]
    fn test_entry_to_slot_fully_within_window() {
        let (ws, we) = w();
        let e = make_entry(1, 3600, 7200, false);
        let slot = entry_to_slot(&e, ws, we).unwrap();
        assert!((slot.left_pct - 25.0).abs() < 0.01, "left={}", slot.left_pct);
        assert!((slot.width_pct - 25.0).abs() < 0.01, "width={}", slot.width_pct);
        assert!(!slot.is_live);
    }

    #[test]
    fn test_entry_to_slot_live_flag_preserved() {
        let (ws, we) = w();
        let e = make_entry(1, 0, 14400, true);
        let slot = entry_to_slot(&e, ws, we).unwrap();
        assert!(slot.is_live);
    }

    #[test]
    fn test_entry_to_slot_clipped_left() {
        let (ws, we) = w();
        let e = make_entry(1, -3600, 3600, false);
        let slot = entry_to_slot(&e, ws, we).unwrap();
        assert!((slot.left_pct - 0.0).abs() < 0.01, "left={}", slot.left_pct);
        assert!((slot.width_pct - 25.0).abs() < 0.01, "width={}", slot.width_pct);
    }

    #[test]
    fn test_entry_to_slot_clipped_right() {
        let (ws, we) = w();
        let e = make_entry(1, 10800, 18000, false);
        let slot = entry_to_slot(&e, ws, we).unwrap();
        assert!((slot.left_pct - 75.0).abs() < 0.01, "left={}", slot.left_pct);
        assert!((slot.width_pct - 25.0).abs() < 0.01, "width={}", slot.width_pct);
    }

    #[test]
    fn test_entry_to_slot_entirely_before_window() {
        let (ws, we) = w();
        assert!(entry_to_slot(&make_entry(1, -7200, -3600, false), ws, we).is_none());
    }

    #[test]
    fn test_entry_to_slot_entirely_after_window() {
        let (ws, we) = w();
        assert!(entry_to_slot(&make_entry(1, 18000, 21600, false), ws, we).is_none());
    }

    #[test]
    fn test_now_line_pct_at_midpoint() {
        let (ws, we) = w();
        let pct = now_line_pct(dt(7200), ws, we).unwrap();
        assert!((pct - 50.0).abs() < 0.01, "pct={}", pct);
    }

    #[test]
    fn test_now_line_pct_outside_window() {
        let (ws, we) = w();
        assert!(now_line_pct(dt(-1), ws, we).is_none());
        assert!(now_line_pct(dt(14401), ws, we).is_none());
    }

    #[test]
    fn test_time_labels_aligned_4h_window() {
        let (ws, we) = w();
        let labels = time_labels(ws, we);
        assert_eq!(labels.len(), 5, "expected 5 labels, got {}", labels.len());
        assert_eq!(labels[0].label, "00:00");
        assert!((labels[0].left_pct - 0.0).abs() < 0.01);
        assert_eq!(labels[4].label, "04:00");
        assert!((labels[4].left_pct - 100.0).abs() < 0.01);
    }

    #[test]
    fn test_time_labels_non_aligned_start() {
        // window 00:30–04:30 UTC (ts 1800–16200)
        let labels = time_labels(dt(1800), dt(16200));
        assert_eq!(labels.len(), 4, "expected 4 labels, got {}", labels.len());
        assert_eq!(labels[0].label, "01:00");
        // (3600-1800)/(16200-1800)*100 = 12.5%
        assert!((labels[0].left_pct - 12.5).abs() < 0.01);
    }
}
```

- [ ] **Step 2: Verify compile**

```bash
cd /Users/kunwu/Workspace/playground/MyTV && cargo build 2>&1 | grep -E "^error"
```

Expected: no output. (The templates still have placeholder content — compile-time template validation will catch Askama errors.)

- [ ] **Step 3: Replace templates/guide.html with full content**

Replace `/Users/kunwu/Workspace/playground/MyTV/templates/guide.html`:

```html
{% extends "base.html" %}
{% block content %}
<div id="player-panel">
  <video id="video" controls></video>
</div>

<div id="epg-content">
  {% include "partials/epg_content.html" %}
</div>
{% endblock %}
```

- [ ] **Step 4: Replace templates/partials/epg_content.html with full content**

Replace `/Users/kunwu/Workspace/playground/MyTV/templates/partials/epg_content.html`:

```html
<div class="epg-nav">
  <div class="tabs">
    <a class="tab{% if active_category == "all" %} active{% endif %}"
       hx-get="/guide/partial?category=all&amp;offset={{ offset_hours }}"
       hx-target="#epg-content"
       hx-swap="innerHTML">All</a>
    {% for cat in categories %}
    <a class="tab{% if active_category == cat %} active{% endif %}"
       hx-get="/guide/partial?category={{ cat }}&amp;offset={{ offset_hours }}"
       hx-target="#epg-content"
       hx-swap="innerHTML">{{ cat }}</a>
    {% endfor %}
  </div>
  <div class="time-nav">
    <a class="nav-btn"
       hx-get="/guide/partial?category={{ active_category }}&amp;offset={{ offset_hours - 2 }}"
       hx-target="#epg-content"
       hx-swap="innerHTML">&#8592; Earlier</a>
    <span class="time-range">{{ window_label }}</span>
    <a class="nav-btn"
       hx-get="/guide/partial?category={{ active_category }}&amp;offset={{ offset_hours + 2 }}"
       hx-target="#epg-content"
       hx-swap="innerHTML">Later &#8594;</a>
  </div>
</div>

<div class="epg-wrapper">
  <div class="epg-grid">
    <!-- Time header row -->
    <div class="epg-row time-header">
      <div class="channel-col"></div>
      <div class="programs-col">
        {% for label in labels %}
        <span class="time-label" style="left: {{ label.left_pct }}%">{{ label.label }}</span>
        {% endfor %}
        {% if let Some(pct) = now_pct %}
        <div class="now-line" style="left: {{ pct }}%"></div>
        {% endif %}
      </div>
    </div>
    <!-- Channel rows -->
    {% for row in rows %}
    <div class="epg-row">
      <div class="channel-col">{{ row.name }}</div>
      <div class="programs-col">
        {% if let Some(pct) = now_pct %}
        <div class="now-line" style="left: {{ pct }}%"></div>
        {% endif %}
        {% for prog in row.programs %}
        <div class="program{% if prog.is_live %} live{% endif %}"
             style="left: {{ prog.left_pct }}%; width: {{ prog.width_pct }}%"
             onclick="tune({{ prog.channel_id }})">
          {% if prog.is_live %}<span class="live-badge">LIVE</span>{% endif %}
          <span class="program-title">{{ prog.title }}</span>
        </div>
        {% endfor %}
      </div>
    </div>
    {% endfor %}
  </div>
</div>
```

**Note on `{% if let Some(pct) = now_pct %}`:** This is valid Askama 0.12 syntax. If your version of askama does not support `if let`, replace with:
```html
{% if now_pct.is_some() %}
<div class="now-line" style="left: {{ now_pct.unwrap() }}%"></div>
{% endif %}
```

**Note on `&amp;`:** In HTML attributes, `&` must be written as `&amp;`. This is required for valid HTML in HTMX `hx-get` URLs with multiple query params.

- [ ] **Step 5: Build to confirm templates compile**

```bash
cd /Users/kunwu/Workspace/playground/MyTV && cargo build 2>&1 | grep -E "^error"
```

Expected: no output. Askama validates templates at compile time — any template syntax errors appear here.

If you see a template error like `unknown filter` or `expression not supported`, read the error message carefully. Common fixes:
- `{% if let %}` not supported → use `{% if now_pct.is_some() %}` / `now_pct.unwrap()` pattern
- Arithmetic `offset_hours - 2` not supported → pre-compute these values in Rust and pass as separate template fields

- [ ] **Step 6: Run all tests**

```bash
cd /Users/kunwu/Workspace/playground/MyTV && cargo test -- --test-threads=1 2>&1 | tail -5
```

Expected: all tests pass (no regressions)

- [ ] **Step 7: Smoke test the guide endpoint**

In one terminal (background):
```bash
cd /Users/kunwu/Workspace/playground/MyTV && RUST_LOG=info cargo run &
sleep 3
```

In another terminal:
```bash
# Full page returns 200 with HTML
curl -s -o /dev/null -w "%{http_code}" http://localhost:3000/guide
# Should print: 200

# Partial returns 200 with HTML fragment
curl -s -o /dev/null -w "%{http_code}" "http://localhost:3000/guide/partial?category=all&offset=-2"
# Should print: 200

# Category filter works (no channels exist yet so returns same empty grid)
curl -s -o /dev/null -w "%{http_code}" "http://localhost:3000/guide?category=news"
# Should print: 200

kill %1 2>/dev/null || true
```

Expected: all three return 200

- [ ] **Step 8: Commit**

```bash
cd /Users/kunwu/Workspace/playground/MyTV && \
git add src/routes/guide.rs templates/ && \
git commit -m "feat: add epg guide routes with category tabs and time navigation"
```

---

## Task 4: Player panel + hls.js integration

The player JS is already in `templates/base.html` (added in Task 1). This task wires up the player panel display and `onclick="tune()"` on program blocks, then smoke-tests the full UI with real data.

**Files:**
- No code changes needed if Task 1 and 3 were done correctly — the JS `tune()` function and `onclick` attributes are already in place.
- Verify the player panel is visible and functional with a manual smoke test using seeded data.

- [ ] **Step 1: Seed test data into the DB**

Start the server to initialize the DB, then insert test channels:

```bash
cd /Users/kunwu/Workspace/playground/MyTV && cargo run &
sleep 2

# Insert two live channels and one VOD loop channel
sqlite3 mytv.db "
INSERT INTO channels (name, category, logo_url, type, sort_order)
VALUES
  ('CNN International', 'news', NULL, 'live', 1),
  ('Al Jazeera English', 'news', NULL, 'live', 2),
  ('AI Daily', 'ai', NULL, 'live', 3);

INSERT INTO sources (channel_id, kind, url, priority, is_active)
VALUES
  (1, 'hls', 'https://cnn-cnninternational-1-eu.rakuten.wurl.tv/manifest/playlist.m3u8', 1, 1),
  (2, 'hls', 'https://live-hls-web-aje.getaj.net/AJE/index.m3u8', 1, 1),
  (3, 'hls', 'https://dai.google.com/linear/hls/event/Sid4xiTQTkCT1bvYXx5m5A/master.m3u8', 1, 1);
"

kill %1 2>/dev/null || true
```

Note: these are example public HLS URLs — they may or may not be active at time of testing. Use any working HLS stream URL you know.

- [ ] **Step 2: Start the server and open the guide in a browser**

```bash
cd /Users/kunwu/Workspace/playground/MyTV && RUST_LOG=info cargo run
```

Open `http://localhost:3000/guide` in a browser.

Verify:
- [ ] The EPG grid shows channel names and LIVE blocks
- [ ] Category tabs appear ("All", "news", "ai")
- [ ] Clicking "All" or a category tab swaps the grid rows via HTMX (no page reload)
- [ ] Clicking "← Earlier" / "Later →" shifts the time window by 2h
- [ ] The red "now" line is visible in the grid
- [ ] Clicking any program block starts playback in the player panel above the grid
- [ ] The player panel appears and the video loads
- [ ] When a live stream plays: video plays continuously without calling `/next`
- [ ] Check browser console for JavaScript errors — should be none

- [ ] **Step 3: Verify hls.js fallback for non-HLS sources**

If you have an HLS stream URL (`.m3u8`), the player should load it. Check the browser console for hls.js logs confirming manifest is fetched.

Expected console output:
```
Hls.js v... - ... - ...
[...] MANIFEST_PARSED ...
```

- [ ] **Step 4: Commit seed data note**

The SQLite database file (`mytv.db`) is in `.gitignore`. No commit needed for data. Commit only if you made any code fixes during testing:

```bash
cd /Users/kunwu/Workspace/playground/MyTV && cargo test -- --test-threads=1 2>&1 | tail -3
```

All tests must still pass after any fixes.

```bash
cd /Users/kunwu/Workspace/playground/MyTV && \
git add -p  # only stage code changes, not mytv.db
git commit -m "fix: <describe any fix made during smoke test>"
```

If no code changes were needed, skip this commit.

---

## Self-Review

**Spec coverage:**

| Spec requirement | Plan task |
|---|---|
| EPG grid at `/guide` | ✅ Task 3 |
| Category tabs filter channel rows (HTMX swap) | ✅ Task 3 `epg_content.html` tabs |
| 24h window scrollable horizontally in 2h steps | ✅ Task 3 nav buttons (`offset_hours ± 2`) |
| Red "now" line marks current time | ✅ Task 3 `now_pct` + CSS `.now-line` |
| Live channels show "LIVE" badge | ✅ Task 3 `prog.is_live` + `.live-badge` |
| Clicking program row loads player | ✅ Task 3 `onclick="tune()"` |
| Player uses hls.js | ✅ Task 1 CDN + Task 1 JS |
| On tune-in: calls `/channel/:id/tune` | ✅ Task 1 `tune()` JS function |
| When asset ends: calls `/channel/:id/next` | ✅ Task 1 `video.ended` handler |
| Mobile: collapses to vertical list | ✅ Task 1 CSS `@media(max-width:600px)` |

**Placeholder scan:** All steps have complete code. No TBDs.

**Type consistency:**
- `ProgramSlot.left_pct: f64` / `width_pct: f64` — used in templates as `{{ prog.left_pct }}`
- `TimeLabel.left_pct: f64` — used in templates as `{{ label.left_pct }}`
- `GuidePageTemplate` and `EpgContentTemplate` have identical field names — `{% include %}` in `guide.html` shares scope correctly
- `offset_hours: i64` — used in template arithmetic `offset_hours - 2` and `offset_hours + 2`
- `now_pct: Option<f64>` — `{% if let Some(pct) = now_pct %}` in template

---

## Next Plans

- **Plan 4:** Admin UI — CRUD pages for channels, sources, and playlists; password-protected `/admin` routes; Askama admin templates
- **Plan 5:** Discovery tools — YouTube Data API search, iptv-org M3U import, manual URL entry
