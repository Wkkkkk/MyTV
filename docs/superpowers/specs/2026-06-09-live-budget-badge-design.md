# Budget Badge for YouTube/Twitch Live Streams

**Date:** 2026-06-09
**Idea:** #37

## Problem

YouTube/Twitch live sources resolve via yt-dlp to an HLS `.m3u8` manifest, but
`health::probe_and_cache_cors` early-returns `None` for any `needs_resolution()`
URL (`health.rs:250`) — there is no stable manifest to HEAD-probe before
resolution. Consequently live streams never get a CORS cache entry and the guide
shows a **blank** budget badge, unlike VOD (idea #34, now Direct ⚡) and plain
HLS/DASH sources (⚡/☁).

Unlike VOD (which skips the proxy entirely), live HLS **still proxies the
manifest**; segments load direct from the `googlevideo.com` CDN only when that
CDN sends `Access-Control-Allow-Origin: *`. So the badge is genuinely meaningful
here — it is not always Direct.

### Why the badge cannot render from the resolved host alone

The guide never sees the resolved URL. yt-dlp resolution happens only at tune
time and at Test time; the guide renders straight from the DB. For a live
channel, `build_guide_data` (`routes/guide/data.rs:109`) uses
`first_active_urls.get(&ch.id)` — the **DB source URL** (`youtube.com/live/<id>`)
— as `budget_url`, then:

```
budget_for_url(budget_url)
  → status_for_url(url)
    → cors_cache.get(&extract_manifest_host(url))   // budget.rs:18
```

`extract_manifest_host("https://www.youtube.com/live/<id>")` = `https://www.youtube.com`.
So the **only** cache key the guide ever queries with is `https://www.youtube.com`.
The admin source row uses the same lookup (`AdminSourceRow::apply_budget` keys off
`src.url`). If the probe result lived solely under the resolved CDN host
(`https://rr3---sn-xyz.googlevideo.com`), both lookups return `None` →
`BudgetStatus::Unknown` → blank badge, and idea #37 fails silently.

## Approach

On a deliberate admin **Test** of a live source whose URL `needs_resolution()`:
resolve via yt-dlp, descend the resolved manifest (master → variant → segment),
HEAD-probe the segment CDN's CORS, and cache the result under **both** the
resolved CDN host and the original source host.

The background 15-min sweep stays unchanged — resolving every live source per
cycle is too expensive. The badge is set on deliberate admin action, exactly
like the VOD case (idea #34).

Caching under both hosts is the established pattern: `probe_and_cache_cors`
already inserts under the probed host *and* the manifest host when they differ
(`health.rs:264–269`), "so existing lookups by source/playlist URL continue to
work." The resolved-CDN-host entry is the semantically true key (googlevideo's
CORS is what actually governs direct-vs-proxy at playback); the original-host
entry is the bridge that lets the guide and admin row — which only know the DB
URL — find it.

## Design

### New helper — `src/health.rs`

Mirrors `probe_and_cache_cors`, testable in isolation:

```rust
/// Resolves a YouTube/Twitch live source via yt-dlp, probes the resolved HLS
/// manifest's segment-CDN CORS, and caches the result under BOTH the resolved
/// CDN host and the original source host. Returns `None` (cache unchanged) if
/// resolution fails or the resolved URL is not a probeable HLS manifest.
pub async fn probe_and_cache_resolved_cors(
    client: &reqwest::Client,
    cors_cache: &CorsCache,
    source_url: &str,
) -> Option<bool> {
    let resolved = crate::media::resolver::resolve_url(source_url).await.ok()?;
    let cors = crate::media::hls::probe_source_cors(client, &resolved).await?;

    let resolved_host = crate::media::hls::extract_manifest_host(&resolved);
    let original_host = crate::media::hls::extract_manifest_host(source_url);

    let mut cache = cors_cache.write().await;
    cache.insert(resolved_host.clone(), cors);
    if original_host != resolved_host {
        cache.insert(original_host, cors);
    }
    Some(cors)
}
```

Notes:
- `resolve_url` returns the resolved URL unchanged for non-resolution URLs, but
  this helper is only invoked when the caller has already checked
  `needs_resolution`.
- `probe_source_cors` returns `None` if the resolved URL is not an HLS manifest
  with a discoverable segment (e.g. a finished-live MP4). In that case the cache
  is left untouched and the badge stays blank — VOD-conversion of finished live
  streams is idea #36's concern, out of scope here.

### Wire into `source_test` — `src/routes/admin/sources.rs`

After the existing `probe_source` call (`sources.rs:96`), add a resolution
branch mirroring `playlist_item_test`:

```rust
crate::health::probe_source(&state.pool, &state.http_client, &state.cors_cache, &src).await;

if crate::media::resolver::needs_resolution(&src.url) {
    crate::health::probe_and_cache_resolved_cors(
        &state.http_client, &state.cors_cache, &src.url,
    ).await;
}
```

The existing `row.apply_budget(&cors)` then picks up ⚡/☁ and re-renders the
source row via `SourceRowTemplate`, identical to the VOD/HLS Test flow. No
template, route, or schema change.

## Testing

- **Unit (`health.rs`):** `probe_and_cache_resolved_cors` — assert that on a
  successful probe the result is cached under both the resolved host and the
  original host (drive via the cache-insertion logic; the yt-dlp/network paths
  follow the existing `probe_and_cache_cors` test style — verify the `None`
  early-return when resolution or probe fails leaves the cache unchanged).
- **Integration (`tests/http.rs`):** existing `source_test` coverage exercises
  the non-resolution path; add a case asserting a `needs_resolution` source URL
  routes through the new branch (the handler returns 200 and re-renders the row;
  with yt-dlp absent in CI the probe returns `None` and the badge stays blank,
  which is the correct degraded behavior).

## Scope (YAGNI)

- No background-sweep change.
- No schema change, no new route, no template change.
- Strictly live HLS. Resolved-MP4 / finished-live handling is idea #36.
- Reuses the existing Test button, CORS cache, `resolve_url`, and
  `probe_source_cors` unchanged.
