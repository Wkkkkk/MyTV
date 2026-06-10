# YouTube Discover Improvements — Design

**Date:** 2026-06-10
**Idea:** docs/IDEAS.md #35
**Status:** Approved (approach B)

## Context

Idea #35 listed four gaps in the YouTube Discover tab. Code inspection showed two
are already fixed:

1. ~~`source_kind` hardcoded to `youtube_live`~~ — `build_video_rows`
   (`src/routes/admin/discover/youtube.rs`) already sets `youtube_vod` when
   `liveBroadcastContent != "live"`, with a unit test.
2. ~~Live channels not findable by keyword~~ — the YouTube tab already has a
   Videos/Channels type selector backed by `fetch_youtube_channels`
   (`type=channel` search), plus a "Channel URL / @handle" resolve form.

This design covers the two remaining gaps:

3. **No thumbnails** — `snippet.thumbnails` is never extracted; the results
   table is text-only.
4. **"Upcoming" streams mishandled** — `is_live` is
   `liveBroadcastContent == "live"`, so a scheduled stream falls into the VOD
   branch: it shows a VOD badge, duration 0, and the add form asks for a manual
   duration.

### Decision: upcoming streams are badged and addable

Upcoming streams get an UPCOMING badge and an Add button that creates a
`youtube_live` source. This enables pre-adding scheduled events (launches,
matches) — the channel starts working the moment the stream goes live.

### Why this is safe in the existing source lifecycle

No model or health-checker changes are needed; an upcoming stream added as a
`youtube_live` source flows through the existing state machine gracefully:

- **Health check:** the `watch?v=…` page returns HTTP 200 before the stream
  starts, and `youtube_live` health only requires a 2xx/3xx response
  (`do_http_check` in `src/health.rs` skips the body-chunk read for that kind)
  → the source stays `ok`/active and is never auto-disabled while waiting.
- **Live-status badge:** the admin badge shows Offline until the stream
  actually starts (yt-dlp probe).
- **Tune before start:** yt-dlp resolution fails → failover to the next source
  or 503. A "waiting for stream" UX is out of scope here (idea #38).
- **Once live:** the same source starts working — no transition needed.

## Design

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
- The yt-dlp live-status probe is **skipped** for upcoming rows (we already
  know it is not live yet; saves probe-cap slots on the 2-permit semaphore).
- The hidden `is_live` form field sends `true` for upcoming rows too
  (`row.is_live || row.is_upcoming`), so the add form treats them like live:
  no manual duration input, `youtube_live` source kind.
- One new `.badge-upcoming` CSS rule in `templates/admin/base.html` next to
  `badge-live` / `badge-vod`.

### Error handling

Missing thumbnails, missing `liveStreamingDetails`, or unparseable timestamps
all degrade to today's rendering — no new failure paths. The YouTube API
error-surface handling (`error.message` → red empty-state) is unchanged.

### Testing

Unit tests in `src/routes/admin/discover/youtube.rs`:

- an upcoming item yields `is_upcoming = true`, `is_live = false`,
  `source_kind = "youtube_live"`, formatted `scheduled_start`;
- thumbnail extraction for video rows and channel rows, including the
  missing-thumbnail → empty-string case;
- the timestamp-format helper: valid RFC 3339 in, `"Jun 12 18:00 UTC"` out;
  garbage in, empty string out.

No new routes, so no integration-test changes.

### Docs

`docs/IDEAS.md` #35 is struck through with a done note recording that gaps 1–2
were already fixed earlier and gaps 3–4 land with this change.

## Out of scope

- Card-grid redesign of the results table (approach C, rejected for now).
- "Waiting for stream" playback UX for offline/upcoming live sources
  (idea #38).
- Local-timezone rendering of the scheduled start time (UTC only).
