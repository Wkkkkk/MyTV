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
| `tests/http.rs` | One new integration test for `skip_proxy: false` on plain HLS channel |
