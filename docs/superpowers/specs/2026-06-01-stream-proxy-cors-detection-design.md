# Design: Stream Proxy — Automatic CORS Detection

**Date:** 2026-06-01  
**Status:** Draft

---

## Problem

Every HLS stream currently routes through the Fly.io server — manifest files _and_ every `.ts`/`.m4s` segment. The proxy exists to satisfy browser CORS rules: IPTV streams are designed for native players, so most don't serve `Access-Control-Allow-Origin` headers, and browsers block cross-origin video fetches without them.

The cost: at 4 Mbps, a single stream running 8 hours = ~14 GB of egress through Fly.io. Segments are the bulk; manifests are kilobytes.

---

## Goal

Let the browser fetch segments directly from the origin CDN when it is safe to do so, without breaking streams that lack CORS headers. Manifests always go through the proxy (they need URL rewriting regardless).

---

## Decision Matrix

Two independent browser restrictions determine whether a segment can go direct:

**Mixed content** — a browser on an HTTPS page blocks HTTP sub-resources. Since the app is served over HTTPS (`kunstv.fly.dev`), HTTP stream segments must always proxy. No probe needed; the answer is always "proxy."

**CORS** — only applies to HTTPS segments. The origin server must send `Access-Control-Allow-Origin: *` for the browser to allow a cross-origin fetch.

| Origin scheme | Has CORS headers | Segment handling |
|---|---|---|
| `http://` | any | Always proxy (mixed content) |
| `https://` | No | Proxy (CORS blocks direct) |
| `https://` | Yes | Write absolute URL — browser fetches directly |

**Relative vs absolute paths in the manifest do not affect this decision.** `rewrite_hls_urls` already resolves all relative paths to absolute before deciding what to do with them. For the direct path, the resolved absolute URL is written into the manifest as-is. For the proxy path, it is wrapped in `/stream-proxy?url=...` as today.

### Real example: Mux stream

`https://stream.mux.com/v69RSHhFelSm4701snP22dYz2jICy4E4FUyk02rW4gxRM.m3u8`

- Master manifest: `stream.mux.com` — HTTPS, `Access-Control-Allow-Origin: *`
- Variant playlists: `manifest-oci-us-ashburn-1-vop1.edgemv.mux.com` — HTTPS
- Segments (`.m4s`): `chunk-oci-us-ashburn-1-vop1.cfcdn.mux.com` — HTTPS, `Access-Control-Allow-Origin: *`

Result: segments go direct from Cloudflare CDN to browser. Fly.io sees only the manifests (a few KB every few seconds). Zero segment egress.

---

## Approach: Automatic CORS Probing with In-Memory Cache

When the proxy serves a **variant playlist** (a `.m3u8` that contains segment URLs, not sub-playlists):

1. Find the first segment URL in the manifest body.
2. If its scheme is `http://` → skip probe, use proxy mode.
3. If its scheme is `https://` → check the in-memory cache keyed by `scheme://host`.
   - Cache hit → use cached result.
   - Cache miss → HEAD-request the segment URL, check for `Access-Control-Allow-Origin: *`, store result.
4. Pass `direct_segments: bool` into `rewrite_hls_urls`.
   - `true` → segment lines become absolute URLs; sub-playlist lines (`.m3u8`) still proxy.
   - `false` → all lines proxy as today.

The probe adds one HEAD request (headers only, no body) on the first tune of a new hostname. Subsequent tunes for the same CDN hostname hit the cache.

---

## Components

### 1. CORS cache in `AppState`

```rust
cors_cache: Arc<tokio::sync::RwLock<HashMap<String, bool>>>
```

Key: `scheme://host` (e.g., `https://chunk-oci-us-ashburn-1-vop1.cfcdn.mux.com`).  
Value: `true` = go direct, `false` = proxy.  
In-memory only — cleared on restart. No TTL needed for a personal app.

### 2. `probe_cors(client, url) -> bool` (new, in `src/media/hls.rs` or `src/media/cors.rs`)

HEAD-requests `url`, returns `true` if the response includes `Access-Control-Allow-Origin: *`. Returns `false` on any network error (fail safe — proxy is the safe default).

### 3. `find_first_segment_url(content, base_url) -> Option<String>` (new, in `src/media/hls.rs`)

Scans manifest lines, skips comments and `.m3u8` lines, returns the first resolved absolute URL. Used by `stream_proxy` to know what URL to probe.

### 4. Modified `rewrite_hls_urls(content, base_url, direct_segments: bool) -> String`

- `direct_segments = false` → existing behaviour unchanged.
- `direct_segments = true` → lines ending in `.m3u8` / `.m3u` still proxy; all other lines (segments) are written as resolved absolute URLs.

### 5. Modified `stream_proxy` handler

After fetching the manifest body and determining `is_playlist`:
- If it's a playlist, call `find_first_segment_url` to get a candidate.
- Determine `direct_segments` via cache + probe (as described above).
- Pass `direct_segments` into `rewrite_hls_urls`.

---

## What Does Not Change

- HTTP streams: always proxied, no probe, no cache lookup.
- Master playlists: always proxied (variant URLs inside are `.m3u8` lines, treated as sub-playlists by the modified `rewrite_hls_urls`).
- Non-playlist responses (raw `.ts`/`.m4s` that were proxied before): unchanged path — these only arrive at the proxy for streams in proxy mode.
- Database schema: no changes.
- Admin UI: no changes.

---

## Fly.io Network Budget Impact

| Scenario | Before | After |
|---|---|---|
| HTTP stream (any) | All bytes through Fly.io | No change (always proxy) |
| HTTPS stream, no CORS | All bytes through Fly.io | No change (always proxy) |
| HTTPS stream, with CORS (e.g. Mux) | All bytes through Fly.io | Only manifests through Fly.io; segments go direct |
| CORS probe (first tune per hostname) | — | One HEAD request (headers only) |

---

## Error Handling

- Probe fails (network error, timeout) → log a warning, treat as `false` (proxy mode). Safe default.
- Probe succeeds but returns no CORS header → treat as `false`. Safe default.
- Cache is read-locked for reads, write-locked only on miss. Concurrent tunes to the same new hostname may both probe; the second write just overwrites with the same value.

---

## Testing

- **Unit:** `rewrite_hls_urls` with `direct_segments = true` — segment lines are absolute URLs, `.m3u8` lines still go through proxy.
- **Unit:** `find_first_segment_url` — returns first non-comment, non-playlist line resolved to absolute URL; returns `None` for a master playlist.
- **Unit:** `probe_cors` — mock HTTP client returning various header combinations.
- **Integration:** `stream_proxy` with a mock upstream that returns a variant playlist; assert rewritten URLs are direct when CORS probe returns `true`.

---

## Files Changed

| File | Change |
|---|---|
| `src/lib.rs` | Add `cors_cache` field to `AppState` |
| `src/media/hls.rs` | Add `direct_segments` param to `rewrite_hls_urls`; add `find_first_segment_url`; add `probe_cors` |
| `src/routes/player.rs` | `stream_proxy` handler: CORS cache lookup + probe + pass flag to rewriter |
| `tests/http.rs` | Integration test for direct-segment rewriting |
