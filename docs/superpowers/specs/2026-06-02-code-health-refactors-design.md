# Code-health refactors for `discover.rs`, `guide.rs`, `player.rs`, `hls.rs`

**Date:** 2026-06-02
**Idea:** IDEAS.md #14 — Code-health refactors
**Type:** Pure refactor — **zero behavior change**

## Goal

Two route modules have grown too large and tangle several responsibilities:

- `src/routes/admin/discover.rs` (~890 lines): M3U fetch/parsing, YouTube API, manual
  resolve, DB writes, and templating in one file.
- `src/routes/guide.rs` (~790 lines): EPG geometry, health + budget derivation, DB
  aggregation, and rendering in one file.

Split each into focused submodules with clear boundaries. Plus two smaller
duplication cleanups:

- `src/routes/player.rs`: four near-identical `tune_*`/`next_*` `TuneResponse`
  builders.
- `src/media/hls.rs`: origin/base-dir URL-parsing logic copy-pasted 3×.

## Non-goals

- No route, response, SQL, or template-output changes.
- No new features or behavior.
- No unrelated refactoring beyond the four targets.

## Constraints

- Public API surface must stay stable so wiring is untouched:
  - discover handlers are re-exported via `src/routes/admin/mod.rs`
    (`pub use discover::{...}`).
  - `guide_page` / `guide_partial` are referenced by path in `src/lib.rs`
    (`routes::guide::guide_page`).
  - player handlers (`tune`, `next`, `stream_proxy`) referenced by path in `lib.rs`.
  Converting a `foo.rs` file into a `foo/mod.rs` directory that re-exports the same
  names keeps every reference and integration test working.
- `cargo fmt` before every commit (CI fails on diff; toolchain pinned to 1.96).
- The existing 117 tests (102 unit + 15 integration) are the regression net. Tests
  move alongside the code they cover; no test logic changes.

---

## Part 1 — `routes/admin/discover.rs` → `discover/` directory

Split along the existing `// ──` section seams.

| File | Contents |
|------|----------|
| `discover/mod.rs` | `mod`/`pub use` re-exports (keeps `routes::admin::discover::*` stable); the 6 axum handlers (`discover_page`, `discover_add_form`, `discover_add`, `discover_m3u_search`, `discover_youtube_search`, `discover_manual_resolve`); their form/query types (`M3uSearchForm`, `YoutubeSearchForm`, `ManualResolveForm`, `AddFormQuery`, `AddForm`); template structs (`DiscoverPageTemplate`, `DiscoverAddFormTemplate`, `M3uResultsTemplate`, `YtResultsTemplate`, `ManualResultTemplate`); `DiscoverChannelOption`; `detect_source_kind` |
| `discover/add.rs` | `DiscoverAddParams`, `do_discover_add` + the 6 `do_discover_add` tests |
| `discover/youtube.rs` | `fetch_youtube_results`, `parse_iso8601_duration`, `YoutubeResultRow` + the `parse_iso8601_duration` test |
| `discover/m3u.rs` | `fetch_m3u`, `country_to_code`, `url_is_reachable`, `M3uResultRow` (the ~80-line country table) |

Notes:
- `detect_source_kind` stays in `mod.rs` (used by both the m3u and manual handlers);
  its test moves to `mod.rs`'s test module.
- `M3uResultRow` / `YoutubeResultRow` live with their fetch logic (m3u.rs / youtube.rs)
  and are `pub(super)` or `pub` as needed by the handlers and templates in mod.rs.
- Handlers reach submodule functions via `use super::...` / module paths.
- `do_discover_add` and `DiscoverAddParams` remain `pub` (re-exported from mod.rs;
  used by tests).

## Part 2 — `routes/guide.rs` → `guide/` directory

Split along responsibility seams.

| File | Contents |
|------|----------|
| `guide/mod.rs` | re-exports; `GuideQuery`; the 2 handlers (`guide_page`, `guide_partial`); both template structs (`GuidePageTemplate`, `EpgContentTemplate`) |
| `guide/layout.rs` | pure geometry + display types: `compute_window`, `entry_to_slot`, `now_line_pct`, `time_labels`, `ProgramSlot`, `TimeLabel` + their ~12 tests |
| `guide/badges.rs` | `HealthStatus`, `category_icon`, `derive_health_status`, `budget_for_url`, `vod_budget_url`, `health_badge` + their ~12 tests |
| `guide/data.rs` | `GuideData`, `ChannelRow`, `build_guide_data` (DB aggregation) |

Notes:
- The `pub` geometry helpers (`compute_window`, `entry_to_slot`, `now_line_pct`,
  `time_labels`) stay `pub` and are re-exported from mod.rs to preserve the surface.
- `ChannelRow` lives in `data.rs` (constructed by `build_guide_data`), re-exported so
  the template structs in mod.rs can name it.

Two behavior-preserving micro-dedups inside guide:
- `guide_page` and `guide_partial` parse `category`/`offset` identically — extract into
  one private helper (e.g. `parse_query(params) -> (String, i64)`).
- Both handlers copy all 9 `GuideData` fields into a template verbatim. Collapse the
  repeat with a small declarative macro (e.g. `build_template!(TemplateType, data)`) or
  a generic field-mapping helper, so the 9-line copy exists once. Output is byte-for-byte
  identical.

## Part 3 — `routes/player.rs` `TuneResponse` dedup

Collapse the four near-identical builders:
- `fn tune_response(ch: &Channel, url: String, start_offset_secs: i64) -> Json<TuneResponse>`
  — the single place the 6 channel fields are cloned.
- Live: `tune_live(state, ch)` becomes `next_live(state, ch, None)` — they differ only by
  the `failed_url` filter, which is a no-op when `None`.
- VOD: extract the shared prelude (`loop_anchor` → `list_for_channel` →
  `current_position`, including the empty-playlist `503`) into one helper returning the
  items + resolved index. `tune_vod_at` uses `(idx, offset)`; `next_vod_at` uses
  `((idx + 1) % len, 0)`.

Net: 4 response constructions → 1. All existing player tests pass unchanged.

## Part 4 — `media/hls.rs` URL-parsing extraction

The origin + base-dir + "resolve a manifest line to an absolute URL" logic is copy-pasted
across `rewrite_hls_urls`, `resolve_uri`, and `find_first_segment_url`. `resolve_uri`
already encapsulates exactly that resolution.

- Add a private `origin_of(base_url: &str) -> &str` returning the `scheme://host` slice;
  use it inside `resolve_uri` and refactor `extract_manifest_host` to reuse it.
- Have `rewrite_hls_urls` and `find_first_segment_url` call `resolve_uri` instead of
  re-implementing the http / `/`-absolute / relative branch.

Net: 3 copies → 1 shared resolver. Existing hls tests are the safety net.

---

## Verification

- `cargo test` — all 117 tests green (tests move files but keep identical logic).
- `cargo clippy -- -D warnings` — clean.
- `cargo fmt --check` — clean.
- Spot-check that `cargo build` produces no new warnings and the route table in
  `lib.rs` / `routes/admin/mod.rs` is unchanged.

## Rollout

Single branch, four commits (one per part) or one cohesive commit — each part is
independently verifiable. No migration, no deploy-time risk.
