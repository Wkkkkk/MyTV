# Ideas

Active backlog of potential improvements. Completed ideas move to
[`CHANGELOG.md`](CHANGELOG.md) (with their rationale); deeper design notes live in
`docs/superpowers/specs/`.

## Open

2. **EPG data from XMLTV** — pull real programme schedules for live channels (SVT, BBC, etc.) so the guide shows what's actually airing.

8. **Channel reorder / drag-and-drop** — allow reordering channels in the admin list via drag-and-drop; requires a `sort_order` column on `channels` and a PATCH endpoint to persist new order.

40. **yt-dlp worker pool: queue metrics + priority** — replace the 2-permit semaphore gate (`resolver::run_under_cap`) with a fixed worker pool fed by a bounded channel: callers send a job + `oneshot` reply channel, N worker tasks consume. A semaphore can't report queue depth or reorder its wait queue; a channel-fed pool gives (1) queue-depth/wait-time metrics for `/admin/metrics`, (2) priority — interactive tune/resolve requests jump ahead of background live-status probes (two channels or a priority-aware dispatcher), (3) load-shedding via `try_send`/bounded wait, preserving today's Busy semantics. Keep the global cap at 2 (memory bound: ~73 MB per yt-dlp process on the 256 MB VM).

43. **Bilibili VOD playback** — tune Bilibili videos (`bilibili.com/video/BV…`) as VOD sources. Playback is currently unsupported — see the documented limitation in `docs/superpowers/plans/2026-06-09-bilibili-support.md` (the earlier Bilibili work only covered the youtube-watcher skill's transcripts). Three known blockers: (1) `resolver::needs_resolution` only routes `youtube.com`/`youtu.be`/`twitch.tv` through yt-dlp, so Bilibili URLs are never resolved; (2) Bilibili requires `--cookies-from-browser chrome` to bypass HTTP 412; (3) Bilibili delivers `.m4s` muxed streams (separate video+audio over direct HTTPS), not HLS `.m3u8`/DASH `.mpd`, which the `<video>`/hls.js/dash.js player can't play directly. Needs: add `bilibili.com` to the resolver host list, thread cookies through the yt-dlp call, and either request a DASH/HLS-compatible format from yt-dlp or mux the `.m4s` audio+video — the format-selection question is the open unknown (see the test plan `docs/superpowers/specs/2026-06-09-bilibili-support-test-plan-design.md`).

## Done

See [`CHANGELOG.md`](CHANGELOG.md) — 41 completed ideas (foundational work + backlog #9–#42).
