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
9. ~~**Favicon and PWA manifest**~~ — done: TV+Play SVG favicon at `/favicon.svg`, PWA manifest at `/manifest.json`, both linked from all pages via `<head>` tags.
11. ~~**CORS probe: descend into master playlists + manual Test-button trigger**~~ — done: the CORS probe now descends one level (master → first variant `.m3u8` → first segment → HEAD-probe) in `media::hls::probe_source_cors`, the cache is keyed by source-URL host everywhere (`extract_manifest_host`), `health::check_source` unifies the background health check + CORS probe, and the admin **Test** button runs it and swaps the whole source row showing updated Health + a new Budget column (⚡/☁/blank). Spec: `docs/superpowers/specs/2026-06-02-cors-probe-descent-test-trigger-design.md`.
12. ~~**CORS budget badge for VOD channels**~~ — done: `build_guide_data` derives a VOD channel's budget badge from its currently-playing playlist item (`vod_budget_url` + `playlist_item::current_position`); the CORS cache for item hosts is warmed by a per-item admin **Test** button (`POST /admin/playlist/:id/test`) and a new background-checker sweep (`probe_all_playlist_cors`, deduped by host). The shared `health::probe_and_cache_cors` helper (skips non-HTTPS and youtube/twitch URLs) backs sources, the Test button, and the sweep. Spec: `docs/superpowers/specs/2026-06-02-vod-budget-badge-design.md`.
