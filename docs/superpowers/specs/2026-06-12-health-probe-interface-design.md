# Spec — Narrow the health probe interface

_Candidate #2 of the architecture-deepening effort (`docs/architecture/changes-20260612.md` §2).
Created 2026-06-12._

## Problem

`src/health.rs` exposes five public probe entry points:

- `probe_source` — test-button health probe for a source (health row + CORS cache)
- `probe_playlist_item` — test-button health probe for a playlist item
- `probe_and_cache_cors` — CORS-only probe for one URL
- `probe_and_cache_resolved_cors` — resolve a live source via yt-dlp, then probe the resolved CDN's CORS
- `record_source_liveness` — record a liveness signal from the tune path (no CORS, never touches `is_active`)

The interface is nearly as tall as the implementation, and **callers must pick the path**.
Worse, two callers replicate a "this source needs resolution first" branch that the
implementation should own:

| Caller | Calls | Replicated branch |
|--------|-------|-------------------|
| `admin/sources.rs:89` | `probe_source` | then `probe_and_cache_resolved_cors` **if `needs_resolution(url)`** |
| `admin/playlist.rs:124` | `probe_playlist_item` | then inline `cors_cache.insert(host, true)` **if `needs_resolution(url)`** |
| `api/sources.rs:116` | `probe_source` | — (no resolved-CORS step — diverges from admin) |
| `api/playlist.rs:125` | `probe_playlist_item` | — (no VOD host mark — diverges from admin) |
| `player.rs:409` | `probe_and_cache_cors` (CORS-only) | — |
| `player.rs:191` | `record_source_liveness` | — (separate concern) |

Two leaks: callers choose which probe fn applies, and the admin handlers hand-roll a
dispatch branch that the JSON API handlers silently omit — an unintended behavioral
divergence between the two test paths.

## Solution

A single public `probe(target)` that classifies the target and dispatches every downstream
step internally. Callers stop branching.

### Public surface: 5 → 3

| Keep | Responsibility |
|------|----------------|
| `probe(target)` | health probe + CORS cache + (resolved-CORS if live) + (VOD-host-mark if needs-resolution). Absorbs the dispatch. |
| `probe_and_cache_cors(url)` | CORS-only, no DB. The player's cache-miss path has only a URL — keep it narrow. |
| `record_source_liveness(src, ok)` | Separate concern (liveness signal from the tune path, never touches CORS). Untouched. |

`probe_and_cache_resolved_cors` becomes **private** — absorbed into `probe`.

### The target enum + signature

```rust
pub enum ProbeTarget<'a> {
    Source(&'a Source),
    PlaylistItem(&'a PlaylistItem),
}

pub async fn probe(
    pool: &SqlitePool,
    client: &reqwest::Client,
    cors_cache: &CorsCache,
    live_cache: &LiveStatusCache,
    target: ProbeTarget<'_>,
)
```

`live_cache` is consulted only for the `Source` variant (playlist items are never
`youtube_live` broadcasts); the `PlaylistItem` variant ignores it. Positional deps match the
existing `probe_source`/`probe_playlist_item` style.

### Internal dispatch

```text
match target {
    Source(src) => {
        let ok = run_check(.., src.url, src.kind, .., Some(live_cache), update_source_health);
        if ok { probe_and_cache_cors(client, cors_cache, &src.url).await; }
        if needs_resolution(&src.url) {
            probe_and_cache_resolved_cors(client, cors_cache, &src.url).await;
        }
    }
    PlaylistItem(item) => {
        let kind = SourceKind::detect(&item.url);
        let ok = run_check(.., item.url, kind, .., None, update_item_health);
        if ok { probe_and_cache_cors(client, cors_cache, &item.url).await; }
        if needs_resolution(&item.url) {
            // VOD items skip the proxy entirely, so their host is Direct outright.
            cors_cache.write().await.insert(extract_manifest_host(&item.url), true);
        }
    }
}
```

This is exactly the logic that today lives partly in `probe_source`/`probe_playlist_item`
and partly hand-rolled in the admin callers. It now lives in one place.

### Return type

`()` — side-effects only (DB health row + CORS cache), exactly as `probe_source` /
`probe_playlist_item` return today. Callers already re-fetch the updated row and re-read the
cache to render their response, so a `ProbeOutcome` struct would not remove that read.
(Deliberate deviation from the review doc's tentative `-> ProbeOutcome` — YAGNI.)

## Behavioral reconciliation (approved)

`probe(target)` always runs the full pipeline, so the JSON API test handlers gain the
resolved-CORS / VOD-host-mark side effects that were previously admin-only:

- `api/sources.rs::test` on a live (needs-resolution) source now spawns a yt-dlp resolve to
  warm the budget cache — the same cost the admin Test button already pays. Rare, explicit
  user action, gated by the existing 2-permit `run_under_cap` semaphore.
- `api/playlist.rs::test` on a needs-resolution VOD item now marks its host Direct in the
  cache.

Neither changes the JSON response body (which carries the model, not a budget badge); both
are cache-warming side effects that benefit the guide/player regardless of which endpoint
triggered the test. One behavior, no caller branch.

## Out of scope

- **Background sweep (`check_all`).** Its private `check_source` / `check_playlist_item`
  helpers stay private and unchanged. They deliberately do *not* resolve live sources (too
  expensive for a 15-minute sweep) and warm CORS via the host-dedup loop in `check_all`. Only
  the public *test-button* surface is unified; the sweep's internal path is a different
  concern and is not touched.
- **`record_source_liveness`** — separate concern, untouched.
- **`probe_and_cache_cors`** — stays public for the player's cache-miss path, unchanged.

## Callers after

- `admin/sources.rs` → `probe(.., ProbeTarget::Source(&src))`; delete the
  `if needs_resolution { probe_and_cache_resolved_cors(..) }` block.
- `admin/playlist.rs` → `probe(.., ProbeTarget::PlaylistItem(&item))`; delete the inline
  `if needs_resolution { cors_cache.insert(..) }` block.
- `api/sources.rs`, `api/playlist.rs` → `probe(..)` — now identical to admin.
- `player.rs` → unchanged.

## Testing

- Rewrite `probe_source_does_not_reenable_disabled_source` → call
  `probe(ProbeTarget::Source(..))`; assertion unchanged (manual disable preserved).
- Rewrite `probe_playlist_item_does_not_reenable_disabled_item` → call
  `probe(ProbeTarget::PlaylistItem(..))`; assertion unchanged.
- Add a test asserting the `Source` dispatch path invokes the resolved-CORS step for a
  needs-resolution URL (e.g. via observable cache state or a deterministic short-circuit),
  and the `PlaylistItem` path marks the host Direct for a needs-resolution VOD URL.
- All other `health.rs` tests (`live_status_health`, `process_failures`, the DASH/resolved
  cache-key contract tests, `check_all` independence, `probed_hosts` dedup,
  `record_source_liveness`) unchanged.
- Integration: existing `tests/http.rs` / `tests/api.rs` source/playlist `test` endpoints
  must stay green.

## Acceptance criteria

1. `health.rs` exposes exactly three probe-related public fns: `probe`, `probe_and_cache_cors`,
   `record_source_liveness`. `probe_source` and `probe_playlist_item` are removed (bodies fold
   into `probe`); `probe_and_cache_resolved_cors` is made private.
2. No caller of `probe` branches on `needs_resolution` before/after the call.
3. `cargo test` (all targets, incl. `--no-run` to compile lib `#[cfg(test)]` modules),
   `cargo fmt --check`, `cargo clippy --all-targets -- -D warnings` all green.
