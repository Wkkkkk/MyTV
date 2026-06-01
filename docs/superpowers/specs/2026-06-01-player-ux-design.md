# Player UX + Mobile/TV UI — Design Spec

**Date:** 2026-06-01
**Status:** Approved

---

## Overview

Improve the player experience with a channel info bar below the video, keyboard shortcuts for play/pause, fullscreen, channel navigation, and VOD seeking. Polish the responsive layout for phone and TV screens.

No new backend endpoints. No layout restructuring — the stacked layout (player above, EPG below) is kept as-is on all screen sizes.

---

## Backend changes

### Enrich `TuneResponse`

Add channel metadata to the existing `TuneResponse` struct returned by `/channel/:id/tune` and `/channel/:id/next`:

```rust
pub struct TuneResponse {
    pub url: String,
    pub start_offset_secs: i64,
    // new fields:
    pub channel_id: i64,
    pub name: String,
    pub logo_url: Option<String>,
    pub category: String,
    pub channel_type: String,   // "live" or "vod_loop"
}
```

All fields are already available on the `Channel` struct — this is purely passing them through to the JSON response. No DB query changes.

### Embed channel list in EPG template

The EPG template emits a `<script>` block that sets `window.epgChannels` to a JSON array of `{id, name}` objects in the order the EPG rows render (i.e. sorted by `sort_order`, filtered by the active category).

```html
<script>window.epgChannels = [{"id":1,"name":"SVT1"},{"id":2,"name":"TV4"},...];</script>
```

This block lives in `templates/partials/epg_content.html` so it is re-emitted every time HTMX refreshes the EPG (category tab change, time nav). The JS keyboard handler always reads the current filtered list.

---

## Channel info bar

A `#player-info` bar is added to `guide.html` between the `<video>` and the EPG content div. Hidden by default (`display:none`), shown when a channel is tuned.

### Layout (left → right)

| Slot | Content |
|------|---------|
| **Logo** (28×28px, always present) | `<img src=logo_url>` if set; else a colored tile with the first letter of the channel name. Tile background color is derived from a simple hash of the channel name so the same channel always gets the same color. |
| **Name + category** | Channel name in white bold; below it: category icon + category text (e.g. `⚽ Sports`). If category is empty, show `📺` with no text label. |
| **LIVE badge** | Red `LIVE` pill, shown only when `channel_type == "live"`. |
| **Position** | `Ch N / M` right-aligned. `N` = 1-based index of current channel in `window.epgChannels`; `M` = array length. Hidden if `window.epgChannels` is empty. |

### JS

`tune()` stores the full channel object from the enriched response in a module-level variable `currentChannel`, then calls `renderInfoBar()` immediately. The bar is shown as soon as the tune fetch resolves — before the stream loads — so there is no blank gap. `_loadSource()` does not touch the info bar.

---

## Keyboard shortcuts

A single `keydown` listener on `document`, registered once on `DOMContentLoaded`. Skipped when `event.target` is an `<input>`, `<textarea>`, or `<select>` so admin forms are unaffected.

| Key | Action | Guard |
|-----|--------|-------|
| `Space` | Toggle play/pause on `<video>` | `e.preventDefault()` to suppress page scroll |
| `f` or `F` | Toggle native browser fullscreen (`video.requestFullscreen()` / `document.exitFullscreen()`) | — |
| `ArrowUp` | Tune to previous channel in `window.epgChannels` (wraps) | — |
| `ArrowDown` | Tune to next channel in `window.epgChannels` (wraps) | — |
| `ArrowLeft` | `video.currentTime -= 10` | Only when `currentChannel.channel_type == "vod_loop"` |
| `ArrowRight` | `video.currentTime += 10` | Only when `currentChannel.channel_type == "vod_loop"` |

`ArrowLeft`/`ArrowRight` on a live channel are silently ignored — no re-tune, no error. `ArrowUp`/`ArrowDown` call `tune(id)` which re-uses all existing failover logic.

---

## Responsive CSS

### Mobile (≤600px)

- `#player-info` reduces padding (`6px 8px`) and logo tile shrinks to 24×24px
- Category text (the word, not the icon) hidden via `display:none` on the text span; icon stays
- `Ch N / M` stays visible
- Existing EPG mobile breakpoint (narrow channel column) unchanged

### TV / large screen (≥1280px)

- `#player-panel video` `max-height` raised from `50vh` to `60vh`
- No other changes — keyboard shortcuts map naturally to TV remote D-pad

---

## What is not changing

- Page layout — stacked (player above, EPG below) on all screen sizes
- Debug panel — already fixed at bottom, no z-index conflict with the static info bar
- Backend route wiring, auth, health checker — untouched
- EPG HTMX partial swap mechanism — `window.epgChannels` is re-emitted by the partial, so it stays in sync automatically
