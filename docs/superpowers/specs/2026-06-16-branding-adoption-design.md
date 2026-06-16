# Branding adoption — favicon, header logo, category icons

**Date:** 2026-06-16
**Status:** Approved (design)

## Summary

Adopt three asset sets from the untracked `branding/` folder into the running
app, then trim `branding/` to a committed reference subset:

1. **Favicon refresh** — swap the browser-tab mark for the rounded V1 Playframe favicon.
2. **Header logo** — add the V1 Playframe mark (recolored to brand pink) beside "MyTV" in both site headers.
3. **Category icons** — replace the category *emoji* in the guide grid and the player info-bar with on-brand inline SVGs.

All three use the existing brand pink `#e94560` (`var(--accent)`). No new
dependencies, no new runtime machinery.

## Context

- The app links `/favicon.svg`, served from `static/favicon.svg` via
  `include_str!` in `src/routes/static_files.rs`. The manifest references the
  same path. `/favicon.ico` 308-redirects to it.
- Category glyphs come from **two** places that already duplicate the same
  substring-matching logic (this duplication predates this work and is the
  accepted pattern):
  - `category_icon()` in `src/routes/guide/badges.rs` → `&'static str` emoji,
    rendered server-side in the guide channel column
    (`templates/partials/epg_content.html` line 45).
  - `channelIcon()` JS in `templates/base.html` → emoji, used by
    `renderInfoBar()` for the player info-bar's `.pi-cat-icon`.
- Both site headers (`templates/base.html`, `templates/admin/base.html`) render
  a plain text `<h1>MyTV</h1>` / `<h1>MyTV Admin</h1>` with no logo mark.

## Component 1 — Favicon refresh

Replace the **contents** of `static/favicon.svg` with the markup from
`branding/icons/favicon-playframe.svg` (rounded screen frame + rounded play
triangle, fill `#e94560`).

- No code change: `include_str!` picks up the new content, the manifest and both
  `<link rel="icon">` tags already point at `/favicon.svg`, and the `.ico`
  redirect is unaffected.

## Component 2 — Header logo (V1 Playframe, pink)

Add the V1 mark as **inline SVG** immediately before the `<h1>` in both headers.

- Source: `branding/logo-exploration/svg/v1-playframe.svg`, recolored from
  orange to pink — frame uses `currentColor`, play triangle filled `#e94560`
  (or `currentColor`).
- New CSS class `.site-logo` (≈26px square, `color: var(--accent)`,
  `flex-shrink:0`), added to each header's `<style>` block. The headers are
  already `display:flex; align-items:center; gap`, so the mark sits inline
  before the title with no layout change.
- The `<h1>` text ("MyTV" / "MyTV Admin") stays.

## Component 3 — Category icons replace emoji (inline)

Chosen delivery: **inline SVG markup** (no static files, no extra requests,
icons inherit `currentColor` so they tint to the surrounding text).

### Rust (`src/routes/guide/badges.rs`)

- `category_icon()` keeps its identical lowercase substring-matching branches but
  returns inline SVG markup (`&'static str`) per category instead of an emoji.
- Source SVGs come from `branding/icons/categories/*.svg`, adapted:
  - Drop the hardcoded `style="color:#e94560"` so the icon inherits `currentColor`.
  - Add `class="cat-icon"` (CSS controls size; the `viewBox` is `0 0 100 100`).
  - Add a stable `data-cat="<key>"` attribute (test hook + semantics).
- The `📺` fallback becomes the `general` TV icon (`data-cat="general"`).
- Category → key mapping (unchanged matching, new return values):
  news, sport, movie (film/cinema), music, kids (child), documentary (docu),
  entertainment, cooking (food), travel, science (tech), general (fallback).

### Template (`templates/partials/epg_content.html`)

- Line 45: `{{ row.category_icon }}` → `{{ row.category_icon|safe }}` (Askama
  HTML-escapes by default; inline SVG requires `|safe`).

### JS (`templates/base.html`)

- `channelIcon(category)` returns the matching inline SVG string (same branches
  as the Rust function), `data-cat` included.
- In `renderInfoBar()`, the `.pi-cat-icon` assignment changes from
  `.textContent =` to `.innerHTML =`.

### CSS (`templates/base.html` `<style>`)

- `.cat-icon` sizing: ≈14px in the guide channel column, ≈15px in the info-bar
  (`.pi-cat svg`). Icons inherit the surrounding text color in both contexts
  (monochrome, clean).

## Testing

- `src/routes/guide/badges.rs` unit tests currently assert emoji equality
  (`category_icon("News") == "📰"`, etc.). Rewrite each to assert the returned
  markup carries the right key, e.g.
  `assert!(category_icon("News").contains(r#"data-cat="news""#))`, and that the
  fallback (`category_icon("Whatever")`) contains `data-cat="general"`.
- `cargo fmt` (CI fails on diff — toolchain pinned to 1.96), `cargo clippy
  -- -D warnings`, and `cargo test` must all pass.
- Manual smoke: `cargo run`, load `/guide` (logo in header, SVG category icons
  in the channel column, refreshed favicon in the tab), tune a channel (info-bar
  shows the SVG icon), load `/admin` (logo in header).

## Component 4 — Clean up `branding/`

`branding/` is currently untracked. Trim to a **reference subset** and commit it.

**Keep:**
- `branding/icons/favicon-playframe.svg`
- `branding/icons/categories/*.svg` (11 SVGs)
- `branding/logo-exploration/svg/v1-playframe.svg`

**Delete:**
- `branding/logo-exploration/.venv/` (Python virtualenv — build junk)
- All PNGs: `branding/icons/*.png`, `branding/icons/categories/*.png`,
  `branding/logo-exploration/png/*`, `branding/logo-exploration/preview-contact-sheet.png`
- `branding/logo-exploration/showcase.html`
- The 5 unused logos: `v2-channel-grid`, `v3-broadcast`, `v4-dotted-play`,
  `v5-scanlines`, `v6-stream-hub` (`.svg`)

After cleanup, `git add branding/` and commit the trimmed archive.

## Out of scope (YAGNI)

- Logo appears only in the two site headers — not the player overlay.
- No `<img>`/sprite serving, no PNG export, no extra manifest icon sizes.
- No admin category-picker icons, no per-channel custom icon upload.
- Free-form `category` strings are unchanged; matching stays substring-based.
