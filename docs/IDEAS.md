# Ideas

Backlog of potential improvements, roughly in priority order.

## Planned

4. ~~**Source health monitoring**~~ — done: background checker, health badges in admin, guide warnings
5. ~~**yt-dlp auto-update**~~ — done: `.github/workflows/ytdlp-update.yml` weekly PR workflow
6. ~~**Player UX + mobile/TV UI**~~ — done: channel info bar (logo, name, category, position), keyboard shortcuts (space/arrows/F), fullscreen, responsive CSS for phone and TV, channel logos

## Other ideas

2. **EPG data from XMLTV** — pull real programme schedules for live channels (SVT, BBC, etc.) so the guide shows what's actually airing
3. ~~**Markdown docs with Mermaid charts**~~ — done: architecture docs in `docs/architecture/` covering request flow, health checker, tune flow, yt-dlp resolution, database ER diagram
6. ~~**SQL indexes on foreign keys**~~ — done: compound indexes `(channel_id, priority)` on `sources` and `(channel_id, sort_order)` on `playlist_items`
4. ~~**Multi-agent PR bug review**~~ — done: `pr-bug-review` skill with parallel correctness/security/architecture agents + synthesis pass
5. ~~**Code quality review checklist**~~ — done: `self-review` skill with 13 universal checks (KISS/DRY, HTML a11y, SQL indexes, N+1, dead code, test gaps)
7. ~~**Source auto-re-enable after cooldown**~~ — done: disabled sources are automatically re-enabled on the first passing health check (`HealthAction::Reenable`, no schema change needed)
8. **Channel reorder / drag-and-drop** — allow reordering channels in the admin list via drag-and-drop; requires a `sort_order` column on `channels` and a PATCH endpoint to persist new order
10. ~~**Stream proxy: automatic CORS detection**~~ — done: HLS segments load directly from origin CDN when the CDN sends `Access-Control-Allow-Origin: *`; manifests always proxy. CORS probed on first tune and on each 15-min health check cycle; cached per `scheme://host`. EPG guide shows health (green/red/grey ●) and budget (blue ⚡ / amber ☁) badges per channel.
9. **Favicon and PWA manifest** — add a favicon (`.ico` + PNG sizes) so the browser tab shows a recognizable icon, and a `manifest.json` that declares the app name, theme color, icons, and `display: standalone`. With the manifest linked from every page, browsers on Android/iOS show an "Add to Home Screen" prompt; once installed the app opens without browser chrome (no address bar, no tabs) and appears in the app switcher like a native app. On desktop Chrome/Edge it can also be installed as a standalone window. No service worker or offline support needed — the manifest alone unlocks installability.
