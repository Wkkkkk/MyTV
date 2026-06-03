# DRY / Code Health Round 2 — Design Stub

> **Status:** Stub — needs brainstorming before implementation.

## Problem

The PR bug review surfaced several duplication and design-smell findings that were left out of the previous code-health refactor (U14). These are correctness-neutral but make future changes risky.

## Findings to address

### CORS pipeline duplication (HIGH)

**`src/routes/player.rs:167`** — `resolve_direct_segments` re-implements the fetch-manifest → descend → probe-CORS → write-cache pipeline that was just centralised in `health::probe_and_cache_cors`. A bug fix in one path silently diverges from the other.

### Allowlist constants (MEDIUM)

- **Channel type** `["live", "vod_loop"]` is hard-coded in three places: `model/channel.rs::create`, the `channel_create` handler, and the `channel_update` handler — no shared constant.
- **Source kind** `["hls", "youtube_live", "iptv"]` is duplicated across `model/source.rs::create`, `routes/admin/sources.rs`, and `routes/admin/discover/add.rs`.

### Duration auto-fetch duplication (MEDIUM)

The branch `if resolver::needs_resolution → fetch_duration_secs; else → fetch_hls_duration` is copied verbatim in both `playlist_item_create` and `do_discover_add` with no shared helper.

### Identical guide template structs (MEDIUM)

`GuidePageTemplate` and `EpgContentTemplate` in `src/routes/guide/mod.rs` have identical fields and only differ by template path. The `guide_template!` macro works around the duplication rather than eliminating it.

### `health::start` client coupling (MEDIUM)

`health::start` accepts a bare `reqwest::Client` (the general `http_client`) instead of `AppState`, making it easy to accidentally pass `proxy_client`, which has `redirect::Policy::none()` and shorter timeouts — wrong for health checks.

### Minor (LOW)

- `apply_budget` is a mutable post-construction method on view-model structs; badge state should be set at construction time via `From<(Source, &CorsCache)>`.
- `detect_source_kind` in `discover/mod.rs` duplicates the YouTube/HLS/IPTV classification already validated by `model/source.rs`.
- Two raw `sqlx::query_scalar` calls for distinct source-id sets are inlined in `build_guide_data` (`guide/data.rs`) — belong in `model/source.rs`.

## Questions to answer before designing

- For the CORS pipeline: should `resolve_direct_segments` just call `health::probe_and_cache_cors`, or should the probe logic be moved fully into `media/hls.rs` so neither `health` nor `player` owns it?
- For the allowlist constants: module-level `const` arrays in `model/`, or a dedicated `ChannelType` / `SourceKind` enum with `FromStr`?
- Should `health::start` be changed to accept `AppState`, or is a dedicated `HealthClients` struct cleaner?

## Rough approach options

A. Targeted fixes only — shared constants + call `probe_and_cache_cors` from `resolve_direct_segments` + extract duration helper. Low disruption.
B. Enum-driven — replace string allowlists with `ChannelType` / `SourceKind` enums with `FromStr`; eliminates the duplication class entirely but touches more files.
