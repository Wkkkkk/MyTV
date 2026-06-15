# On-Demand VOD Channel Type (#45) — Design

**Idea:** [#45 in `docs/IDEAS.md`](../../IDEAS.md) — let the viewer navigate *across* VOD
playlist items (prev/next/jump/replay), not just nudge ±10 s within the current item.

**Decision:** rather than bend `vod_loop`'s broadcast-clock semantics, add a **third channel
type** `vod_on_demand`. An on-demand channel has no wall-clock derivation, so cross-item
navigation, seeking, and replay "just work" without the "snap back to the clock" problem.

---

## Mental model

| Type | Model | Position |
|------|-------|----------|
| `live` | live stream w/ fallback sources | n/a |
| `vod_loop` | **broadcast simulation** — you tune into whatever is "airing" | derived from `loop_anchor` + wall clock |
| `vod_on_demand` *(new)* | **viewer-controlled playlist** — you pick what to watch | remembered in the browser (localStorage) |

On-demand behavior:

- Items play **sequentially**, item by item.
- Seeking within an item uses the **native `<video controls>` timeline** (MP4 / progressive
  media is fully seekable via HTTP range requests). No custom seek logic.
- The viewer **clicks any item** in an on-screen list to jump to it or replay it.
- **No loop.** After the last item ends, playback **stops silently** (last frame stays; no
  notice, no auto-advance, no channel hop).
- Playback position (which item + offset) is **remembered in the browser** — resume works on
  the same browser; no server-side cursor, no cross-device sync.

### Why not the other options

The IDEAS note listed three ways to retrofit navigation onto `vod_loop`:
(a) a transient server endpoint, (b) rebasing `loop_anchor` on skip, (c) a client-side cursor
that breaks free of the clock. (a) and (b) both stay inside the broadcast-clock model, so a
jump is either momentary or rewrites a shared timeline. This design is essentially (c), but
promoted to a **first-class channel type** so the clock simply isn't there to fight.

---

## Channel type & admin

- Extend the `ChannelType` enum with `VodOnDemand` ↔ stored string `"vod_on_demand"`.
- **No schema migration.** `channels.type` is already a free-form string column; `loop_anchor`
  stays `NULL` (unused for on-demand).
- **Admin:** add the option to the channel create/edit **type dropdown**. Playlist items use
  the *same* `playlist_items` table and the *same* admin CRUD as `vod_loop`; duration auto-fill
  (#44) still applies.
- `duration_secs` keeps its existing `> 0` validation. For on-demand it is **display-only** (no
  clock math) — kept required to avoid changing the shared intake path.

---

## Server: two player endpoints

Both are **public** (same as `/channel/:id/tune` and `/next` — no auth) and reuse existing
helpers. Registered in `src/lib.rs` next to the existing player routes; handlers in
`src/routes/player.rs`.

1. **`GET /channel/:id/playlist`**
   → JSON `[{ "id", "title", "duration_secs" }, …]` for the channel's **active** items in
   `sort_order`. Drives the clickable list.
   - `404` if the channel does not exist.
   - Empty array if the channel has no active items (client shows a "no items" state).

2. **`GET /channel/:id/item/:item_id`**
   → resolves *that specific* item via `resolver::resolve_url` and returns a normal
   `TuneResponse` (`url`, `skip_proxy` from `resolver::should_skip_proxy`,
   `start_offset_secs: 0`, `playlist_item_id: Some(item_id)`), reusing `tune_response`.
   - `404` if the channel does not exist; `422` if `item_id` is not an active item of this
     channel.
   - `503` if resolution fails (matches `tune_vod_at`).

Additionally:

- Add `"type"` to `channels_json` in `src/routes/guide/data.rs` so each entry is
  `{ id, name, type }`. This lets the client branch on channel type **before** tuning.
- `GET /channel/:id/tune` for a `vod_on_demand` channel resolves the **first** active item
  (offset 0) so a direct API hit still returns something playable. The JS player path below
  does not depend on this — it drives `/playlist` + `/item/:id` directly.

---

## Client (`templates/base.html`)

`tune(channelId)` looks up the channel `type` from `window.epgChannels`.

**If `vod_on_demand`:**

1. `GET /channel/:id/playlist` → render the clickable item list.
2. Read `localStorage["mytv:ondemand:<id>"]` → `{ itemId, offset }`. Default to the first item
   at offset 0; if the saved `itemId` is no longer in the list, fall back to the first item.
3. `GET /channel/:id/item/:itemId` → load the resolved URL at the saved offset (reusing
   `_loadSource(url, offset, skip_proxy)`).

**Otherwise:** existing `live` / `vod_loop` behavior is unchanged.

**Interactions:**

- **Click an item** → resolve + load at offset 0; update the current-item highlight. Replay =
  clicking the item that is already current.
- **`video 'ended'`** → advance to the next item index; if there is none, **stop silently**
  (leave the last frame, no auto-advance, no channel hop).
- **Persist position** → save `{ itemId, offset: video.currentTime }` to localStorage,
  throttled on `timeupdate` and on `pause` / `beforeunload`.
- **Keyboard unchanged** — Up/Down = channel, Left/Right = native seek, Space = play/pause,
  F = fullscreen. The native `<video controls>` timeline provides scrubbing.

**Error handling:**

- Item resolve `503` → show the existing player-error, keep the list visible so the viewer can
  pick another item (no auto-advance — consistent with the controlled model).
- Empty playlist / unavailable / corrupt localStorage → default to first item or a "no items"
  state, mirroring today's VOD-empty handling.

---

## Playlist toolbar UI

The item list lives in the existing **bottom overlay** (`#player-panel`), toggled by a new
toolbar button next to the help (`?`) button, reusing the existing show-controls / 3 s
auto-hide behavior.

**Row layout** (handles mixed CJK / Latin titles cleanly):

- Each row is a flex row: `[now-playing marker] [title — flex:1] [duration — fixed, right]`.
- **Title**: left-aligned, `flex: 1`, single line —
  `overflow: hidden; text-overflow: ellipsis; white-space: nowrap` — with the full title in a
  `title=` attribute for when it is truncated. Uniform row height regardless of script.
- **Duration**: fixed-width, right-aligned, `font-variant-numeric: tabular-nums` so `2:05` and
  `12:30` line up digit-for-digit.
- **Font / line-height**: rely on the OS system font stack already used in the app (picks the
  right CJK face automatically); line-height ~1.4 so CJK glyphs aren't cramped. Do **not**
  force `word-break` — ellipsis handles overflow, avoiding mid-character CJK breaks.
- The current item gets the marker plus an accent highlight, reusing the `--accent` design
  token (#30).

---

## Testing

Integration tests (`tests/http.rs`, `tower::ServiceExt::oneshot`):

- `/channel/:id/playlist` returns active items in `sort_order`; `404` for a missing channel;
  empty array for a channel with no active items.
- `/channel/:id/item/:item_id` returns a resolved `TuneResponse` with `start_offset_secs: 0`
  and the correct `playlist_item_id`; `404` for a missing channel; `422` for an item not on the
  channel; `503` when resolution fails.
- `channels_json` includes `type` for each entry.
- `ChannelType` parses/serializes `vod_on_demand` round-trip.

Fixtures: add a `vod_on_demand` channel (e.g. ID 6) with 2–3 items to `tests/fixtures/seed.sql`.

Client JS stays manual / e2e-covered (project convention — no JS unit tests).

---

## Out of scope

- `vod_loop` behavior is unchanged.
- No server-side or cross-device resume cursor (browser-only).
- No item reordering UI (that is idea #8).
- No transcoding; on-demand assumes browser-playable media, same as the rest of the app.
