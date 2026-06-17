# Ideas

Active backlog of potential improvements. Completed ideas move to
[`CHANGELOG.md`](CHANGELOG.md) (with their rationale); deeper design notes live in
`docs/superpowers/specs/`.

## Open

2. **EPG data from XMLTV** — pull real programme schedules for live channels (SVT, BBC, etc.) so the guide shows what's actually airing.

8. **Channel reorder / drag-and-drop** — allow reordering channels in the admin list via drag-and-drop; requires a `sort_order` column on `channels` and a PATCH endpoint to persist new order.

40. **yt-dlp worker pool: queue metrics + priority** — replace the 2-permit semaphore gate (`resolver::run_under_cap`) with a fixed worker pool fed by a bounded channel: callers send a job + `oneshot` reply channel, N worker tasks consume. A semaphore can't report queue depth or reorder its wait queue; a channel-fed pool gives (1) queue-depth/wait-time metrics for `/admin/metrics`, (2) priority — interactive tune/resolve requests jump ahead of background live-status probes (two channels or a priority-aware dispatcher), (3) load-shedding via `try_send`/bounded wait, preserving today's Busy semantics. Keep the global cap at 2 (memory bound: ~73 MB per yt-dlp process on the 256 MB VM).

50. **Collapse the model-layer CRUD triplication** *(architecture deepening, 2026-06-16; reopened 2026-06-17)* — `source.rs`, `playlist_item.rs`, and `channel.rs` repeat the same skeleton (`Row`/`NewRow`/`UpdateRow`/`Input` + `create`/`get`/`list`/`delete`/`set_active`/`update`) — ~2,000 lines of mostly SQL-binding boilerplate. **Low-risk first step DONE (2026-06-16):** the string-parameterized `model::update_health_sql` (which `format!`'d a table name into SQL — the *only* dynamic-table-name query in the codebase — shared across two tables) is retired; `source::update_health` and `playlist_item::update_health` now each inline a literal-table-name query, killing the silent field-skew hazard. **Note on the broader collapse:** the codebase uses runtime `sqlx::query(...)`/`query_as::<_, T>(...)` with literal SQL everywhere (no compile-time-checked `query_as!`, no `.sqlx` offline cache, no build-time `DATABASE_URL`), so a generic `CrudModel<T>` trait gives up no checking that exists today — but it would violate the real constraint of keeping each query a self-contained *literal* string. A per-table *macro* expanding literal queries is the candidate path. **Under active investigation (2026-06-17).**

## Done

See [`CHANGELOG.md`](CHANGELOG.md) — 53 completed/closed ideas (foundational work + backlog #9–#55 minus #50, incl. #43 closed won't-do; #50 reopened).
