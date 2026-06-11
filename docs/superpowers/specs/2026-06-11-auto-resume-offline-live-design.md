# Auto-resume offline live channels (idea #38)

**Date:** 2026-06-11
**Status:** Designed

## Problem

When a YouTube live source is offline at tune time (yt-dlp: "not currently
live"), `next_live` exhausts its sources and returns a hard `503`. The player
shows the error screen, so a channel that is merely between broadcasts looks
broken. We want a "waiting for stream…" state that auto-retries and resumes
playback when the channel returns, and that records a real "offline" signal
when it gives up.

This is the seam already marked in `next_live` (`src/routes/player.rs:143`),
and pairs with the live-status visibility work
(`docs/superpowers/specs/2026-06-10-live-status-visibility-design.md`).

## Decisions

- **Trigger scope:** `Offline` and `Upcoming` both enter the waiting state and
  auto-retry. Genuine failures (bad URL, network, unknown errors) still fail.
- **Retry cadence:** client-side backoff `[15, 30, 60, 120]` seconds, then give
  up. Giving up is an *offline* state, not the error screen.
- **Offline state persistence:** reuse existing source health columns (no new
  schema). The give-up shows as a red health dot in the guide like any other
  down source.
- **Health-checker coupling:** the 15-min background checker becomes
  liveness-aware for `youtube_live` so the offline badge reflects real liveness
  and re-enables only when the stream truly returns — rather than blindly
  re-enabling on an HTTP-200 of the watch page.
- **Architecture (Approach A):** surface live status through the existing tune
  path; no new endpoint. The active poll and the background checker share one
  liveness signal (the yt-dlp live-status probe).

## Design

### 1. Resolver — surface offline/upcoming instead of bailing

`resolver::resolve_url_with_status` currently `bail!`s whenever yt-dlp exits
non-zero (`resolver.rs:285`). Change the failure branch to interpret the status
via the existing `interpret_live_status(false, stdout, stderr)`:

- `LiveStatus::Offline` / `LiveStatus::Upcoming(..)` → `Ok((String::new(), status))`
  — a *known* non-playable state (empty URL = "nothing to play, but we know
  why").
- anything else → keep `bail!` (genuine error).

`resolve_url` (the `.0`-only wrapper used by admin/manual flows) maps an empty
URL back to an `Err` so its callers keep their existing "no URL = error"
contract.

**Contract note:** `resolve_url_with_status` may now return `Ok` with an empty
URL; the only legitimate empty-URL statuses are `Offline` and `Upcoming`.

### 2. `next_live` — a 4th outcome + health feed

Extract a pure, tested classifier (sibling of the existing `is_ended_live`):

```
enum LiveOutcome { Play, Ended, Waiting, }   // Fail is "none of these"
fn classify_live_outcome(url: &str, status: LiveStatus) -> Option<LiveOutcome>
```

Per-source handling in the loop:

| Probe result | Outcome | Action |
|---|---|---|
| playable URL, not ended | Play | `record_source_liveness(ok=true)`; return tune response |
| `WasLive`/`PostLive`, or `force_finished/1` in URL | Ended | existing VOD conversion + `tune_response_ended` |
| `Ok("", Offline)` | Waiting | `record_source_liveness(ok=false)`; continue loop |
| `Ok("", Upcoming)` | Waiting | no health write (scheduled ≠ broken); continue loop |
| `Err(_)` | — | `tracing::warn!` + continue (no health write: may be load-shed `Busy`) |

After the loop:

- if no source was playable but **≥1 source was `Offline`/`Upcoming`** →
  `Ok(tune_response_waiting(ch))` (HTTP 200, mirrors the `ended` pattern).
- otherwise → `Err(StatusCode::SERVICE_UNAVAILABLE)` (unchanged).

Health is fed through a new shared helper:

```
// in health.rs — wraps process_result + update_health with manage_lifecycle = true
pub async fn record_source_liveness(pool, src: &Source, ok: bool) -> ...
```

Because `FAILURE_THRESHOLD == 3`, three consecutive offline probes auto-disable
the source — which aligns with the client's 4-step backoff giving up. Live and
Upcoming results reset the failure count (Upcoming via ok=true so it stays
active for the next poll).

**Non-resolution (HLS) sources are unaffected:** `resolve_url_with_status`
returns HLS/IPTV URLs unchanged with `Unknown` status and no yt-dlp spawn, so
they classify as `Play`. The waiting state only ever triggers for
youtube/twitch sources that probe `Offline`/`Upcoming`. HLS liveness continues
to be covered by the background HTTP checker.

### 3. Health checker — liveness-aware for `youtube_live`

`do_http_check` (`health.rs:321`) returns `(true, None)` for `youtube_live`
unconditionally today. After the existing HTTP-200/redirect check, run
`resolver::cached_live_status(cache, url)` and map via a pure, tested fn:

| LiveStatus | (ok, reason) |
|---|---|
| `Live` | `(true, None)` |
| `Upcoming(..)` | `(true, Some("upcoming"))` — no penalty, stays active |
| `Unknown` | `(true, None)` — load-shed / extractor gap never penalizes |
| `Offline` / `NotLive` | `(false, Some("not currently live"))` |
| `WasLive` / `PostLive` | `(false, Some("broadcast ended"))` |

This requires threading the `LiveStatusCache` into `check_source` (and the
`probe_source` variant). The sweep now spawns yt-dlp per `youtube_live` source,
but every probe goes through `resolver::run_under_cap` (2-permit cap, ~146 MB
bound on the 256 MB VM) and `cached_live_status` (60 s TTL, 10 s for `Unknown`),
so a sweep serializes rather than fanning out. `Unknown` results from load-shed
never count as failures, so a saturated cap degrades to "no change" rather than
false-disabling.

### 4. Frontend — waiting overlay + backoff

- `TuneResponse` gains `waiting: bool`; add a `tune_response_waiting(ch)` builder
  alongside `tune_response_ended` (empty URL, `waiting: true`, all other flags
  false).
- New `#player-waiting` overlay ("Waiting for stream…") in `templates/guide.html`
  and CSS in `templates/base.html`, styled like the existing `#player-ended`.
- `applyTuneResponse(d)`: `if (d.waiting) { enterWaitingState(); return; }`
  before the existing `ended`/url branches.
- `enterWaitingState()`:
  - show the `#player-waiting` overlay;
  - schedule a re-poll of `/channel/:id/tune` on the backoff schedule
    `[15, 30, 60, 120]` seconds, guarded by a `waitingGen` generation counter
    that cancels stale timers on manual tune/navigation (mirrors
    `endedAdvanceGen`);
  - a poll returning a URL → hide overlay, `_loadSource(...)`, reset state;
  - a poll still `waiting` → advance to the next backoff step;
  - after the final (120 s) step still `waiting` → stop polling, leave the
    offline overlay shown (source health already reflects offline server-side);
  - a `503`/network error → `showPlayerError()`.
- `tune()` (manual) bumps `waitingGen` and hides `#player-waiting`, same as it
  already does for `#player-ended`.

### 5. Testing

- **Pure fns (no network):**
  - resolver offline/upcoming interpretation on the failure branch;
  - `classify_live_outcome(url, status)` truth table;
  - live-status → `(ok, reason)` health mapping.
- `interpret_live_status` and `process_result` already have unit tests; extend
  where the new mapping needs coverage.
- yt-dlp/network-dependent paths stay in the existing `#[ignore]` tier; the
  decision logic is fully covered by the extracted pure fns.
- Optionally assert `TuneResponse` serializes the new `waiting` field.

### Migrations

None. Reuses existing `sources` health columns
(`is_active`, `consecutive_failures`, `failure_reason`, `last_checked_at`).

## Files touched

- `src/media/resolver.rs` — offline/upcoming on the failure branch; `resolve_url` empty-URL guard.
- `src/routes/player.rs` — `classify_live_outcome`, waiting outcome in `next_live`, `tune_response_waiting`, `waiting` field.
- `src/health.rs` — liveness-aware `do_http_check` for `youtube_live`, `record_source_liveness` helper, `LiveStatusCache` threading.
- `templates/guide.html`, `templates/base.html` — waiting overlay + backoff JS.

## Out of scope

- Countdown UI for `Upcoming` scheduled start times (cadence is uniform backoff).
- Persisting offline state at the channel level (no new schema).
- Server-side long-poll/SSE (Approach C, rejected as overkill for single-user).
