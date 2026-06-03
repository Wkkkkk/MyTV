# Proxy Response Fidelity

## Goal

Make `/stream-proxy` a more transparent proxy so the browser's HLS player sees accurate HTTP semantics:

1. **Status code passthrough** — upstream 404/403/206 reaches the browser instead of always 200.
2. **Full response header forwarding** — all upstream headers forwarded except `Access-Control-Allow-Origin` (overridden to `*`) and `Content-Length` on the playlist path (body changes after URL rewriting).
3. **`Range` request forwarding** — browser's `Range` header forwarded to CDN on every redirect hop, enabling byte-range segment delivery.

## Changes

All changes are in `src/routes/player.rs`, `stream_proxy` function only. No new modules, no new types.

### Handler signature

Add `HeaderMap` extractor to receive browser request headers:

```rust
pub async fn stream_proxy(
    State(state): State<AppState>,
    Query(q): Query<StreamProxyQuery>,
    request_headers: HeaderMap,
) -> Response {
```

### Range forwarding (redirect loop)

On every hop, attach the browser's `Range` header if present:

```rust
let mut req = state.proxy_client.get(&url);
if let Some(range) = request_headers.get(axum::http::header::RANGE) {
    req = req.header(axum::http::header::RANGE, range);
}
let resp = match req.send().await { ... }
```

### Status passthrough

Extract the upstream status before consuming the response:

```rust
let status = upstream.status();
```

Return `(status, headers, body).into_response()` on both the playlist and segment paths.

### Response header forwarding

After inserting `access-control-allow-origin: *`, copy all upstream headers, skipping only `access-control-allow-origin` (we own it):

```rust
for (key, val) in upstream.headers() {
    if key == axum::http::header::ACCESS_CONTROL_ALLOW_ORIGIN {
        continue;
    }
    headers.insert(key.clone(), val.clone());
}
```

On the **playlist path only**, remove `content-length` after the copy — the rewritten body has a different byte length than the original:

```rust
headers.remove(axum::http::header::CONTENT_LENGTH);
```

On the **segment path**, `Content-Length` is retained — bytes stream unmodified so the CDN's length is accurate.

Note: reqwest/hyper strips hop-by-hop headers (`Transfer-Encoding`, `Connection`, `Keep-Alive`) before exposing `upstream.headers()`, so they never appear in the iteration.

## What does NOT change

- SSRF check logic (`is_safe_url_cached`) — unchanged
- Redirect loop structure (max 5 hops) — unchanged
- Playlist buffering + HLS URL rewriting — unchanged
- Segment streaming (`Body::from_stream`) — unchanged
- 20 MB cap on playlist path — unchanged

## Error handling

No new error cases. If the CDN returns 416 (Range Not Satisfiable), that status passes through to the browser. The browser's HLS player handles it. Header iteration never fails.

## Testing

No new tests required. The four existing SSRF/scheme guard tests (`stream_proxy_blocks_loopback`, `stream_proxy_blocks_link_local_metadata`, `stream_proxy_blocks_private_rfc1918`, `stream_proxy_rejects_non_http_scheme`) exercise all guard paths and continue to pass. Status passthrough and header forwarding are verified by compilation and manual testing against a live stream.

## Files changed

| File | Change |
|------|--------|
| `src/routes/player.rs` | Add `request_headers: HeaderMap` parameter; forward `Range` on each loop hop; extract + pass through `upstream.status()`; copy all upstream headers except `access-control-allow-origin`; remove `content-length` on playlist path |
