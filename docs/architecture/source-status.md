# Source Status model

MyTV shows two indicators per source/item: **Status** and **Budget**.

## Two axes
- **Status** (`src/status.rs`, `SourceStatus`) — *availability*: is this source usable right now? Folds the former Active + Health + Live indicators into one value.
- **Budget** (`src/budget.rs`) — *network cost*: can the browser reach the CDN directly (CORS ⚡) or must we proxy (☁)?

## Intent vs observation
- `is_active` is **manual intent only** — the admin's switch. The health checker never mutates it (`health::process_failures` records `last_status`/`consecutive_failures`/`failure_reason`; it does not disable or re-enable).
- Observed availability comes from persisted health (regular sources) and the cached `LiveStatus` (`youtube_live`).

## Status precedence (`status::compute`)
1. `Disabled` — `is_active = false`.
2. `youtube_live` → from cached `LiveStatus`: Live / Upcoming / Recorded (was/post-live) / Offline / Unchecked (cold cache or Unknown). Never `Down`.
3. regular / VOD → from `last_status`: `Down` (any `error`, with reason) / `Ok` / `Unchecked`.

## Tune gating (`source::list_tunable_for_channel`)
`is_active = 1 AND NOT (kind != 'youtube_live' AND last_status = 'error' AND consecutive_failures >= 3)`. Regular Down sources are skipped at read time (rejoin automatically on recovery, no `is_active` write); `youtube_live` stays in rotation so the resolve-time waiting/backoff (idea #38) can fire.

## Guide aggregation (`badges::derive_channel_status`)
The channel badge is the **most-optimistic** status across its sources: `Live`=`Ok` > `Upcoming` > `Recorded` > `Offline` > `Unchecked` > `Down` > `Disabled`. The guide reads only persisted health + the warm live-status cache; it never probes.
