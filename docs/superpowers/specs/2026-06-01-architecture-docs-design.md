---
name: architecture-docs-design
description: Design spec for Markdown + Mermaid architecture documentation in docs/architecture/
metadata:
  type: project
---

# Architecture Docs — Design Spec

## Goal

Five Markdown files in `docs/architecture/`, each containing a Mermaid diagram paired with prose explaining key concepts and non-obvious behavior. Renders natively on GitHub. No build tooling required.

## Audience

Both the developer returning after a break and new contributors or forkers.

## File Structure

```
docs/architecture/
  request-route-map.md
  health-checker.md
  tune-flow.md
  ytdlp-resolution.md
  database-er.md
```

Each file follows the same shape:
1. Title + one-sentence summary
2. Mermaid diagram
3. Prose: key concepts, non-obvious behavior, edge cases

---

## File 1: `request-route-map.md`

**Diagram type:** `flowchart LR`

**What it shows:**
- Incoming request hits the outermost `.layer()` middleware: `redirect_trailing_slash`
- If path ends with `/` (and is not `/`), 308 redirect fires — auth never runs for `/admin/`
- Request then matches routes: public routes (`/`, `/health`, `/guide`, `/guide/partial`, `/channel/:id/tune`, `/channel/:id/next`, `/stream-proxy`) or the nested `/admin` sub-router
- Admin sub-router has `basic_auth` as a `.route_layer()` — fires only on matched admin routes

**Route inventory shown in diagram:**

| Method | Path | Handler |
|--------|------|---------|
| GET | `/` | redirect → `/guide` |
| GET | `/health` | `health_check` |
| GET | `/guide` | `guide_page` |
| GET | `/guide/partial` | `guide_partial` (HTMX) |
| GET | `/channel/:id/tune` | `tune` |
| GET | `/channel/:id/next` | `next` |
| GET | `/stream-proxy` | `stream_proxy` |
| GET/POST | `/admin/channels` | `channel_list` / `channel_create` |
| GET | `/admin/channels/new` | `channel_new_form` |
| GET/POST | `/admin/channels/:id` | `channel_detail` / `channel_update` |
| GET | `/admin/channels/:id/edit` | `channel_edit_form` |
| POST | `/admin/channels/:id/delete` | `channel_delete` |
| POST | `/admin/channels/:id/sources` | `source_create` |
| POST | `/admin/sources/:id/delete` | `source_delete` |
| POST | `/admin/sources/:id/toggle` | `source_toggle` |
| POST | `/admin/sources/:id/test` | `source_test` |
| POST | `/admin/channels/:id/playlist` | `playlist_item_create` |
| POST | `/admin/playlist/:id/delete` | `playlist_item_delete` |
| GET | `/admin/discover` | `discover_page` |
| POST | `/admin/discover/add-form` | `discover_add_form` |
| POST | `/admin/discover/add` | `discover_add` |
| POST | `/admin/discover/m3u/search` | `discover_m3u_search` |
| POST | `/admin/discover/youtube/search` | `discover_youtube_search` |
| POST | `/admin/discover/manual/resolve` | `discover_manual_resolve` |

**Prose notes:**
- Why `redirect_trailing_slash` is outermost: it must fire before route matching so `/admin/` returns 308 before auth middleware sees the request. Consequence: `GET /admin/` bypasses auth.
- Auth middleware uses `.route_layer()` scoped to the admin sub-router only.

---

## File 2: `health-checker.md`

**Diagram types:** `flowchart TD` for the tick loop + `stateDiagram-v2` for source state machine

**Tick loop diagram shows:**
- `start()` spawns a detached Tokio task
- Consumes first immediate tick on startup (no check at boot)
- 15-minute interval with `MissedTickBehavior::Skip` (slow checks don't pile up)
- On each tick: fetch all sources → for each source: HTTP check → update DB

**HTTP check logic:**
- `youtube_live` sources: HTTP 200/3xx is sufficient (yt-dlp too slow for background)
- `hls` / `iptv` sources: reads one chunk to verify the stream actually delivers bytes
- Timeout: 5 seconds

**Source state machine diagram shows:**

```
Active ──[check fails, failures < 3]──▶ Active (failures++)
Active ──[check fails, failures = 3]──▶ Disabled (auto)
Active ──[check succeeds]────────────▶ Active (failures = 0)
Disabled ──[check runs]──────────────▶ Disabled (no state change, set_inactive guard)
Disabled ──[admin toggles on]────────▶ Active (failures unchanged)
```

**Prose notes:**
- `FAILURE_THRESHOLD = 3`: three consecutive failures trigger auto-disable.
- Once disabled, the health checker does not auto-re-enable. The guard `src.is_active` in `process_result` prevents the disable flag from firing again on already-inactive sources, but successful checks only reset `consecutive_failures` — they do not set `is_active = 1`. Re-enabling a disabled source requires manual admin action (toggle button).
- Why `MissedTickBehavior::Skip`: if a check round takes longer than 15 minutes (e.g., many sources all timing out), missed ticks are dropped rather than catching up all at once.

---

## File 3: `tune-flow.md`

**Diagram type:** `flowchart TD`

**Top-level flow:**
- Fetch channel by ID → 404 if not found
- Branch on `channel_type`: `live` → `tune_live`, `vod_loop` → `tune_vod_at`
- Both paths return `Json<TuneResponse> { url, start_offset_secs }` on success or HTTP 503

**Live branch:**
- Fetch active sources ordered by priority
- Iterate: attempt `resolve_url()` on each source URL
- First success → return `{ url, start_offset_secs: 0 }`
- All fail or no active sources → 503

**VOD branch (subsection):**
- Require `loop_anchor` on channel → 500 if missing
- Fetch playlist items ordered by `sort_order`
- Empty playlist → 503
- Call `current_position(items, now_secs, anchor_secs)`:
  - `total = sum of all item durations`
  - `elapsed = (now_secs - anchor_secs).rem_euclid(total)` — Euclidean modulo handles anchor in the future correctly
  - Walk items accumulating durations until `elapsed < acc`, compute offset within that item
- Attempt `resolve_url()` on the selected item's URL → return `{ url, start_offset_secs: offset }`
- Resolver failure → 503

**`/next` endpoint (same diagram, separate branch):**
- Live: same as `tune_live` but filters out `?failed_url=<url>` from the source list — the player passes this when a URL fails to play in the browser
- VOD: advances `(current_idx + 1) % len` ignoring the `failed_url` parameter; `start_offset_secs` is always 0

**Prose notes:**
- `failed_url` is the raw source URL (before resolution), not the resolved playable URL.
- Fallback is one level deep: `/next` skips the one named failed URL. If no other active source resolves, 503 is returned.
- VOD `start_offset_secs` lets the video player seek to the correct mid-episode position, so the channel behaves like a broadcast schedule.

---

## File 4: `ytdlp-resolution.md`

**Diagram type:** `flowchart TD`

**Flow:**
- Input URL → validate scheme (`http://` or `https://`) → error if invalid
- `needs_resolution()`: returns true for `youtube.com`, `youtu.be`, `twitch.tv`
- Not needed → return URL unchanged
- Needed → spawn `yt-dlp -g --no-playlist -- <url>` with 30s timeout
- Timeout → error
- Non-zero exit → error (stderr captured)
- Empty stdout → error
- Success → return first line of stdout (the playable HLS URL)

**`fetch_duration_secs` (admin-time only, shown as separate flow):**
- Same URL validation
- Spawns `yt-dlp --print duration --no-playlist -- <url>`
- Parses stdout as `f64`, validates finite and positive, returns `i64` seconds
- Called once when an admin adds a VOD item so duration is stored in the DB

**Prose notes:**
- yt-dlp is optional: if not installed, YouTube/Twitch sources will fail resolution and the caller falls back to the next source (or 503).
- The 30s timeout is per-URL. For live channels with multiple YouTube sources, resolution attempts are sequential — total wait could be up to 30s × N sources before 503.
- `needs_resolution` is URL-pattern-based (no HTTP probe). Vimeo and other platforms are not currently supported — they pass through unchanged and typically fail at the player.

---

## File 5: `database-er.md`

**Diagram type:** `erDiagram`

**Entities and fields:**

`channels`:
- `id` PK
- `name`, `category`, `logo_url` (nullable)
- `type` (live | vod_loop)
- `sort_order`, `loop_anchor` (nullable, required for vod_loop)

`sources`:
- `id` PK, `channel_id` FK → channels (ON DELETE CASCADE)
- `kind` (youtube_live | hls | iptv)
- `url`, `priority`, `is_active`
- `last_checked_at`, `last_status` (ok | error), `consecutive_failures`, `failure_reason`

`playlist_items`:
- `id` PK, `channel_id` FK → channels (ON DELETE CASCADE)
- `title`, `url`, `duration_secs`, `sort_order`

**Relationships:**
- `channels` 1..N `sources` (live channels use sources; vod_loop channels may also have sources but primarily use playlist_items)
- `channels` 1..N `playlist_items`

**Prose notes:**
- `loop_anchor` on a `vod_loop` channel is a fixed UTC timestamp used as the epoch for computing current playlist position. It never changes after channel creation.
- `ON DELETE CASCADE` on both child tables: deleting a channel removes all its sources and playlist items.
- Health fields (`last_checked_at`, `last_status`, `consecutive_failures`, `failure_reason`) were added in migration `002_source_health.sql` and are only written by the background health checker.
