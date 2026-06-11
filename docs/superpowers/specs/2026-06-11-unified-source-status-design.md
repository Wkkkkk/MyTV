# Unified source Status — design

**Date:** 2026-06-11
**Status:** approved, pending implementation

## Problem

Live sources carry four separate status indicators that overlap heavily:

| Concept | Meaning | Storage | Shown | Glyphs |
|---|---|---|---|---|
| **Active** | Admin intent — use this source? (but auto-flipped by health) | `sources.is_active` (persistent) | admin rows | `on` / `off` |
| **Health** | Last probe reachable? | `last_status`, `consecutive_failures`, `failure_reason` (persistent) | admin rows + guide | green ● / red ● / grey ○ |
| **Budget** | CORS: browser hits CDN direct, or must we proxy? | ephemeral CORS cache | admin rows + guide | ⚡ / ☁ / · |
| **Live** | yt-dlp `live_status` for `youtube_live` | ephemeral live cache (60s TTL) | admin rows (YouTube only) | ● / ◷ / ◉ / ○ / ▶ / ◌ / · |

Three of the four are the same axis — *"is this source usable right now?"* — seen from different angles:

- **Health is derived from Live** for `youtube_live` (`live_status_health()` maps `Offline`/`WasLive` → unhealthy), so the admin row shows the *same probe* twice.
- **Active is auto-driven by Health** (checker flips `is_active=false` after 3 failures, back on recovery), so `off` conflates *"I disabled this"* with *"the system gave up on it"* — hence the template's special `[auto-disabled]` note.

**Budget is the genuine outlier** — a different axis entirely (network cost/routing, not availability).

## Goal

Collapse the availability trio (Active + Health + Live) into a **single Status indicator** per source/item, keep **Budget** as its own separate badge. Two indicators total instead of four. Distinguish admin *intent* ("Disabled") from system *observation* ("Down"/"Offline"), and decouple the two in the data flow.

## Decisions

1. **Two axes.** One `Status` indicator + the existing Budget badge. The `on/off` *toggle button* stays in the admin actions column — it is the control that sets `is_active` (now unambiguously "admin intent").
2. **Distinct + decoupled.** `is_active` becomes pure manual intent. The health checker **stops mutating `is_active`**. Health/live drive the observed Status; `is_active` is a separate, user-owned switch.
3. **Tune skips Down at read time.** Tune gates on `is_active` AND not-currently-down (read the persisted health verdict). `is_active` is never mutated, so down sources rejoin automatically on recovery.
4. **Applies to VOD items too.** Playlist-item rows get the same unified Status vocabulary (minus the live-only states), for admin consistency.

## The Status model

A new crate-level `SourceStatus` type (sibling of `budget.rs`), computed in precedence order:

| Order | Status | Glyph | Color | Condition | Applies to |
|---|---|---|---|---|---|
| 1 | **Disabled** | ⏸ | `#888` grey | `is_active = false` | all |
| 2 | **Down** | ✕ | `#e94560` red | `last_status='error'` & `consecutive_failures >= 3` (shows `failure_reason`) | regular / VOD |
| 3 | **Live** | ● | `#4caf50` green | `LiveStatus::Live` | youtube_live |
| 3 | **OK** | ● | `#4caf50` green | `last_status='ok'` | regular / VOD |
| 3 | **Upcoming** | ◷ | `#db4` amber | `LiveStatus::Upcoming(ts)` (tooltip shows scheduled start) | youtube_live |
| 3 | **Recorded** | ⏺ | `#88f` blue | `LiveStatus::WasLive` / `PostLive` (heads-up: next tune converts the channel to VOD) | youtube_live |
| 3 | **Offline** | ○ | `#888` grey | `LiveStatus::Offline` / `NotLive` (recoverable — not currently live) | youtube_live |
| 4 | **Unchecked** | · | `#666` dark grey | never probed / `LiveStatus::Unknown` | all |

**Precedence rationale:**

- **Disabled wins** — manual intent is shown first regardless of observed reachability.
- **Down outranks live-ness** — a hard HTTP error (threshold reached) is more important to surface than a cached "Live"; it also matches the tune-path skip (a Down regular source won't be tuned, so the badge should say so). **Down is regular/VOD only:** for `youtube_live`, Status derives from the cached `LiveStatus` and never from `last_status` — so a stream recorded offline (which `live_status_health` writes as `last_status='error'`) shows ○ Offline, not ✕ Down.
- **Live and OK share green ●** — both mean "this will play"; they differ only in label/tooltip ("Currently live" vs "Reachable").

Computation signature (illustrative):

```rust
pub enum SourceStatus {
    Disabled,
    Down,        // carries failure_reason for the tooltip
    Live,
    Ok,
    Upcoming,    // carries scheduled-start ts for the tooltip
    Recorded,
    Offline,
    Unchecked,
}

// inputs: is_active, kind, last_status, consecutive_failures, failure_reason,
//         and (for youtube_live) the cached LiveStatus when available.
pub fn compute(...) -> SourceStatus { ... }

pub fn status_badge(s: &SourceStatus) -> (/*class*/ &str, /*glyph*/ &str, /*label*/ &str, /*title*/ String) { ... }
```

`status.rs` **consolidates** logic currently scattered across `routes/admin/live_status.rs::badge_parts`, `routes/guide/badges.rs::health_badge`, and the inline `on/off` markup in the source-row template.

## Decoupling the health checker (behavior change)

The background checker (`health.rs`) **no longer writes `is_active`**:

- Remove `HealthAction::Disable` / `HealthAction::Reenable` and the corresponding `is_active` mutations in `process_result`.
- It still records `last_status`, `consecutive_failures`, and `failure_reason` — these drive the Down state and the badge, and `consecutive_failures` is still counted against `FAILURE_THRESHOLD` (3).
- `record_source_liveness` (from idea #38) keeps resetting `consecutive_failures` on success but no longer re-enables.

**Consequence (a fix):** a manually-disabled source can no longer be silently auto-re-enabled on recovery — today's auto-re-enable overrides manual intent; decoupling removes that.

## Tune-path gating (two lanes)

- **Regular HLS/DASH:** a new `list_tunable` query (or equivalent) gates on
  `WHERE is_active = 1 AND NOT (last_status = 'error' AND consecutive_failures >= 3)`.
  Down sources are skipped at read time but rejoin automatically when health recovers (no `is_active` mutation).
- **youtube_live:** exempt from the down-skip — stays in rotation whenever `is_active = 1`. This is required so idea #38's resolve-time classification (Play / Ended / Waiting+backoff) still fires for offline/upcoming streams. (A `youtube_live` "offline" recorded as `last_status='error'` by `live_status_health` must NOT cause a read-time skip, or the channel would 503 instead of entering the waiting state.)

The unified Status badge presents both lanes with one vocabulary; only the *gating* differs by kind.

## Rendering

- **`status.rs`** (new): enum + `compute` + `status_badge` (above).
- **Admin source row** (`templates/admin/partials/source_row.html`): the three columns Active / Health / Live collapse to **one Status column**. Budget column unchanged. Toggle button (Enable/Disable) stays in actions.
  - Regular / VOD rows render Status **server-side inline** (instant, from persisted DB fields).
  - `youtube_live` *active* rows keep **lazy-loading** the Status via the repurposed `GET /admin/live-status?url=...` endpoint (now returns the unified Status badge), so yt-dlp never blocks page render.
  - `youtube_live` *disabled* rows render ⏸ Disabled inline — no probe.
- **VOD playlist-item row** (`templates/admin/partials/playlist_item_row.html`): same single Status column (Disabled / OK / Down / Unchecked).
- **Guide** (`templates/partials/epg_content.html`, `routes/guide/badges.rs`, `routes/guide/data.rs`): channel row shows one aggregated **Status** + Budget. Aggregate = the **most optimistic** status across the channel's sources (best-case wins), by this order (best → worst):

  `Live` = `OK` > `Upcoming` > `Recorded` > `Offline` > `Unchecked` > `Down` > `Disabled`

  So a channel with one Live source and one Down source shows ● Live; all-down → ✕ Down; all-disabled → ⏸ Disabled; no sources → · Unchecked. The guide reads **only persisted health + warm caches and never probes** (preserves today's non-blocking guide). For `youtube_live` the per-source status uses the warm `LiveStatus` cache; a cold cache yields · Unchecked (never Down).

## Data model

**No migration.** Every input already exists:

- `is_active`, `last_status`, `consecutive_failures`, `failure_reason` — persistent columns on `sources` and `playlist_items`.
- Live status — ephemeral `LiveStatusCache`.
- Budget — ephemeral `CorsCache`.

## Testing

- **Unit (`status.rs`):** `compute` precedence — Disabled beats everything; Down beats Live; each kind→status mapping; Unchecked fallback. `status_badge` glyph/class/label mapping for every variant.
- **Health checker:** rewrite the tests that asserted auto-disable / auto-re-enable (`process_result`, `record_source_liveness`, the #38 liveness tests) to assert `is_active` is now **untouched** and only health fields change.
- **Tune:** `list_tunable` skips down regular sources, keeps `youtube_live` regardless of recorded down-ness, and respects manual disable; #38 waiting/backoff still triggers for offline `youtube_live`.
- **Integration (`tests/http.rs`):** admin source row + playlist-item row render the single Status column; guide renders the aggregated Status badge; the repurposed `/admin/live-status` returns the unified Status badge.

## Documentation

- Add an architecture note describing the `SourceStatus` model and the intent-vs-observation split (e.g. `docs/architecture/source-status.md`), linked from the existing architecture docs.

## Main risk / effort

The weight is in **decoupling the health checker** — removing auto-disable touches `process_result`, `record_source_liveness`, and the #38 liveness path, and several existing tests assert the old mutation behavior and must be rewritten. The rest (`status.rs`, templates, the `list_tunable` query) is additive and mechanical.
