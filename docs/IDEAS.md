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
11. **CORS probe: descend into master playlists + manual Test-button trigger** — the background CORS probe and guide budget badge are dead for master-playlist streams (the common case): `find_first_segment_url` returns `None` on a master manifest, so `probe_cors_for_source` never populates `cors_cache` and HTTPS+CORS channels stay `Unknown` (no ⚡). (A) Make the probe descend one level — master → first variant `.m3u8` → first segment → HEAD-probe — so the background health cycle actually caches a result. (B) Wire the same probe into the per-source admin **Test** button (`source_test`) so a probe can be forced on demand and the result (Direct ⚡ / Proxied ☁) shown inline. Also fix the cache-key mismatch where the lazy probe in `player.rs` keys by the variant host while the guide reads by the source-URL host.
12. **CORS budget badge for VOD channels** — VOD channels store URLs in `playlist_items`, not `sources`, so `build_guide_data` (which derives budget only from the `sources` table) always yields `Unknown` → no badge for VOD. Extend budget derivation to cover VOD playlist-item URLs, and add a probe trigger for playlist items (VOD items have no per-source Test button today). Depends on idea 11A's descend-into-master probe.
