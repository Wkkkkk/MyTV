# Skip Stream-Proxy for YouTube VOD Direct URLs

**Date:** 2026-06-09
**Idea:** #34

## Problem

`proxyUrl()` in `base.html` wraps every external URL through `/stream-proxy` before playback, including the `googlevideo.com` MP4 URLs that yt-dlp resolves for YouTube VOD. This routes all video bytes through the Fly.io server (~450 MB/hour at 720p), consuming egress bandwidth unnecessarily.

YouTube CDN (`googlevideo.com`) supports Range requests natively. A `<video src>` element does not require CORS headers — the browser loads media elements in `no-cors` mode — so no proxy is needed for direct MP4 playback.

Other direct MP4 sources (e.g. Bilibili) may need the proxy, so we cannot unconditionally skip it in the `else` branch.

## Approach

**Server-side flag in `TuneResponse`.** Add `skip_proxy: bool` to the JSON response. The server sets it to `true` when the source URL required yt-dlp resolution (YouTube, Twitch). The client checks this flag before calling `proxyUrl()` in the direct MP4 branch.

Alternatives considered:
- **Client-side hostname check** (`googlevideo.com`) — rejected: brittle, hardcodes a CDN domain that could change silently.
- **CORS cache lookup** — rejected: overcomplicated; CORS budget tracks fetch-based requests, not `<video src>` elements which don't need CORS.

## Design

### Server — `src/routes/player.rs`

Add `skip_proxy: bool` to `TuneResponse`:

```rust
pub struct TuneResponse {
    pub url: String,
    pub start_offset_secs: i64,
    pub name: String,
    pub logo_url: Option<String>,
    pub category: String,
    pub channel_type: String,
    pub skip_proxy: bool,   // true when url was resolved via yt-dlp
}
```

Add `skip_proxy: bool` parameter to `tune_response()`:

```rust
fn tune_response(ch: &channel::Channel, url: String, start_offset_secs: i64, skip_proxy: bool) -> Json<TuneResponse>
```

At each of the three resolution call sites, pass `resolver::needs_resolution(&src.url)` (or `&item.url`):

| Function | Call site | Value |
|---|---|---|
| `next_live` | line 97 | `resolver::needs_resolution(&src.url)` |
| `tune_vod_at` | line 137 | `resolver::needs_resolution(&item.url)` |
| `next_vod_at` | line 154 | `resolver::needs_resolution(&item.url)` |

`resolver::needs_resolution` is already `pub` and checks for `youtube.com`, `youtu.be`, `twitch.tv`. No changes to `resolver.rs`.

Note: Twitch sources also get `skip_proxy: true`. This is harmless — Twitch yt-dlp output is HLS (`.m3u8`), so the `isHls` branch fires first and `skipProxy` is never consulted in the `else` branch.

### Client — `templates/base.html`

**`_loadSource` signature:** add `skipProxy` third parameter (falsy when absent — all existing call sites safely default to proxied behaviour).

**`else` branch:** use `currentUrl` (the original unproxied URL, already stored at line 227) instead of `url` (the proxied URL) when `skipProxy` is true:

```javascript
function _loadSource(url, offset, skipProxy) {
  currentUrl = url;                          // original url stored here
  var isDash = url.indexOf('.mpd') >= 0;
  var isHls  = url.indexOf('.m3u8') >= 0;
  url = proxyUrl(url);                       // unchanged — DASH/HLS always proxy

  if (isDash) {
    // ... unchanged
  } else if (isHls) {
    // ... unchanged
  } else {
    // Direct MP4: skip proxy for yt-dlp-resolved URLs
    video.src = skipProxy ? currentUrl : url;
    // ... rest unchanged
  }
}
```

**All 6 call sites** updated to pass `d.skip_proxy`:

```javascript
_loadSource(d.url, d.start_offset_secs, d.skip_proxy)
```

Call sites are at lines 213, 245, 278, 305, 349, 363.

## Budget Badge on Test for yt-dlp VOD Items

When an admin clicks **Test** on a playlist item, `playlist_item_test` runs `probe_playlist_item` (health check) then calls `probe_and_cache_cors`. For YouTube URLs, `probe_and_cache_cors` short-circuits at line 250 (`needs_resolution(url)` → `return None`) and nothing is written to the CORS cache, leaving the budget badge blank.

With `skip_proxy: true`, these items never consume server egress — their budget is effectively Direct (⚡). The Test button should reflect this.

**Change:** In `playlist_item_test` (`src/routes/admin/playlist.rs`), after `probe_playlist_item` completes, if `resolver::needs_resolution(&item.url)` is true, insert the item URL's host into `cors_cache` directly:

```rust
if crate::media::resolver::needs_resolution(&item.url) {
    let host = crate::media::hls::extract_manifest_host(&item.url);
    state.cors_cache.write().await.insert(host, std::time::Instant::now());
}
```

This reuses the same CORS cache that `apply_budget` reads (at line 136–138), so the returned row HTML immediately shows ⚡ with no further changes.

The background health checker (`probe_playlist_item` called from the sweep) is **not** changed — it still skips YouTube URLs in `probe_and_cache_cors`. Only the explicit Test button action writes the Direct entry. This is intentional: the badge is set by deliberate admin action, not silently in the background.

**Scope:** playlist items only. Live sources with `needs_resolution` URLs (YouTube live, Twitch) return HLS from yt-dlp and still go through the proxy; the source Test button (`probe_and_cache_cors` path) is unchanged.

## URL Expiry

YouTube `googlevideo.com` URLs expire after ~6 hours. Without the proxy, expiry behaviour is identical:

1. URL expires mid-playback → browser gets 403 on next Range request → `video.onerror` fires
2. Client calls `/next?failed_url=<expired-googlevideo-url>`
3. Server re-runs yt-dlp on the stored `youtube.com` source URL → fresh `googlevideo.com` URL
4. New URL ≠ `failed_url` → source not skipped → fresh URL returned → playback resumes

The proxy was a byte relay with no refresh logic. Recovery is identical.

## Testing

- **No new unit tests needed** for `needs_resolution` — it already has tests covering YouTube and non-YouTube URLs.
- **Existing integration tests** all pass unchanged: seed fixture sources are plain HLS/IPTV URLs, so `skip_proxy` is `false` for all of them; the proxy path is exercised as before.
- **One new integration test** in `tests/http.rs`: assert that tuning channel 1 (Live OK, plain HLS source) returns `"skip_proxy": false` in the JSON response, confirming the field is serialized and defaults correctly.

## Files Changed

| File | Change |
|---|---|
| `src/routes/player.rs` | Add `skip_proxy` to `TuneResponse`; add param to `tune_response()`; update 3 call sites |
| `templates/base.html` | Add `skipProxy` param to `_loadSource`; update `video.src` in else branch; update 6 call sites |
| `src/routes/admin/playlist.rs` | In `playlist_item_test`: write host to CORS cache when `needs_resolution` is true |
| `tests/http.rs` | One new integration test for `skip_proxy: false` on plain HLS channel |
