# Live-Status Visibility — Design

Date: 2026-06-10
Follow-up to: docs/superpowers/specs/2026-06-09-youtube-discover-improvements-design.md

## Problem

A YouTube `youtube_live` source backed by a channel `/live` URL (or `@handle/live`)
only resolves while that channel is actively broadcasting. NASA event channels,
for example, are idle most of the time. Today:

- The 15-min health checker, for `kind == "youtube_live"`, only checks HTTP
  reachability (`do_http_check` returns `(true, None)` on any 2xx/3xx). YouTube
  serves a 200 page for an offline `/live` URL, so an idle live channel is shown
  as **healthy** even when it is not broadcasting.
- The only place "not currently live" surfaces is at tune time, as a 503.

So an admin has no way to see whether a live channel is actually live right now.

## Goal (this spec)

Surface **fresh-on-view** live status (Live / Offline / Unknown) wherever a
YouTube live source appears in the admin UI:

1. The channel-URL / @handle resolver result (discover 2b).
2. The keyword channel-search results (discover 2a) — and live video rows.
3. The admin channel/source pages (source rows).

Out of scope (deferred): auto-resume playback. A stored/point-in-time status is
the wrong model for a toggling channel — status must be checked when viewed.

## Probe primitive (verified empirically)

`yt-dlp --print is_live --no-playlist -- <url>`:
- live channel → stdout `True`, exit 0
- offline channel → stderr `ERROR: [youtube:tab] …: The channel is not currently live`, non-zero exit
- VOD / non-live → stdout `False`, exit 0

## Section A — Probe primitive + cache

**`src/media/resolver.rs`:**

```rust
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum LiveStatus {
    Live,
    Offline,
    Unknown,
}

// Pure, unit-testable — no process, no network.
pub fn interpret_is_live(success: bool, stdout: &str, stderr: &str) -> LiveStatus {
    let out = stdout.trim();
    if success && out == "True" {
        return LiveStatus::Live;
    }
    if success && out == "False" {
        return LiveStatus::Offline;
    }
    if stderr.to_ascii_lowercase().contains("not currently live") {
        return LiveStatus::Offline;
    }
    LiveStatus::Unknown
}

// Runs yt-dlp (8s timeout); times out / spawn failure → Unknown.
pub async fn probe_live(url: &str) -> LiveStatus { /* spawn yt-dlp, call interpret_is_live */ }
```

**Cache** — mirrors the existing `cors_cache` / ssrf-cache pattern (an
`Arc<RwLock<HashMap<…>>>` field on `AppState`):

```rust
pub type LiveStatusCache = Arc<RwLock<HashMap<String, (LiveStatus, Instant)>>>;
```

- TTL: **60 s**, keyed by URL.
- Helper (lives near the cache definition, e.g. in `health.rs` or a small module):
  `async fn cached_live_status(cache: &LiveStatusCache, url: &str) -> LiveStatus`
  — returns the cached value if younger than 60 s, else calls `probe_live`,
  stores `(status, Instant::now())`, and returns it.
- `Instant::now()` is acceptable here (runtime cache), consistent with the
  ssrf/cors caches already in the codebase.

## Section B — Unified endpoint + badge

**Route** (`src/lib.rs`, admin sub-router so auth applies):
`GET /admin/live-status?url=<url>` → `live_status_badge` handler.

Handler:
1. If `!resolver::needs_resolution(&url)` → render the badge partial with
   `LiveStatus::Unknown` rendered as a neutral dot (never run yt-dlp on a
   non-YouTube/Twitch host).
2. Else `cached_live_status(&state.live_cache, &url)` → render the badge.

**Badge partial** `templates/admin/partials/live_status_badge.html`:
- `Live` → `<span style="color:#4caf50" title="Currently live">● live</span>`
- `Offline` → `<span style="color:#888" title="Not currently live">○ offline</span>`
- `Unknown` → `<span style="color:#666" title="Live status unknown">· ?</span>`

Templates build the query with Askama's built-in `|urlencode` filter, so a single
endpoint serves every surface.

## Section C — Lazy consumers, tests, ideas entry

All consumers use the same lazy span (fires once on load, replaces itself):

```html
<span hx-get="/admin/live-status?url={{ <the-url> | urlencode }}"
      hx-trigger="load" hx-swap="outerHTML"
      style="color:#666">checking…</span>
```

1. **`templates/admin/partials/discover_manual_result.html`** — add the lazy
   badge when the resolved URL needs resolution (channel-URL resolver, 2b).
2. **`templates/admin/partials/discover_yt_results.html`** — add the lazy badge
   in a cell for any row where `row.is_live` (channel rows + live video rows).
3. **`templates/admin/partials/source_row.html`** — add a "Live" column; render
   the lazy badge only when `src.kind == "youtube_live"`, otherwise a blank cell.
   Add the matching `<th>Live</th>` to the sources table header in
   `templates/admin/channel_detail.html`.

**Cost note:** a 12-result channel search fires up to 12 background probes. They
are parallel (independent HTMX requests), bounded by result count, and cached for
60 s. Acceptable; documented here so it is not a silent surprise.

**Tests:**
- Unit (`resolver.rs`): `interpret_is_live` — `True`→Live, `False`→Offline,
  stderr "not currently live"→Offline, other/empty→Unknown.
- Integration (`tests/http.rs`):
  - `GET /admin/live-status?url=<non-youtube>` (authed) → 200 + neutral badge
    (asserts no live/offline marker), proving the `needs_resolution` gate.
  - `GET /admin/live-status` without auth → 401 (auth required).
- The live yt-dlp path requires network; covered by the manual checklist, not CI.

**Deferred-work entry:** append an idea to `docs/IDEAS.md`:
"Auto-resume offline live channels — when a live source is offline at tune time,
the player shows a waiting state and auto-retries on an interval, resuming
playback when the channel returns (instead of a hard 503)."

## Non-goals

- No change to the health checker (no yt-dlp in the 15-min sweep — too costly).
- No persisted live status / no schema change (cache is in-memory only).
- No auto-resume playback (separate spec).
- No change to tune/resolve behavior.

## Conventions

- `cargo fmt` → `cargo clippy -- -D warnings` → `cargo test` before each commit.
- No comments unless the WHY is non-obvious.
- Askama `{% match %}`/`{% when %}` for `Option`. The badge partial takes the
  `LiveStatus` enum in its template struct and renders it with
  `{% match status %}{% when LiveStatus::Live %}…{% when LiveStatus::Offline %}…
  {% when LiveStatus::Unknown %}…{% endmatch %}` (import the enum into the
  template's scope the same way other model enums are used in templates).
