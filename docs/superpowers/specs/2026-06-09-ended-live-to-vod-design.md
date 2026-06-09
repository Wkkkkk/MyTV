# Auto-convert ended YouTube live streams to VOD (idea 36)

**Date:** 2026-06-09
**Status:** Approved — ready for implementation plan

## Problem

When a YouTube live broadcast ends, yt-dlp resolves the source URL to a frozen
HLS manifest whose URL contains `force_finished/1`. The player loads it but the
playlist never advances (no new segments), so the viewer sees a permanent black
screen. The channel stays configured as `live` forever.

## Goal

Detect the ended broadcast at tune time, skip the viewer past it to the next
channel without a black screen, and convert the channel into a `vod_loop` so the
recording stays watchable in the lineup from then on.

## Behavior

1. A viewer tunes a YouTube-live channel whose broadcast has ended.
2. `tune` / `next` resolve the live source and detect `force_finished/1` in the
   resolved URL.
3. They return **HTTP 200** with a new `ended: true` field and an empty `url`
   (the broken manifest is never sent to the player).
4. The frontend shows a ~1.5s overlay — *"Stream ended — switching to next
   channel…"* — then auto-tunes the next channel in `window.epgChannels`
   (wrapping around the lineup).
5. A spawned background task converts the ended channel to `vod_loop`. It stays
   in the lineup and replays the recording on subsequent tunes.

## Backend

### `media/resolver.rs` — pure helpers (unit-testable, no network)

- `is_finished_live(resolved_url: &str) -> bool`
  Returns `resolved_url.contains("force_finished/1")`.
- `live_url_to_watch_url(source_url: &str) -> Option<String>`
  Rewrites `youtube.com/live/<id>` and `youtu.be/<id>` →
  `https://www.youtube.com/watch?v=<id>`. Returns `None` for channel/handle
  `/live` forms (no id embedded in the URL) and for URLs that are already
  `watch?v=` form.

### Background conversion task

Orchestration function lives in `routes/player.rs` and is unit-tested against an
in-memory pool. Steps:

1. Determine the canonical watch URL: try `live_url_to_watch_url(&source.url)`;
   if `None`, fetch the video id via yt-dlp (`--print id`) and build
   `https://www.youtube.com/watch?v=<id>`. This also handles channel/handle
   live URLs that carry no id in the path.
2. Fetch `duration_secs` via the existing `resolver::fetch_duration_secs(watch_url)`.
3. DB mutations (in order):
   - `playlist_item::create` — `title` = the **channel name**, `url` = the watch
     URL, `duration_secs` from step 2, `sort_order` = 0.
   - Flip `channel.type` → `vod_loop` and set `loop_anchor` = now (via the
     existing `channel::update`, or a focused setter if cleaner).
   - `source::deactivate_all_for_channel(channel_id)` — sets `is_active = 0` on
     every source for the channel. Rows are kept for reference/undo; `next_live`
     is never used for this channel again.
4. Any step that fails logs a warning and leaves the channel as `live`; the
   conversion is retried on the next tune. The conversion is effectively
   idempotent — a second run just appends another playlist_item, so the task
   first checks the channel is still `live` before creating the item.

The idea's proposed `source::update_url` is **dropped**: the watch URL now lives
on the new playlist_item, so updating the (deactivated) source URL would be
unused. (YAGNI.)

### `routes/player.rs`

- Add `ended: bool` to `TuneResponse` (defaults `false` at all existing call
  sites; `tune_response` helper sets it `false`).
- In `next_live`, when a resolved source satisfies `is_finished_live`:
  `tokio::spawn` the conversion task and immediately return
  `TuneResponse { ended: true, url: String::new(), .. }`. Do not try the
  remaining sources — they are about to be deactivated anyway.

### `model/source.rs`

- Add `deactivate_all_for_channel(pool, channel_id) -> Result<()>` —
  `UPDATE sources SET is_active = 0 WHERE channel_id = ?`.

## Frontend (`templates/base.html`)

- Route every tune/next response through one shared handler: if `d.ended` is
  truthy, call `advanceEndedChannel()`; otherwise `_loadSource(...)` as today.
  This covers `tune()`, the `video 'ended'` → `/next` handler, and the other
  `/next` fetch sites.
- `advanceEndedChannel()`:
  - Shows the "Stream ended — switching to next channel…" overlay.
  - After ~1.5s, computes the next channel id from `window.epgChannels`,
    reusing the arrow-key wrap logic (factored into a shared
    `nextChannelId(dir)` helper), and calls `tune(nextId)`.
  - **Loop guard:** carries a hop counter so that a lineup where *every* channel
    has ended stops after one full lap (hops ≥ `epgChannels.length`) and shows
    the existing player-error state instead of cycling forever.

## Testing (TDD)

- **Unit (`resolver.rs`):**
  - `is_finished_live` — true for a URL containing `force_finished/1`, false for
    a normal manifest URL.
  - `live_url_to_watch_url` — `/live/<id>` → watch URL; `youtu.be/<id>` → watch
    URL; channel/handle `/live` → `None`; already-`watch?v=` → `None`.
- **Unit (conversion fn, in-memory pool):** after running against a seeded
  live channel + youtube_live source, assert: `channel.type == "vod_loop"` with
  a `loop_anchor` set; exactly one playlist_item exists with the watch URL and
  the expected duration; all sources have `is_active = 0`.
- **Integration (`tests/http.rs`):** seed a source whose resolution yields a
  `force_finished/1` URL → `GET /channel/:id/tune` returns 200 with
  `ended: true` and an empty `url`. yt-dlp-dependent resolution paths stay
  behind `#[ignore]`, matching the existing resolver tests.

## Out of scope

- No new DB migration (sources gains no duration column; duration lives on the
  playlist_item per the existing schema).
- No change to the background health checker.
- No UI for manually reverting a converted channel back to live (sources are
  retained deactivated, so a manual revert is possible via existing admin tools
  if ever needed).
