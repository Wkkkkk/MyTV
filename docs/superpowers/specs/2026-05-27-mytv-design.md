# MyTV — Design Spec
_2026-05-27_

## Overview

MyTV is a personal web application that repackages live internet streams and VOD content into a familiar cable TV experience. Users browse channels through an Electronic Program Guide (EPG) grid organized by category, select a channel, and watch it in a built-in player — no scrolling through YouTube, no playlist management at watch time.

---

## Goals

- Display live streams and looping VOD playlists in a unified EPG grid
- Feel like a FAST TV / DVR channel guide (categories, 24h window, "LIVE" badges)
- Work on desktop, mobile, and any browser without installing an app
- Admin interface to manage channels, sources, and playlists
- Discovery tools to find and add new streams with minimal effort

---

## Architecture

```
Browser (HTMX + Askama templates + hls.js)
        │ HTTP
Axum (Rust) server
  ├── EPG renderer    (/guide)
  ├── Player API      (/channel/:id/tune, /channel/:id/next)
  ├── Admin UI        (/admin)
  └── Discovery tools (/admin/discover)
        │
SQLite (via sqlx)
  channels · sources · playlist_items
        │ (external calls)
  yt-dlp (URL resolution)
  YouTube Data API v3 (discovery)
  iptv-org M3U lists (discovery)
```

**Key technology choices:**
- **Axum** — async Rust web framework
- **SQLite via sqlx** — no separate DB server; file lives alongside the app
- **Askama** — type-safe, compiled HTML templates
- **HTMX** — dynamic UI interactions without writing JavaScript
- **hls.js** (CDN) — HLS stream playback in the browser
- **yt-dlp** — resolves YouTube and other platform URLs to direct playable URLs at runtime

---

## Data Model

### `channels`
| field | type | notes |
|---|---|---|
| id | INTEGER PK | |
| name | TEXT | display name |
| category | TEXT | news, sports, ai, etc. |
| logo_url | TEXT | |
| type | TEXT | `live` or `vod_loop` |
| sort_order | INTEGER | position in guide |
| loop_anchor | DATETIME | VOD loop channels only — epoch start for position math |

### `sources`
Live channels only. Multiple rows per channel, tried in priority order on failover.

| field | type | notes |
|---|---|---|
| id | INTEGER PK | |
| channel_id | INTEGER FK | |
| kind | TEXT | `youtube_live`, `hls`, `iptv` |
| url | TEXT | stream URL or YouTube channel/video ID |
| priority | INTEGER | 1 = primary, 2 = first backup, etc. |
| is_active | BOOLEAN | |

### `playlist_items`
VOD loop channels only.

| field | type | notes |
|---|---|---|
| id | INTEGER PK | |
| channel_id | INTEGER FK | |
| title | TEXT | |
| url | TEXT | YouTube URL, direct video URL, etc. |
| duration_secs | INTEGER | fetched once via yt-dlp at add time |
| sort_order | INTEGER | playback sequence |

---

## Channel Types

### Live Channels
- Always-on streams with one or more source URLs ranked by priority
- EPG shows a single continuous "LIVE" block — no time slots
- On tune-in, server resolves the primary source URL (via yt-dlp if needed) and returns it to the player
- On player error, server tries the next priority source transparently

### VOD Loop Channels
- A playlist of assets (external URLs) that loops continuously, like a FAST channel
- No local video storage — only URLs and durations are stored
- Position computed at request time: `elapsed = (now - loop_anchor) % total_playlist_duration`; server walks the playlist to find the current asset and offset
- EPG shows the computed schedule for the next 24h (what's "on air" and coming up)
- On tune-in, server returns `{ url, start_offset_seconds }` for the current asset
- When an asset ends, the player calls `/channel/:id/next` to get the next URL

---

## EPG Grid UI

The main view at `/guide`:

```
┌─────────────────────────────────────────────────────────────┐
│  MyTV            [All] [News] [Sports] [AI]   ← category tabs│
├──────────┬─────────────┬─────────────┬─────────────┬────────┤
│          │   12:00     │   13:00     │   14:00     │ 15:00  │
├──────────┼─────────────┼─────────────┼─────────────┼────────┤
│ CNN Intl │ ● LIVE ━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━ │
├──────────┼─────────────┼─────────────┴─────────────┴────────┤
│ Al Jazeer│ ● LIVE ━━━━━│  Inside Story  │  The Stream       │
├──────────┼─────────────┴────────────────────────────────────┤
│ AI Daily │  Lex Fridman #421        │ ● LIVE ━━━━━━━━━━━━━━ │
└──────────┴────────────────────────────────────────────────────┘
```

- Red "now" line marks current time, always visible on load
- 24h window: ±12h from current time, scrollable horizontally in 2h steps (HTMX)
- Category tabs filter channel rows (HTMX swap, no page reload)
- Clicking any program row loads the player panel and begins playback
- "LIVE" badge on currently-live programs (live channels and current VOD asset)
- Mobile: collapses to a vertical channel list showing current program inline

Navigation is EPG-only — channel switching always goes through the guide grid.

---

## Player

A panel above the EPG grid that appears when a channel is selected.

- Always uses `hls.js` — the player knows nothing about content type (live vs. VOD)
- On tune-in: calls `/channel/:id/tune` → receives `{ hls_url, start_offset_seconds }`
- When asset ends: calls `/channel/:id/next` → receives next `{ hls_url, start_offset_seconds }`
- ~20 lines of JavaScript total for the tune + next-asset handoff logic
- No YouTube iframe API — all YouTube content is resolved to direct HLS URLs by yt-dlp server-side

---

## Admin Interface

Protected by a single password set via environment variable. Server-rendered pages at `/admin`.

### Channels
Create, edit, delete channels. Set name, category, type, logo, sort order.

### Sources (live channels)
Add/remove/reorder source URLs per channel. Set priority. "Test" button checks if the source is currently reachable.

### Playlist (VOD loop channels)
Add/remove/reorder playlist items. Set loop anchor. Preview the computed 24h schedule.
Duration is fetched automatically via yt-dlp when an asset is added.

### Discover
Three tools to find and add content:

1. **YouTube Search** — keyword or channel URL → YouTube Data API v3 → results list → one-click add to channel
2. **IPTV-org Import** — fetch public M3U playlist → filter by category/country → one-click add
3. **Manual entry** — paste any HLS/RTMP/YouTube URL directly

All three write to the same `sources` or `playlist_items` tables.

---

## Stream Resolution

`yt-dlp` is invoked server-side (as a subprocess) to:
- Resolve YouTube live stream URLs to direct HLS manifests at tune-in time
- Resolve YouTube VOD URLs to direct video URLs at tune-in time
- Fetch video duration when a VOD asset is added to a playlist

Direct HLS/IPTV URLs are used as-is without yt-dlp.

---

## Error Handling

- **Live channel source failure**: server tries next priority source; if all fail, player receives an error response and displays a "channel unavailable" message
- **VOD asset resolution failure**: skip to next playlist item, log the error
- **yt-dlp failure**: return error to player, admin sees a warning on the channel
- **SQLite**: single-writer, personal use — no concurrent write concerns

---

## Deployment

- Single Rust binary + SQLite file + `.env` for configuration
- Environment variables: `DATABASE_URL`, `ADMIN_PASSWORD`, `YOUTUBE_API_KEY`, `PORT`
- Runs on any machine with `yt-dlp` installed alongside the binary
- No Docker required, though a simple Dockerfile can be provided
