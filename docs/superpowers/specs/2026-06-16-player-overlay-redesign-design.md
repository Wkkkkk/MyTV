# Player overlay redesign — design (idea #51)

**Date:** 2026-06-16
**Status:** Approved design, pre-implementation
**Idea:** #51 in `docs/IDEAS.md` — redesign the player overlay for VOD-on-demand fit and mobile/touch.

## Problem

The player overlay (`#player-toolbar` in `templates/guide.html`; player/keyboard logic in
`templates/base.html`) was built for keyboard-driven live/`vod_loop` viewing and fits poorly elsewhere:

1. **VOD-on-demand mismatch** — the overlay's model is up/down *channel* navigation, and seek
   (`←`/`→`) is gated to `vod_loop` only (`base.html:859`), so on-demand channels — which are
   *playlist* navigation (prev/next item via the `☰` panel) — get a toolbar built for a different
   interaction model.
2. **Mobile / no keyboard** — every primary interaction is a key press (Space, `↑`/`↓`, `←`/`→`, `F`),
   and the only on-screen affordances are small toolbar glyphs. No touch-friendly seek, no swipe/tap
   navigation; the help overlay lists only keyboard shortcuts.

## Goals & non-goals

**Goals**
- Touch and keyboard are **co-equal, first-class** interactions (neither is a fallback).
- A gesture-forward, minimal-chrome player (Direction C) that adapts to channel type.
- On-demand channels become first-class: item navigation + within-item seek.

**Non-goals / unchanged**
- **No backend, API, DB, or migration changes.** The tune/next/playlist endpoints already return
  `channel_type` and item data; nothing server-side changes.
- The EPG guide layout, the `#player-info` strip (channel name/category/LIVE/position, shown below
  the video), and the error/ended/waiting/buffering overlays are unchanged.
- No new runtime dependencies.

**Behavior fix that comes along:** seek is currently gated to `vod_loop` only; on-demand becomes
seekable too.

## Chosen direction — C (gesture-forward, minimal chrome)

- **Idle state:** video fills the screen, no chrome.
- **Reveal chrome:** single tap (touch) or `mousemove`/keypress (desktop). Chrome auto-hides after
  ~3s of inactivity.
- **Playback controls are custom** (the native `<video controls>` bar is removed — it would fight
  the gesture model and looks inconsistent across iOS/Android). Fullscreen delegates to the native
  Fullscreen API. This is the "hybrid: custom transport, native fullscreen" decision.
- **Help is on-demand only** — a `?` button/key opens a gesture + keyboard cheat-sheet. No
  first-run coachmarks, no always-visible hint affordances.

## Interaction model

| Action | Touch | Keyboard | Mouse (chrome shown) | Channel types |
|---|---|---|---|---|
| Play / pause | tap center | `Space` | click center play | all |
| Seek ±10s | double-tap left / right edge | `←` `→` | drag scrubber | vod_loop, on-demand |
| Navigate (item → channel overflow) | swipe up / down | `↑` `↓` | — | all |
| Fullscreen | ⛶ button | `F` | ⛶ button | all |
| Close player | ✕ button | `Esc` | ✕ button | all |
| Help / cheat-sheet | ? button | `?` | ? button | all |

The chrome also keeps **⏮ / ⏭** item buttons and the **☰** playlist picker (on-demand only) for
discoverability and direct mouse use.

**Dropped from the old model:** the dedicated `[` / `]` item keys and any horizontal swipe gesture —
the unified vertical axis now handles items, and dropping horizontal swipe avoids clashing with the
browser's back-swipe on phones.

### Unified vertical navigation (the core behavior)

`↑`/`↓` and swipe up/down are a single **navigate** axis that treats the whole channel list as one
continuous feed. On-demand channels expand into their playlist items; live/`vod_loop` channels are
single entries.

- **Down / next:** advance to the next item in the current on-demand playlist. At the **last** item
  (or on a live/`vod_loop` channel, which has no items), advance to the **next channel** — landing on
  its **first item** if that channel is on-demand, otherwise just starting it normally.
- **Up / prev:** go to the previous item. At the **first** item (or on live/`vod_loop`), go to the
  **previous channel** — landing on its **last item** if on-demand, otherwise starting it normally.
- **Boundaries:** at the very top (first item of the first channel) and bottom (last item of the last
  channel) the feed **stops — no wrap**, matching today's channel navigation. (The current
  `nextChannelId` no-wrap behavior is to be confirmed during planning and preserved.)

### Per-channel-type behavior

| Type | Scrubber + seek | Item navigation | Badges |
|---|---|---|---|
| `live` | hidden / disabled | none (single feed entry) | LIVE |
| `vod_loop` | shown, seek enabled | none | position |
| `vod_on_demand` | shown, seek enabled | items via vertical nav, ⏮/⏭, ☰ | position |

Gestures map cleanly with no conflict: **single tap** (toggle chrome) vs **double-tap near an edge**
(seek) vs **swipe** (navigate) are all distinguishable. On `live`, seek gestures are inactive.

## Components & implementation surface

This is a **frontend-only** change across three files.

- **`templates/guide.html`** — rework the `#player-toolbar` markup into the Direction-C chrome:
  - top bar: `✕` close + channel name;
  - center: large play/pause button;
  - bottom transport: `⏮` · scrubber · time (`current / duration`) · `⏭` · `☰` · `⛶`.
  - Update `#player-help` content to a gesture + keyboard cheat-sheet.
- **`templates/base.html`** (player JS, ~L185–866) — the bulk of the work:
  - Remove the `controls` attribute from `<video>`.
  - **Gesture layer:** detect single tap (toggle chrome), double-tap near left/right edge (seek
    ∓10s), and vertical swipe (navigate). Distinguish tap vs double-tap vs swipe.
  - **`navigate(dir)` function:** implement the continuous item→channel feed, extending today's
    `nextChannelId` (channel stepping) and the on-demand `odItems`/`odIndex` logic (item stepping).
  - **Custom transport:** play/pause toggle; scrubber bound to `timeupdate`/`seeking`/`durationchange`;
    time display; chrome show/hide timer (reuse the existing `showControls` plumbing at
    `base.html:760`).
  - **Channel-type-aware:** enable/disable the scrubber and seek per `currentChannel.channel_type`;
    show the LIVE badge for live; show ⏮/⏭/☰ only for on-demand.
  - Ungate seek so it works for `vod_loop` **and** `vod_on_demand` (replacing the
    `channel_type !== 'vod_loop'` early return at `base.html:859`).
  - Keyboard handler updated to the new map (`↑`/`↓` → `navigate`, `←`/`→` → seek for both seekable
    types, `Esc` → close, `?` → help; drop `[`/`]`).
- **`templates/app.css`** (served at `/app.css`) — new overlay / transport / scrubber styles using
  the existing design tokens; touch-sized hit targets.

## Testing & verification

This project has **no JS test harness** (tests are Rust `oneshot` integration tests + the prod e2e
suite). Verification is therefore **manual / visual across viewports**, walking the interaction table
per channel type:

- **Desktop keyboard:** `Space`, `←`/`→` (seek on vod_loop + on-demand), `↑`/`↓` (navigate incl.
  item→channel overflow), `F`, `Esc`, `?`.
- **Desktop mouse:** chrome reveal on `mousemove`, center play, scrubber drag, ⏮/⏭, ☰, ⛶, ✕.
- **Phone (touch):** tap toggle, tap center play, double-tap-edge seek, swipe up/down navigate,
  chrome auto-hide, fullscreen.
- **Per type:** confirm `live` has no scrubber/seek; `vod_loop` seeks but has no item nav;
  `vod_on_demand` does items + seek; and the continuous-feed boundaries (overflow into adjacent
  channels, no-wrap at the ends) behave as specified.

The existing Rust integration tests continue to guard that the tune/next/playlist endpoints are
unchanged. `cargo fmt`, `cargo clippy -- -D warnings`, and `cargo test` must pass (no Rust changes
expected, but they confirm nothing regressed and the templates still compile via Askama).

## Open items to confirm during planning

- Verify and preserve the no-wrap behavior of `nextChannelId` for the feed boundaries.
- Confirm the scrubber behaves sensibly for `live` HLS (hidden; play/pause only).
