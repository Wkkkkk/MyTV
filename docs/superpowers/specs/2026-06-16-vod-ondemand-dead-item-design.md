# Idea #53 — VOD-on-demand dead-item handling — Design

> Status: design (brainstormed 2026-06-16). Next: implementation plan via writing-plans.

## Problem

When a self-hosted MP4 item on a `vod_on_demand` channel is deleted from R2 (or
otherwise goes unreachable), two things fail quietly today:

1. **No reporting** — the player's `item` handler returns a bare
   `503 SERVICE_UNAVAILABLE` with no detail (`src/routes/player.rs:111`, the
   `resolver::resolve_url` error arm), so the viewer sees a generic stall.
2. **No cleanup** — the health checker accumulates `consecutive_failures` on
   playlist items but **never acts on it**: `check_playlist_item` calls
   `update_health` with `is_active = None` deliberately (manual intent was the
   source of truth). So a dead R2 item fails forever.

Sources do not have this problem in the same way: after idea #48,
`list_tunable_for_channel` filters out observed-down sources via the pure
`is_observed_down` predicate. Playlist items have **no equivalent filter** —
`list_active_for_channel` returns every active item regardless of health.

**Key asymmetry that shapes the design:** a Down *live source* can recover (it
may just be offline and will return), but a VOD item whose R2 object is deleted
will **never** recover. That is why items get *disabled* (an action), whereas
sources are merely *filtered* (a query-time view).

## Decisions

| Question | Decision |
|----------|----------|
| Action on a Down item | **Auto-disable** (`is_active = 0`). Reversible — admin re-enables manually. No hard-delete, no auto-re-enable. |
| Does a failed *tune* count toward the threshold? | **Yes** — alongside the 15-min health loop. But a single model-layer fn owns the disable rule so two writers stay safe. |
| Player UX on a dead item | **Report + auto-skip** — toast "*{title}* unavailable — skipping…" and advance to the next playable item. |
| Scope of auto-disable | **All playlist items** (`vod_loop` + `vod_on_demand`) — it is per-item health hygiene. The report+skip player UX is on-demand-specific. |

This **intentionally diverges** from source behavior: health *will* mutate
`is_active` for items. The divergence is justified by the asymmetry above and is
documented in the code.

## Architecture

### 1. Pure predicate — `playlist_item::is_dead`

```rust
/// An on-demand/VOD playlist item is "dead" when its last health probe errored
/// and it has failed at least FAILURE_THRESHOLD consecutive times. Unlike
/// `source::is_observed_down`, there is no `youtube_live` exemption: a playlist
/// item is never a live broadcast, so an errored item past threshold is always
/// dead (a deleted R2 object never recovers on its own).
pub fn is_dead(last_status: Option<&str>, consecutive_failures: i64) -> bool {
    last_status == Some("error")
        && consecutive_failures >= crate::model::source::FAILURE_THRESHOLD
}
```

Reuses the shared `source::FAILURE_THRESHOLD = 3`. Deliberately **not**
`is_observed_down`: that predicate exempts `youtube_live`, which would wrongly
shield a dead youtube-VOD item (whose detected kind may classify as live).
Items are never live, so the plain threshold rule is both simpler and correct.

### 2. The single disable owner — `playlist_item::apply_health_result`

```rust
/// Records one health-probe result against an item and applies the auto-disable
/// rule. This is the ONLY place the disable decision lives, so the health loop
/// and the interactive tune path (two writers) stay consistent.
/// - ok=true  resets failures (status "ok"); never re-enables (manual intent).
/// - ok=false counts a failure (status "error"); disables once `is_dead`.
pub async fn apply_health_result(
    pool: &SqlitePool,
    item: &PlaylistItem,
    ok: bool,
    reason: Option<&str>,
) -> bool
```

Internals:
- `new_failures = if ok { 0 } else { item.consecutive_failures + 1 }`
- `status = if ok { "ok" } else { "error" }`
- `is_active_arg = if is_dead(Some(status), new_failures) { Some(false) } else { None }`
- calls `update_health(pool, item.id, status, reason, new_failures, is_active_arg)`
- returns `ok`

On recovery (`ok=true`) it resets failures but passes `is_active = None`, so a
re-enabled-then-recovered item is not flipped further by health — re-enabling
stays a manual admin action. A manually re-enabled item that is *still* dead is
re-disabled on the next check, which is the desired behaviour.

### 3. Two call sites feed `apply_health_result`

- **`health::check_playlist_item`** (the 15-min background loop,
  `src/health.rs:138`) — route its probe result through `apply_health_result`
  instead of the current closure that passes `is_active = None`.
- **`player::item` resolver-error arm** (`src/routes/player.rs:111`) — call
  `apply_health_result(pool, &item, false, Some(reason))` before returning the
  503. The item is still active and found here; once it crosses the threshold
  and is disabled, subsequent tunes fall through to the existing
  `422 UNPROCESSABLE_ENTITY` arm (item not in the active list), so a disabled
  item is never double-counted.

### 4. Player frontend — auto-skip (`templates/base.html`, `odPlayIndex` ~L546)

The on-demand player loads its playlist from `/channel/:id/playlist`
(`list_active_for_channel`), so a disabled item is already absent from `odItems`
on a fresh tune — steady-state skipping is free. The frontend change covers the
**in-session edge**: an item that dies while the channel is open (its `odItems`
snapshot is already loaded), or one that fails on its 1st/2nd failure before
crossing the disable threshold.

In `odPlayIndex`, the failure path (`!r.ok` / `.catch`) currently calls
`showPlayerError()`. Change it to:
- toast "*{odItems[i].title}* unavailable — skipping…" (the client already knows
  the title locally — no new server response body required);
- call `odPlayIndex(i + 1, 0)` — **forward only, no wrap**;
- if there is no next item (`i + 1 >= odItems.length`), fall back to
  `showPlayerError()`.

Forward-only advance guarantees termination even if every item is dead.

### 5. Admin surfacing (`templates/admin/partials/playlist_item_row.html`)

The row already shows a status glyph and a `failure_reason` line, but the reason
is gated behind `{% if item.is_active %}` (line 10), so an auto-disabled item
hides *why* it was disabled and looks identical to a manual disable. Change:
show the reason whenever it is present; when `!is_active`, prefix it
"auto-disabled — {reason}" to distinguish it from a manual toggle. The row view
model already carries `failure_reason`, so this is template-only.

## Data flow

```
background loop (every 15 min)          interactive tune (viewer clicks item)
        │                                         │
check_playlist_item                      player::item → resolver::resolve_url
        │ (ok / error + reason)                   │ Err(e)
        └──────────────┬──────────────────────────┘
                       ▼
        playlist_item::apply_health_result(item, ok, reason)
                       │  new_failures, status
                       ▼
                 is_dead(status, new_failures)?
                  ├─ yes → update_health(.., is_active = Some(false))   ← disabled
                  └─ no  → update_health(.., is_active = None)
                       │
        ┌──────────────┴───────────────────────────┐
        ▼                                           ▼
  /playlist & /item filter is_active=1        admin row shows
  → disabled item absent from odItems         "auto-disabled — {reason}"
  → never played (steady state)
  + player auto-skips an item that dies mid-session
```

## Testing

- **Unit** — `is_dead` truth table: `ok`/`null` status never dead; errored below
  threshold not dead; errored at and above threshold dead.
- **Model** — `apply_health_result`: below threshold keeps `is_active = true`;
  reaching threshold flips it to `false`; a later `ok` resets failures but leaves
  `is_active = false` (no auto-re-enable).
- **Integration** (`tests/http.rs`) — tuning an unresolvable on-demand item
  returns 503 and increments failures; after `FAILURE_THRESHOLD` tunes the item
  is disabled and absent from `/channel/:id/playlist`. Requires a dead-URL
  on-demand item in `tests/fixtures/seed.sql` (new channel + item).

## Out of scope (YAGNI)

- **No hard-delete** — disable is reversible; a transient 45-min outage must not
  permanently drop a recoverable item.
- **No structured 503 JSON body** — the player names the dead item from its local
  `odItems[i].title`; the server keeps returning status codes.
- **No auto-re-enable** — recovery is surfaced (the item's status returns to
  "ok" and the admin can re-enable), but re-enabling is a manual decision.
- **No migration** — reuses existing `consecutive_failures`, `is_active`, and
  `failure_reason` columns.
