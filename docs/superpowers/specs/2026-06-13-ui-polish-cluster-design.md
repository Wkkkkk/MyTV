# UI Polish Cluster (#29–#33) — Design

**Date:** 2026-06-13
**Status:** Approved, ready for implementation plan
**Ideas covered:** `docs/ideas.md` #29 (UI quick-wins), #30 (CSS design tokens), #31 (keyboard & accessibility polish), #32 (player overlay controls), #33 (HTMX loading skeletons)

## Summary

Five related front-end improvements, shipped as one cohesive change. They touch the same handful of template/JS files and share a foundation: a new shared stylesheet (`static/app.css`) holding CSS design tokens, which the color normalization, dedup, and focus-ring work all build on.

The only backend change is a single new static-asset route. Everything else is templates, CSS, and vanilla JS — no new dependencies.

## Decisions locked during brainstorming

- **Scope:** all five ideas in one spec.
- **Color direction:** neutralize the blue-tinted backgrounds to true-neutral grey, but **keep the pink-red `#e94560` brand accent** (header, active tab, "now" line, player border). This is what #29 literally describes; we are not going fully monochrome.
- **Debug panel:** **keep exactly as-is** (always visible, fixed bottom). The #29 sub-item to gate it behind `?debug`/localStorage is dropped from scope. Only non-visual a11y `aria-label`s are added to its buttons.
- **Player overlay layout:** **top fading toolbar** (not edge arrows). Native `<video controls>` bottom bar stays untouched.
- **Grey consolidation:** the four muted greys (`#555`/`#666`/`#777`/`#999`) collapse to two tokens, `--text-muted` (`#999`) and `--text-dim` (`#666`).

## Architecture

Introduce **`static/app.css`**, served at `/app.css` by a new `app_css()` handler in `src/routes/static_files.rs` — mirroring the existing `favicon_svg`/`manifest_json` pattern (`include_str!("../../static/app.css")` + a `text/css` content type + a route registered in `src/lib.rs`).

`app.css` contains:
- the `:root` design-token custom properties, and
- the shared `.tabs` / `.tab` CSS block currently copy-pasted into both base templates.

Both `templates/base.html` and `templates/admin/base.html` link it via `<link rel="stylesheet" href="/app.css">` in `<head>`, then replace their hardcoded hex values with `var(--…)` references. All other CSS remains inline in its template.

This route is the sole backend change.

## Components

### 1. Design tokens + color normalization (#30, #29 color)

Define this token ramp in `:root` within `app.css`:

| Token | Value | Replaces / role |
|-------|-------|-----------------|
| `--bg` | `#0f0f0f` | page background |
| `--surface-1` | `#111` | header, channel column, info bar, debug panel base |
| `--surface-2` | `#1a1a1a` | tabs, program blocks — **normalized** from blue `#1a2235` |
| `--surface-nav` | `#141414` | EPG nav bar — **normalized** from blue `#141420` |
| `--border` | `#222` | primary borders |
| `--border-subtle` | `#1c1c1c` | row separators / faint borders |
| `--text` | `#e0e0e0` | body text |
| `--text-muted` | `#999` | secondary text (consolidates `#777`/`#999`) |
| `--text-dim` | `#666` | tertiary/faint text (consolidates `#555`/`#666`) |
| `--accent` | `#e94560` | brand accent (kept) |
| `--live` | `#c0001a` | LIVE badge |
| `--ok` | `#4caf50` | health-ok glyph |

- The green "live" program tint (`#1a2a1a`) stays as a semantic signal, expressed as a token (e.g. `--live-tint`); only the neutral base surfaces lose their blue cast.
- Inline `style="…"` attributes in `base.html`'s header nav move into classes.
- The budget badge colors (`--budget-direct`, `--budget-proxied`) may also become tokens for consistency, but their values do not change.

### 2. Font-size bumps (#29)

In `base.html`'s EPG styles: `.time-label` `0.7rem → 0.75rem`; `.program` / `.program-title` `0.78rem → 0.82rem`.

### 3. HTMX spinners + loading skeletons (#29, #33)

- **EPG skeleton:** a shimmer-animated placeholder block carrying the `htmx-indicator` class, rendered inside `#epg-content`. The category and time-nav buttons get `hx-indicator="#epg-skeleton"` so the skeleton shows while the partial is in flight. Skeleton rows mirror the grid geometry (channel column + variable-width program strips). Shimmer via a CSS `@keyframes` gradient sweep.
- **Discovery search spinners:** an inline `htmx-indicator` spinner on the discovery search forms in `templates/admin/discover.html`.
- **Player buffering overlay:** a centered spinner + "Loading…" element inside `#player-panel`, shown when `_loadSource` begins and hidden on first playback (`video` `playing` event / HLS `MANIFEST_PARSED` / DASH `CAN_PLAY`), and on error / waiting / ended transitions. Replaces today's silent black screen.

### 4. Keyboard & accessibility polish (#31)

- `:focus-visible` outline rings (using `--accent`) on `.program`, `.tab`, `.nav-btn` in `app.css` / `base.html`.
- Program blocks (`templates/partials/epg_content.html`) already carry `role="button" tabindex="0"`; add a `keydown` handler so Enter/Space invokes `tune()` (matching the existing `onclick`).
- Convert the hrefless `<a hx-get>` EPG category tabs and time-nav controls to `<button>` elements (semantically correct, keyboard-operable without JS hacks); keep their `hx-get`/`hx-target`/`hx-swap` attributes. Add `aria-selected` to the active tab.
- Add `aria-label`s to the debug panel's Clear/Hide buttons (no visual change; panel otherwise unchanged).

### 5. Player overlay toolbar (#32)

Add a top fading toolbar inside `#player-panel` (markup in `templates/guide.html`):
- Left group: **✕ close**, **↑ prev channel**, **↓ next channel**.
- Right group: **? help**.
- The native bottom `<video controls>` bar is untouched.

Behavior (JS in `base.html`):
- Toolbar fades in on `mousemove` / `touchstart` / focus within the panel; fades out after ~3s of inactivity.
- **prev/next** reuse the existing `nextChannelId('up'|'down')` + `tune()`.
- **close** calls the existing `stopPlayback()` and hides the panel (and any overlay).
- **?** toggles a small shortcut panel listing the existing bindings: ↑↓ change channel, Space play/pause, ←→ seek (VOD only), F fullscreen.

No new keyboard bindings are introduced — the toolbar surfaces controls that already exist.

## Testing

- **Integration (Rust, `tests/http.rs`):**
  - `GET /app.css` returns `200` with `content-type: text/css`.
  - The guide page HTML contains the overlay toolbar markup, the EPG skeleton element, and `<button>` (not `<a>`) category tabs.
- **CI gates:** `cargo fmt` clean, `cargo clippy -- -D warnings` clean, `cargo test` green.
- **Manual:** browser check (via the run skill) confirming color normalization, skeleton on category switch, overlay fade behavior, focus rings on Tab navigation, and the buffering overlay on tune.

## Out of scope

- Debug panel gating (kept as-is by decision).
- Full monochrome theme (accent retained).
- Any new JS/CSS dependencies.
- Any change to EPG data, player logic, or routes beyond the `/app.css` static route.
