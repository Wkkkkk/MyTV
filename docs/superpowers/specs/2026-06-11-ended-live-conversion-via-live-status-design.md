# Ended-Live → VOD Conversion via live_status — Design

**Date:** 2026-06-11
**Idea:** docs/IDEAS.md #39 (builds on #36's conversion machinery and #35's `LiveStatus` model)
**Status:** Approved — pending implementation

## Context

The ended-live → VOD conversion (idea #36) triggers only when the resolved
manifest URL contains `force_finished/1` (`resolver::is_finished_live`). That
marker exists only in the brief *post-live* window while YouTube is still
processing the recording. Once processing finishes (yt-dlp `live_status =
was_live`), the same source resolves to a perfectly normal recording URL with
no marker — so the channel plays the recording while still typed `live` and
never converts.

Observed live: channel 10 ("Scott TRANSFER Boost!") — the admin badge correctly
shows ◉ recorded (from idea #35's state model), but tuning plays it as a live
stream.

Decision (user-confirmed): conversion triggers on **`was_live` and
`post_live`**. `not_live` (a never-live video on a live channel) keeps playing
as-is — possibly an intentional configuration. `Upcoming`/`Offline` keep
today's behavior (resolve fails → failover/503); handling them is idea #38.

## Design

### Resolver: one call returns URL + status

`resolve_url`'s yt-dlp invocation changes from

```
yt-dlp -g --no-playlist -f b[ext=mp4]/b
```

to

```
yt-dlp --print live_status --print urls --no-playlist -f b[ext=mp4]/b
```

`-g` is an alias for `--print urls`, so resolution semantics are unchanged;
stdout now carries a status line first:

```
was_live
https://rr3---sn-xyz.googlevideo.com/videoplayback?...
```

New public API in `src/media/resolver.rs`:

- `resolve_url_with_status(url) -> Result<(String, LiveStatus)>` — owns the
  invocation and parsing: line 1 → status token, line 2 → playable URL (the
  first URL line, preserving today's first-URL rule for video+audio multi-line
  output). Non-resolution URLs (plain HLS/IPTV) pass through unchanged as
  `(url, LiveStatus::Unknown)` with no yt-dlp spawn.
- `resolve_url(url) -> Result<String>` becomes a thin wrapper that drops the
  status. Its three status-indifferent call sites (two VOD tune paths in
  `routes/player.rs`, `probe_and_cache_resolved_cors` in `health.rs`) are
  untouched.

Shared mapping: the status-token → `LiveStatus` match currently inside
`interpret_live_status` is extracted into `live_status_from_str(token:
&str) -> LiveStatus` (no timestamp — `is_upcoming` maps to `Upcoming(None)`);
`interpret_live_status` delegates to it for the token half and keeps its
timestamp and stderr-fallback logic. One mapping, two consumers (badge probe +
resolver).

### Tune decision in `next_live`

A pure helper makes the decision testable without yt-dlp:

```rust
fn is_ended_live(status: LiveStatus, resolved_url: &str) -> bool {
    matches!(status, LiveStatus::WasLive | LiveStatus::PostLive)
        || resolver::is_finished_live(resolved_url)
}
```

`next_live` switches to `resolve_url_with_status` and calls the helper where it
checks `is_finished_live` today. The `force_finished/1` manifest check remains
as a fallback for extractors that don't set `live_status`. On ended: the
existing, unchanged idea-#36 machinery runs — `spawn_live_to_vod_conversion`
(detached, idempotent: watch-URL derivation, duration fetch, playlist-item
append, type flip, source deactivation) plus the `{ ended: true, url: "" }`
response that the frontend already turns into a "Stream ended — switching…"
overlay with auto-advance.

The resolve-`Err` branch is unchanged and is the documented seam for idea #38:
upcoming/offline streams fail resolution (no formats) and fall through to
failover/503 today; a future "waiting for stream" state plugs in there.

### Effect on the trigger case

First tune of channel 10 after deploy: yt-dlp reports `was_live` → response
`{ ended: true }` → background task converts the channel to `vod_loop` with the
recording as its playlist item and deactivates the sources. No manual action,
no migration.

## Error handling

- Unrecognized or `NA`/`None` status line → `LiveStatus::Unknown` → plays
  normally (exactly today's behavior).
- Fewer than two stdout lines on a successful exit → error, equivalent to
  today's empty-output bail.
- Resolve failure (offline, upcoming, network) → warn + next source / 503,
  unchanged.
- yt-dlp cap/timeout semantics (`yt_dlp_output`, 2-permit semaphore, 15 s wait
  / 30 s command timeout) unchanged — still exactly one subprocess per live
  tune.

## Testing

Unit (`src/media/resolver.rs`):
- stdout parser: two-line happy path; three-line (video+audio) takes line 2;
  `NA` status → Unknown; missing URL line → error.
- `live_status_from_str`: all five tokens + `NA`/garbage → Unknown;
  `interpret_live_status` existing table test must keep passing unchanged
  (regression on the delegation refactor).
- `#[ignore]`d yt-dlp test: `resolve_url_with_status` on the stable "Me at the
  zoo" VOD → status `NotLive`, URL starts with `http`. Pins the two-line
  output shape and `--print` ordering empirically.

Unit (`src/routes/player.rs`):
- `is_ended_live`: `WasLive`/`PostLive` → true regardless of URL;
  `Live`/`NotLive`/`Unknown` + plain URL → false; `Unknown` +
  `force_finished/1` URL → true (fallback path).

Integration (`tests/http.rs`): the existing force_finished test
(`test_tune_finished_live_returns_ended_and_no_url`) keeps passing — its seed
source is a direct HLS URL carrying the marker, no resolution involved, and the
fallback preserves that path. No new integration tests (the `was_live` path
requires a real yt-dlp subprocess; covered by the unit seam + ignored test).

## Out of scope

- Idea #38 (auto-resume waiting on `Upcoming`/`Offline`) — this design only
  documents the seam.
- Converting `not_live` sources on live channels.
- Background-sweep conversion (rejected: recurring yt-dlp load on the 256 MB
  VM; see docs/bug-logs/2026-06-10-live-status-badge-ytdlp-oom.md precedent).
- Frontend changes — the `ended` response contract is unchanged.

## Docs

- `docs/IDEAS.md` #39 struck through with a done note.
- `docs/architecture/tune-flow.md` updated with the status line in the resolve
  step and the broadened ended decision.
