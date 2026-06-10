# YouTube Discover Improvements + Live-Status State Coverage — Design

**Date:** 2026-06-10
**Idea:** docs/IDEAS.md #35 (+ state foundation for #38/#39)
**Status:** Draft — rewritten to add yt-dlp `live_status` coverage; pending review

## Context

Idea #35 listed four gaps in the YouTube Discover tab. Code inspection showed two
are already fixed:

1. ~~`source_kind` hardcoded to `youtube_live`~~ — `build_video_rows`
   (`src/routes/admin/discover/youtube.rs`) already sets `youtube_vod` when
   `liveBroadcastContent != "live"`, with a unit test.
2. ~~Live channels not findable by keyword~~ — the YouTube tab already has a
   Videos/Channels type selector backed by `fetch_youtube_channels`
   (`type=channel` search), plus a "Channel URL / @handle" resolve form.

This design covers the two remaining gaps — (3) thumbnails and (4) upcoming
streams in Discover — **plus a third part added on review**: the admin
live-status probe currently collapses YouTube's lifecycle into three states
(Live / Offline / Unknown) because it reads `yt-dlp --print is_live`. yt-dlp's
`live_status` field distinguishes six, at the same one-subprocess cost. Richer
states make the admin badge truthful for scheduled and ended streams, and are
the state foundation ideas #38 (auto-resume offline live channels) and #39
(ended-live vs. waiting-live compatibility) need.

### Decision: upcoming streams are badged and addable

Upcoming streams in Discover get an UPCOMING badge and an Add button that
creates a `youtube_live` source. This enables pre-adding scheduled events
(launches, matches) — the channel starts working the moment the stream goes
live.

### Why this is safe in the existing source lifecycle

No model or health-checker changes are needed; an upcoming stream added as a
`youtube_live` source flows through the existing state machine gracefully:

- **Health check:** the `watch?v=…` page returns HTTP 200 before the stream
  starts, and `youtube_live` health only requires a 2xx/3xx response
  (`do_http_check` in `src/health.rs` skips the body-chunk read for that kind)
  → the source stays `ok`/active and is never auto-disabled while waiting.
- **Live-status badge:** shows Upcoming (Part 2) until the stream starts.
- **Tune before start:** yt-dlp resolution fails → failover to the next source
  or 503. A "waiting for stream" UX is out of scope here (idea #38).
- **Once live:** the same source starts working — no transition needed.

---

## Part 1 — Discover: thumbnails + upcoming (YouTube Data API)

### Data flow / API

The existing `videos.list` call in `fetch_youtube_results` (already made for
durations) changes `part=contentDetails` to
`part=contentDetails,liveStreamingDetails`. From the response we build a second
map `video_id → scheduledStartTime` alongside the existing duration map. No new
API calls; `videos.list` quota cost is 1 unit regardless of parts requested.

### Row model

`YoutubeResultRow` gains three fields:

- `thumbnail_url: String` — from `snippet.thumbnails.default.url`
  (120×90 for videos, 88×88 for channels; both `build_video_rows` and
  `build_channel_rows` extract it). Empty string if absent → no `<img>`
  rendered.
- `is_upcoming: bool` — `liveBroadcastContent == "upcoming"`. `is_live` keeps
  meaning "live right now"; the two are mutually exclusive by API contract.
- `scheduled_start: String` — preformatted server-side with chrono as
  `"Jun 12 18:00 UTC"` (`%b %d %H:%M UTC` from the RFC 3339
  `liveStreamingDetails.scheduledStartTime`). Empty when absent or
  unparseable, in which case the badge renders alone.

Upcoming rows get `source_kind: "youtube_live"` so they add as a live source.

### Template (`templates/admin/partials/discover_yt_results.html`)

- New leading thumbnail column: `<img src="…" width="80" loading="lazy">`
  when `thumbnail_url` is non-empty; empty cell otherwise.
- Type column becomes three-way:
  - `is_upcoming` → amber `UPCOMING` badge, followed by `scheduled_start` when
    non-empty;
  - `is_live` → existing `LIVE` badge + the lazy live-status checker span;
  - else → `VOD` badge.
- The yt-dlp live-status probe is **skipped** for upcoming rows — the Data API
  already told us the state, so don't spend a slot on the 2-permit yt-dlp cap.
- The hidden `is_live` form field sends `true` for upcoming rows too
  (`row.is_live || row.is_upcoming`), so the add form treats them like live:
  no manual duration input, `youtube_live` source kind.
- One new `.badge-upcoming` CSS rule in `templates/admin/base.html` next to
  `badge-live` / `badge-vod`.

---

## Part 2 — Resolver: full `live_status` state model (yt-dlp)

### Probe command

`probe_live` (`src/media/resolver.rs`) changes from

```
yt-dlp --print is_live --no-playlist <url>
```

to

```
yt-dlp --print "%(live_status)s|%(release_timestamp)s" --ignore-no-formats-error --no-playlist <url>
```

- `live_status` is one of `is_live`, `is_upcoming`, `post_live`, `was_live`,
  `not_live`, or `NA`/`None` when the extractor doesn't set it (some
  non-YouTube sites).
- `release_timestamp` is the scheduled start as a unix epoch for upcoming
  streams (`NA` otherwise) — same subprocess, no extra cost.
- `--ignore-no-formats-error` is required: an upcoming stream has no formats
  yet, and without the flag yt-dlp exits non-zero ("This live event will begin
  in …") before printing. Exact behavior must be pinned empirically during
  implementation with `#[ignore]`d tests (the repo's existing pattern for
  yt-dlp/network tests).

### State model

`LiveStatus` becomes a faithful mirror (stays `Copy` — the payload is an
`Option<i64>`):

```rust
pub enum LiveStatus {
    Live,                      // live_status = is_live
    Upcoming(Option<i64>),     // is_upcoming; payload = release_timestamp (unix)
    PostLive,                  // just ended, still processing
    WasLive,                   // finished broadcast, recording available
    NotLive,                   // regular video, never was live
    Offline,                   // channel has no current/scheduled broadcast
    Unknown,                   // probe failed / extractor has no live_status
}
```

Interpretation (`interpret_live_status`, replacing `interpret_is_live`):

| Probe outcome | Status |
|---|---|
| exit 0, stdout starts `is_live` | Live |
| exit 0, stdout starts `is_upcoming` | Upcoming(parsed ts or None) |
| exit 0, stdout starts `post_live` | PostLive |
| exit 0, stdout starts `was_live` | WasLive |
| exit 0, stdout starts `not_live` | NotLive |
| exit 0, stdout `NA`/`None`/empty | Unknown |
| exit ≠ 0, stderr contains "not currently live" | Offline |
| exit ≠ 0, stderr contains "live event will begin" | Upcoming(None) — fallback if the flag doesn't suppress the error |
| any other failure / timeout | Unknown |

Cache (`cached_live_status`) is unchanged structurally: TTL stays 60 s for
determinate states and 10 s for Unknown. (`Upcoming`, `WasLive`, etc. are
determinate → 60 s.)

### Badge rendering (`src/routes/admin/live_status.rs`)

`badge_parts` maps the new states:

| Status | Symbol | Color | Label | Title |
|---|---|---|---|---|
| Live | ● | #4caf50 | live | Currently live |
| Upcoming(Some(ts)) | ◷ | #db4 | upcoming | Scheduled — starts Jun 12 18:00 UTC |
| Upcoming(None) | ◷ | #db4 | upcoming | Scheduled, start time unknown |
| PostLive | ◌ | #f77 | ended | Broadcast just ended (processing) |
| WasLive | ◌ | #f77 | ended | Finished broadcast (recording available) |
| NotLive | ▶ | #88f | vod | Regular video (never live) |
| Offline | ○ | #888 | offline | Not currently live |
| Unknown | · | #666 | ? | Live status unknown |

`Upcoming(Some(ts))` formats the timestamp with the same `%b %d %H:%M UTC`
pattern as Part 1 — extract one shared helper rather than duplicating
(chrono `DateTime::from_timestamp` for the epoch path, `parse_from_rfc3339`
for the Data-API path).

The title attribute is no longer `&'static str` (it embeds the formatted
time) — `LiveStatusBadgeTemplate.title` becomes `String`.

### Consumers and future work (out of scope, but the point of the shape)

- Idea #38 (auto-resume): tune-time "waiting for stream" can branch on
  `Upcoming`/`Offline` and use the `release_timestamp` for a countdown.
- Idea #39 (ended-vs-waiting compatibility): the ended-live→VOD conversion can
  distinguish `WasLive`/`PostLive` (convert) from `Upcoming`/`Offline` (wait)
  instead of inferring only from `force_finished/1` in the manifest.
- This spec changes **no tune-flow behavior** — only the probe, the enum, and
  the admin badge.

---

## Error handling

- Part 1: missing thumbnails, missing `liveStreamingDetails`, or unparseable
  timestamps degrade to today's rendering; YouTube API error-surface handling
  unchanged.
- Part 2: every unexpected probe outcome degrades to `Unknown` (today's
  behavior); a malformed `release_timestamp` degrades to `Upcoming(None)`.
  Non-YouTube resolvable URLs (Twitch) that report plain `is_live` booleans
  via `live_status = NA` keep working through the Offline/Unknown rows of the
  table above.

## Testing

Part 1 (unit tests in `src/routes/admin/discover/youtube.rs`):

- an upcoming item yields `is_upcoming = true`, `is_live = false`,
  `source_kind = "youtube_live"`, formatted `scheduled_start`;
- thumbnail extraction for video rows and channel rows, including the
  missing-thumbnail → empty-string case;
- the timestamp-format helper: valid RFC 3339 in, `"Jun 12 18:00 UTC"` out;
  garbage in, empty string out.

Part 2 (unit tests in `src/media/resolver.rs` and `src/routes/admin/live_status.rs`):

- `interpret_live_status` table test covering every row above, including
  `is_upcoming|1781546400` → `Upcoming(Some(1781546400))`, `is_upcoming|NA` →
  `Upcoming(None)`, and both stderr fallbacks;
- epoch-format helper: known timestamp → `"Jun 12 18:00 UTC"`;
- `badge_parts` mapping for every state;
- cache TTL: `Upcoming` cached 60 s (determinate), `Unknown` still 10 s;
- `#[ignore]`d yt-dlp integration test pinning real output for an upcoming
  stream URL (exit code + stdout shape with `--ignore-no-formats-error`).

No new routes; no integration-test (tests/http.rs) changes.

## Docs

`docs/IDEAS.md` #35 is struck through with a done note recording that gaps 1–2
were already fixed earlier and gaps 3–4 land with this change, plus a pointer
that the live-status probe now exposes the full `live_status` state model
(foundation for #38/#39).

## Out of scope

- Card-grid redesign of the results table (approach C, rejected).
- "Waiting for stream" playback UX and tune-flow branching on the new states
  (ideas #38/#39 — this spec only provides the state model they will consume).
- Local-timezone rendering of times (UTC only).
- Health-checker changes (`do_http_check` keeps its HTTP-only youtube_live
  rule).
