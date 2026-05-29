# Source Health Monitoring — Design Spec

**Date:** 2026-05-29
**Scope:** A background health checker that periodically tests every source, flags failures, and auto-disables sources after 3 consecutive failures. Results surfaced in the admin panel and EPG guide.

---

## Problem

Sources can go stale silently — a broken HLS URL or a dead IPTV stream is only discovered when a viewer tries to tune in and gets a 503. The app has priority-based fallback at tune time, but no proactive detection. This feature adds a 15-minute background check so broken sources are flagged before anyone watches.

---

## Architecture

A new `src/health.rs` module owns all checking logic. At startup, `health::start(pool, client)` is called from `main.rs` and spawns a background Tokio task. That task runs a `tokio::time::interval` loop every 15 minutes, fetching all sources and checking them concurrently. Results are written back to SQLite. No new processes, no new dependencies beyond what already exists.

---

## Data Model

### Migration: `migrations/002_source_health.sql`

Adds four columns to the `sources` table:

```sql
ALTER TABLE sources ADD COLUMN last_checked_at  INTEGER;
ALTER TABLE sources ADD COLUMN last_status       TEXT CHECK(last_status IN ('ok', 'error'));
ALTER TABLE sources ADD COLUMN consecutive_failures INTEGER NOT NULL DEFAULT 0;
ALTER TABLE sources ADD COLUMN failure_reason    TEXT;
```

### `Source` struct (`src/model/source.rs`)

Four new fields matching the columns above. Two new methods:

- `list_all(pool) -> Vec<Source>` — fetches every source across all channels (used by the background checker)
- `update_health(pool, id, status, reason, consecutive_failures, set_inactive: bool)` — atomically writes health fields; sets `is_active = 0` when `set_inactive` is true

---

## Health Checker (`src/health.rs`)

### Startup

```rust
pub fn start(pool: SqlitePool, client: reqwest::Client)
```

Called once from `main.rs` after pool and client are initialised. Spawns a detached Tokio task. No handle is stored — the task runs for the lifetime of the process.

### Check loop

Every 15 minutes: fetch all sources via `Source::list_all()`, then check each one concurrently using `futures::future::join_all`. After all checks complete, write results to the DB.

### Per-source logic

**HLS and IPTV sources:**
1. HTTP GET the URL with a 5-second timeout.
2. If response status is 2xx or 3xx, read up to 8 KB of body.
3. If at least 1 byte received → **ok**.
4. Any network error, non-2xx/3xx status, or zero bytes → **error**.

**YouTube live sources:**
- HTTP GET only (no yt-dlp — too slow and rate-limited for background use).
- Status 200 → **ok**. Anything else → **error**.
- No chunk read for this kind.

### After each check

| Result | Action |
|--------|--------|
| ok | Reset `consecutive_failures` to 0, set `last_status = 'ok'`, clear `failure_reason`, update `last_checked_at` |
| error | Increment `consecutive_failures`, set `last_status = 'error'`, write `failure_reason`, update `last_checked_at` |
| error AND `consecutive_failures >= 3` AND source is active | Additionally set `is_active = 0` |

Auto-disable is one-way: health checks never re-enable a source. Re-enabling is always a manual admin action.

---

## UI

### Admin panel — source health badge

In the channel detail view, each source entry in the sources list gains an inline badge:

- `●` green — `last_status = 'ok'`
- `●` red — `last_status = 'error'`, failure reason shown in small text below
- `○` gray — `last_status` is NULL (never checked)
- `[auto-disabled]` label — shown when `is_active = 0` and `consecutive_failures >= 3`

No new page. The badge is added to the existing sources partial template.

### EPG guide — broken channel warning

`ChannelRow` in `src/routes/guide.rs` gains a boolean field `all_sources_down`.

In `build_guide_data`:
1. Run one extra query: `SELECT DISTINCT channel_id FROM sources WHERE is_active = 1` to get the set of channel IDs with at least one live source.
2. For each live-kind channel not in that set (and that has at least one source), set `all_sources_down = true`.
3. VOD channels (no sources) are never flagged.

In `templates/partials/epg_content.html`, the channel name cell prepends `⚠` when `all_sources_down` is true.

---

## What is not in scope

- Auto-re-enabling sources after recovery
- Health history / time-series log
- Notifications (email, webhook) on failure
- yt-dlp resolution as part of the health check for YouTube sources
- Configurable check interval or failure threshold (hardcoded: 15 min, 3 failures)
