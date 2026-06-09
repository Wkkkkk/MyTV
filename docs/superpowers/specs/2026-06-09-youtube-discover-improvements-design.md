# YouTube Discover Improvements — Design

Date: 2026-06-09
Idea: docs/IDEAS.md #35 (sub-parts 1 and 2 only)

## Scope

From idea 35, this work implements **two** of the four sub-parts:

1. **VOD `source_kind` fix** — stop hardcoding `youtube_live` for VOD results;
   label them `youtube_vod` and confirm VOD videos flow to `vod_loop` handling.
2. **Channel search** — let live channels (e.g. NASA TV) be found by keyword
   (`type=channel`) *and* by pasting a channel URL / `@handle`.

Explicitly **out of scope** (deferred from idea 35): (3) thumbnails, (4)
`liveBroadcastContent="upcoming"` filtering/badge.

## Background / findings

- `SourceKind` (`src/model/source.rs`) has only `Hls`, `YoutubeLive`, `Iptv`,
  `Dash`. There is no `youtube_vod` kind anywhere in the codebase.
- `source_kind` is **not** what gates playback. At tune time the player decides
  whether to run yt-dlp from the URL (`resolver::needs_resolution`), and
  live-vs-VOD playback is driven by the **channel type** (`live` vs `vod_loop`),
  not the source's kind string. So `source_kind` on a YouTube source is
  effectively an admin-facing label.
- The Add form (`templates/admin/partials/discover_add_form.html`) **already**
  flips the new-channel `new_channel_type` default to `vod_loop` when
  `is_live=false`. So VOD steering for new channels is largely in place; the
  concrete gap is the hardcoded `youtube_live` label in
  `discover_yt_results.html` line 32.
- Channel live streams resolve through the existing yt-dlp path via the
  canonical URL `https://www.youtube.com/channel/{channelId}/live`.

## Section A — Data model & sub-part (1)

1. Add `SourceKind::YoutubeVod` in `src/model/source.rs`:
   - `as_str()` → `"youtube_vod"`
   - `FromStr` accepts `"youtube_vod"`
   - `Display` derives from `as_str()` (already does)
   - `detect()` **unchanged** — a `watch?v=` URL cannot be told apart from live
     by URL alone; the live/VOD distinction comes from the API's
     `liveBroadcastContent`.
   - Audit for any exhaustive `match` on `SourceKind` and add the new arm.
2. `YoutubeResultRow` gains a `source_kind: String` field. In
   `fetch_youtube_results`, set it to `youtube_live` when `is_live` else
   `youtube_vod`. `discover_yt_results.html` line 32 becomes
   `value="{{ row.source_kind }}"`.
3. No new VOD-steering code: the existing `is_live`-based `new_channel_type`
   default ordering is correct. Lock it with a test instead.

## Section B — Channel search (sub-part 2, both mechanisms)

### 2a — Videos/Channels keyword toggle

- `YoutubeSearchForm` gains `search_type: Option<String>` (default `"video"`).
- `discover_youtube_search` branches on `search_type`:
  - `"video"` → existing `fetch_youtube_results` path.
  - `"channel"` → new `fetch_youtube_channels(keyword, api_key, client)`:
    - GET `search?part=snippet&type=channel&maxResults=12&q=…&key=…`
    - read `id.channelId` + `snippet.channelTitle` (and `snippet.title`)
    - build `url = https://www.youtube.com/channel/{channelId}/live`
    - `is_live = true`, `duration_secs = 0`, `source_kind = "youtube_live"`
    - no `videos.list` call (no durations needed)
    - returns the same `YoutubeResultRow` type so the
      results → add-form → add pipeline is fully reused.

### 2b — Channel-URL input

- New pure fn `normalize_channel_url(input: &str) -> Option<String>`
  (no network, unit-testable):
  - `youtube.com/channel/UC…`, `/@handle`, `/c/Name`, `/user/Name`
    → append `/live` (idempotent if the path already ends with `/live`).
  - bare `@handle` → `https://www.youtube.com/@handle/live`.
  - non-YouTube / unparseable → `None`.
- New form type `ChannelUrlForm { url: String }` and handler
  `discover_channel_resolve`:
  - normalize; `None` → HTTP 422.
  - render a prefilled add-form (reuse `ManualResultTemplate`) with
    `is_live=true`, `source_kind="youtube_live"`, `duration_secs=0`,
    title = derived handle/channel segment (user-editable in the form).
  - **No API key required** for this path — so it is integration-testable.

## Section C — UI, routes, tests

### UI (`templates/admin/discover.html`)

- Add `<select name="search_type">` (Videos / Channels) to the YT keyword form,
  inside the `youtube_api_key_set` guard (channel keyword search needs the key).
- Add a "Channel URL / @handle" input form on the YouTube tab posting to the new
  endpoint. Placed **outside** the key guard so 2b works even without a key,
  targeting its own result div.
- `discover_yt_results.html`: line 32 uses `{{ row.source_kind }}`; channel rows
  carry `is_live=true` so they render the existing `LIVE` badge.

### Routes (`src/lib.rs`)

- Add `POST /admin/discover/channel/resolve` → `discover_channel_resolve`.
- Export the new handler from `src/routes/admin/mod.rs` and
  `src/routes/admin/discover/mod.rs`.

### Tests

- `src/model/source.rs`: `YoutubeVod` round-trip (`as_str` / `FromStr`).
- `src/routes/admin/discover/youtube.rs`:
  - parse a `type=channel` JSON fixture → rows with `/channel/{id}/live` URLs,
    `is_live=true`, `source_kind="youtube_live"`.
  - video results emit `source_kind="youtube_vod"` when `liveBroadcastContent`
    is not `"live"`, and `youtube_live` when it is.
  - `normalize_channel_url` cases: `/channel/UC…`, `/@handle`, `/c/Name`,
    `/user/Name`, bare `@handle`, already-`/live` idempotency, non-YouTube
    → `None`.
- `tests/http.rs`: integration test POSTing to
  `/admin/discover/channel/resolve` (valid handle → 200 prefilled add-form;
  non-YouTube URL → 422). No API key required.

### Conventions

- `cargo fmt` then `cargo clippy -- -D warnings` then `cargo test` before commit.
- No comments unless the WHY is non-obvious.
- Askama `{% match %}`/`{% when %}` for `Option` types.

## Non-goals

- No new migration (no schema change; `youtube_vod` is just a stored string).
- No change to the tune/resolve playback path.
- No thumbnails, no upcoming-stream handling (deferred sub-parts of idea 35).
