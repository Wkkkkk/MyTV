# Health Checker Refactor — Design

**Date:** 2026-06-08
**Idea:** #23

## Problem

Two intertwined code-quality issues in `src/health.rs`:

1. **Duplication across four functions.** `check_source`, `probe_source`, `check_playlist_item`, and `probe_playlist_item` share the same lifecycle (do_http_check → update_health → log action → probe CORS), differing only in: which DB update function they call, and whether they run `process_result` (background checker) or a simple failure increment (admin Test button). Any health-check behavior change requires four coordinated edits.

2. **Missing CORS host deduplication.** When playlist-item health checking was added, the old `probed_hosts` dedup set was dropped. Every playlist item now generates an individual `probe_and_cache_cors` call per 15-minute cycle. A VOD channel with N episodes on one CDN sends N CORS HEAD probes instead of 1.

## Scope

All changes confined to `src/health.rs`. No schema changes, no new files, no public API signature changes (`probe_source`, `probe_playlist_item`, `start`, `probe_and_cache_cors` keep their current signatures).

## Design

### 1. `run_check` helper

Extract a private `async fn run_check` replacing the duplicated bodies of the four functions:

```rust
async fn run_check<F, Fut>(
    client: &reqwest::Client,
    url: &str,
    kind: &str,
    id: i64,
    is_active: bool,
    consecutive_failures: i64,
    manage_lifecycle: bool,
    update: F,
) -> bool
where
    F: FnOnce(&str, Option<&str>, i64, Option<bool>) -> Fut,
    Fut: std::future::Future<Output = Result<(), sqlx::Error>>,
```

Returns `ok` (bool) so the caller controls CORS probing.

Internal steps:
1. `do_http_check(client, url, kind)` → `(ok, reason)`
2. If `manage_lifecycle`: `process_result(is_active, consecutive_failures, ok)` → `(new_failures, action)`, derive `is_active_change: Option<bool>` from `action`; else `new_failures = if ok { 0 } else { consecutive_failures + 1 }`, `is_active_change = None`
3. Call `update(status, reason, new_failures, is_active_change)` — a capturing closure over `pool` and `id` provided by the caller; return early (`false`) on error
4. If `manage_lifecycle`: log disable/re-enable from `action`
5. Return `ok`

The four functions become thin wrappers:

```rust
async fn check_source(pool, client, src) -> bool {
    run_check(client, &src.url, src.kind.as_str(), src.id,
              src.is_active, src.consecutive_failures,
              /*manage_lifecycle=*/ true,
              |s, r, f, a| source::update_health(pool, src.id, s, r, f, a)).await
}

pub async fn probe_source(pool, client, cors_cache, src) {
    let ok = run_check(client, &src.url, src.kind.as_str(), src.id,
                       src.is_active, src.consecutive_failures,
                       /*manage_lifecycle=*/ false,
                       |s, r, f, a| source::update_health(pool, src.id, s, r, f, a)).await;
    if ok { probe_and_cache_cors(client, cors_cache, &src.url).await; }
}
```

Same pattern for `check_playlist_item` and `probe_playlist_item`, using `playlist_item::update_health`. Note: playlist items do not store a `kind` field — the wrapper must compute it as `SourceKind::detect(&item.url).as_str()` before calling `run_check`, matching the current `check_playlist_item` behavior.

### 2. CORS dedup in `check_all`

A single `probed_hosts: HashSet<String>` shared across both the source loop and the playlist-item loop:

```rust
async fn check_all(pool, client, cors_cache) {
    let mut probed_hosts: HashSet<String> = HashSet::new();

    for src in sources {
        let ok = check_source(pool, client, &src).await;
        if ok {
            let host = hls::extract_manifest_host(&src.url);
            if probed_hosts.insert(host) {
                probe_and_cache_cors(client, cors_cache, &src.url).await;
            }
        }
    }

    for item in items {
        let ok = check_playlist_item(pool, client, &item).await;
        if ok {
            let host = hls::extract_manifest_host(&item.url);
            if probed_hosts.insert(host) {
                probe_and_cache_cors(client, cors_cache, &item.url).await;
            }
        }
    }
}
```

`check_source` and `check_playlist_item` no longer call `probe_and_cache_cors` — they return `ok` and the dedup loop in `check_all` decides. `probe_source` and `probe_playlist_item` (admin Test button) are unaffected — they call `probe_and_cache_cors` directly as before.

`hls::extract_manifest_host` is already public — no new function needed.

### 3. Error handling

No behavior changes. `run_check` returns `false` and exits early if `update` fails, same as today. The `manage_lifecycle` flag does not affect error paths.

## Testing

All existing tests pass unchanged (public API signatures are unchanged).

Two new tests:

1. **`run_check` with `manage_lifecycle: false`** — verify `is_active` is never passed as `Some(...)` to the update closure even when the source is unhealthy (covers the probe path preserving admin-set state). This can be implemented as a lightweight unit test using a fake in-memory server and an in-memory DB, asserting `src.is_active` is unchanged after the call.

2. **CORS dedup in `check_all`** — two playlist items sharing the same CDN host; spin up one real TCP server, count accepted connections, assert exactly 1 CORS HEAD probe connection was made rather than 2. Uses the same real-server pattern as `probe_source_does_not_reenable_disabled_source`.
