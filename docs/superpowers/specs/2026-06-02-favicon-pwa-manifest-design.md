# Favicon & PWA Manifest Design

**Date:** 2026-06-02  
**Status:** Approved

## Overview

Add a favicon and PWA web app manifest to MyTV so the browser tab shows a recognizable icon and browsers on Android/desktop offer an "Add to Home Screen" / "Install" prompt. No service worker or offline support. No iOS home screen icon required.

## Icon

TV outline with a play triangle inside, using the brand accent color `#e94560` on a transparent background. Implemented as SVG for crisp rendering at all sizes. The SVG file lives at `static/favicon.svg` and is embedded into the binary via `include_str!` at compile time.

## Routes

Three new handlers added to the main router in `src/lib.rs`:

| Route | Handler | Response |
|---|---|---|
| `GET /favicon.svg` | inline in `src/routes/static_files.rs` | SVG, `Content-Type: image/svg+xml` |
| `GET /manifest.json` | inline in `src/routes/static_files.rs` | JSON, `Content-Type: application/manifest+json` |
| `GET /favicon.ico` | inline in `src/routes/static_files.rs` | 308 redirect to `/favicon.svg` |

All content is embedded at compile time — no filesystem reads at runtime, consistent with the Askama template approach. No new dependencies required.

## Manifest content

```json
{
  "name": "MyTV",
  "short_name": "MyTV",
  "start_url": "/guide",
  "display": "standalone",
  "background_color": "#0f0f0f",
  "theme_color": "#e94560",
  "icons": [{ "src": "/favicon.svg", "sizes": "any", "type": "image/svg+xml" }]
}
```

`"sizes": "any"` is the correct manifest value for SVG icons.

## Template changes

Both `templates/base.html` and `templates/admin/base.html` get three tags added to `<head>`:

```html
<link rel="icon" type="image/svg+xml" href="/favicon.svg">
<link rel="manifest" href="/manifest.json">
<meta name="theme-color" content="#e94560">
```

## New files

- `static/favicon.svg` — the TV+Play SVG icon
- `src/routes/static_files.rs` — three route handlers

`src/routes/mod.rs` gets `pub mod static_files;` added. `src/lib.rs` gets three `.route()` calls added to the main router.

## Testing

One integration test in `tests/http.rs`: `GET /favicon.svg` returns HTTP 200 with `content-type: image/svg+xml`. The manifest and ICO redirect are verified manually.

## Out of scope

- PNG icons (not needed for SVG-only PWA support)
- iOS apple-touch-icon
- Service worker / offline support
- Caching headers (Fly.io / reverse proxy handles this)
