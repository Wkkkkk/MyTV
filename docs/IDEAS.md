# Ideas

Backlog of potential improvements, roughly in priority order.

## Planned

4. ~~**Source health monitoring**~~ — done: background checker, health badges in admin, guide warnings
5. ~~**yt-dlp auto-update**~~ — done: `.github/workflows/ytdlp-update.yml` weekly PR workflow
6. **Player UX + mobile/TV UI** — spec: `docs/superpowers/specs/2026-06-01-player-ux-design.md`. Channel info bar (logo, name, category, position), keyboard shortcuts (space/arrows/F), fullscreen, responsive CSS for phone and TV. Channel logos folded in here. **Post-implementation: update README to document keyboard shortcuts and channel info bar.**

## Other ideas

2. **EPG data from XMLTV** — pull real programme schedules for live channels (SVT, BBC, etc.) so the guide shows what's actually airing
3. ~~**Markdown docs with Mermaid charts**~~ — done: architecture docs in `docs/architecture/` covering request flow, health checker, tune flow, yt-dlp resolution, database ER diagram
6. ~~**SQL indexes on foreign keys**~~ — done: compound indexes `(channel_id, priority)` on `sources` and `(channel_id, sort_order)` on `playlist_items`
4. ~~**Multi-agent PR bug review**~~ — done: `pr-bug-review` skill with parallel correctness/security/architecture agents + synthesis pass
5. ~~**Code quality review checklist**~~ — done: `self-review` skill with 13 universal checks (KISS/DRY, HTML a11y, SQL indexes, N+1, dead code, test gaps)
7. **Source auto-re-enable after cooldown** — health checker currently auto-disables sources after 3 consecutive failures but never re-enables them; add cooldown logic so sources are re-activated automatically after N successful checks or a time window
