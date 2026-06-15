# Bottom Player Toolbar (#46) — Design

## Problem

The player overlay toolbar (close `✕` / prev `↑` / next `↓` / help `?`) is anchored
to the **top** of the video (`#player-toolbar{position:absolute;top:0…}` in
`templates/base.html:32`, markup in `templates/guide.html:9`). In normal windowed
playback this strip covers the top of the picture / channel-logo area, which is the
annoyance we are fixing.

Fullscreen is explicitly **out of scope**: fullscreen is invoked on the `<video>`
element itself (`base.html:587`), so DOM overlays on `#player-panel` — including this
toolbar — do not render in true fullscreen. The toolbar only ever appears in windowed
mode, so the only collision risk is the native `<video controls>` bar at the bottom.

## Goal

Move the toolbar to the **bottom** of the player, stacked just above the native
control bar, without overlapping the picture or the native seek/volume controls.

## Approach (chosen: A — bottom overlay, offset above native bar)

Keep the existing floating, auto-hiding toolbar exactly as it behaves today; only
change where it sits. The custom strip floats a fixed offset above the native control
bar so the two stack (YouTube-style) rather than overlap.

Rejected alternatives:

- **B — move buttons into the `#player-info` bar below the video.** Eliminates all
  overlap permanently but loses the floating/auto-hide aesthetic and makes the buttons
  always-visible. Heavier change than the request warrants.
- **C — bottom overlay + custom controls** (replace native `<video controls>`). Most
  control but large scope and fights the project's minimalism. Rejected.

## Changes

All changes are CSS-only, inside the `<style>` block of `templates/base.html`. The
markup (`templates/guide.html`) and all JavaScript — the `show-controls` toggle, the
3 s auto-hide timer, and the button click handlers (`base.html:509–544`) — are
**unchanged**.

1. **`#player-toolbar` anchor** (`base.html:32`): `top:0` → `bottom:44px`. The `44px`
   offset clears the native `<video controls>` bar (≈30–48px across Chrome / Safari /
   Firefox) so the custom strip sits just above it.

2. **Scrim gradient** (`base.html:34`): flip
   `linear-gradient(#000,transparent)` → `linear-gradient(transparent,#000)` so the
   background scrim darkens downward, matching the new bottom anchor.

3. **`#player-help` popup** (`base.html:43`): `top:52px` → `bottom:100px` so the
   shortcuts popup opens **upward** from above the relocated toolbar instead of
   dropping down from the top. (`100px` clears the toolbar at `bottom:44px` plus its
   ~54px height.)

A one-line comment will note that `44px` / `100px` are tied to the approximate native
control-bar height.

## Behavior after the change

- Both the custom toolbar and the native control bar appear on the same
  mousemove / touch / focus trigger and hide together after the 3 s idle timeout, so
  they are only ever visible stacked — the effective overlap window is nil.
- The help popup opens upward, anchored above the toolbar.
- Fullscreen behavior is unchanged (toolbar still not rendered in true fullscreen).

## Testing

- Markup is untouched, so existing integration tests are unaffected; `cargo test`
  stays green.
- Positioning is visual and not unit-testable via the `oneshot` HTTP harness. Verify
  manually by running the app (`cargo run`): confirm the toolbar sits just above the
  native control bar (not over the picture), the help popup opens upward, and the
  fullscreen path is unchanged.

## Out of scope

- Fullscreen toolbar visibility (would require fullscreening `#player-panel` instead of
  the `<video>` — a separate change).
- Custom video controls.
