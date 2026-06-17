# Ideas

Active backlog of potential improvements. Completed ideas move to
[`CHANGELOG.md`](CHANGELOG.md) (with their rationale); deeper design notes live in
`docs/superpowers/specs/`.

## Open

2. **EPG data from XMLTV** — pull real programme schedules for live channels (SVT, BBC, etc.) so the guide shows what's actually airing.

8. **Channel reorder / drag-and-drop** — allow reordering channels in the admin list via drag-and-drop; requires a `sort_order` column on `channels` and a PATCH endpoint to persist new order.

40. **yt-dlp worker pool: queue metrics + priority** — replace the 2-permit semaphore gate (`resolver::run_under_cap`) with a fixed worker pool fed by a bounded channel: callers send a job + `oneshot` reply channel, N worker tasks consume. A semaphore can't report queue depth or reorder its wait queue; a channel-fed pool gives (1) queue-depth/wait-time metrics for `/admin/metrics`, (2) priority — interactive tune/resolve requests jump ahead of background live-status probes (two channels or a priority-aware dispatcher), (3) load-shedding via `try_send`/bounded wait, preserving today's Busy semantics. Keep the global cap at 2 (memory bound: ~73 MB per yt-dlp process on the 256 MB VM).

## Done

See [`CHANGELOG.md`](CHANGELOG.md) — 54 completed/closed ideas (foundational work + backlog #9–#55, incl. #43 closed won't-do).
