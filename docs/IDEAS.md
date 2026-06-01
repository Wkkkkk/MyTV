# Ideas

Backlog of potential improvements, roughly in priority order.

## Planned

1. **Channel logos** — store a logo URL per channel, show it in the EPG guide for visual identity
4. ~~**Source health monitoring**~~ — done: background checker, health badges in admin, guide warnings
5. ~~**yt-dlp auto-update**~~ — done: `.github/workflows/ytdlp-update.yml` weekly PR workflow
6. **Player UX + mobile/TV UI** — keyboard shortcuts (space/arrows), fullscreen, channel info overlay; responsive layout for phone and TV browser

## Other ideas

2. **EPG data from XMLTV** — pull real programme schedules for live channels (SVT, BBC, etc.) so the guide shows what's actually airing
3. **Markdown docs with Mermaid charts** — write architecture/flow documentation in Markdown using Mermaid diagrams (e.g. request flow, health checker lifecycle, route map)
6. **SQL indexes on foreign keys** — add indexes on `channel_id` in `sources` and `playlist_items` tables; both are heavily queried in WHERE/ORDER BY but currently unindexed (flagged by self-review U10)
4. ~~**Multi-agent PR bug review**~~ — done: `pr-bug-review` skill with parallel correctness/security/architecture agents + synthesis pass
5. ~~**Code quality review checklist**~~ — done: `self-review` skill with 13 universal checks (KISS/DRY, HTML a11y, SQL indexes, N+1, dead code, test gaps)
