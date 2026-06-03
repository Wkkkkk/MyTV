# Streaming Proxy — First-Byte Latency, Peak Memory, and DNS Cache

## Goal

Three improvements to `/stream-proxy` that work together:

1. **Segment streaming** — pipe CDN bytes directly to the browser instead of buffering, cutting first-byte latency and peak memory per request.
2. **SSRF hostname cache** — skip the DNS round-trip for hosts that were validated within the last 60 s, avoiding one DNS lookup per HLS segment (~1 per 2–8 s per active stream).
3. **Timeout split** — separate connect timeout (5 s) from read/response timeout (30 s) on `proxy_client`, so slow CDNs that accept connections but deliver data slowly don't time out mid-segment.

## Segment Streaming

HLS segment requests (`!is_playlist`) currently buffer the full body into a `Vec<u8>` (up to 20 MB) before sending. With the `stream` feature enabled in reqwest, `upstream.bytes_stream()` yields chunks as they arrive. `axum::body::Body::from_stream(stream)` wraps that into a streaming response body — the browser starts receiving bytes as soon as the CDN sends the first chunk.

The 20 MB cap is removed for the segment path: the browser closes the connection when it has the segment, dropping the reqwest stream and releasing the CDN connection. No memory accumulates. The 20 MB cap is **kept** for the playlist path (buffering is required for HLS URL rewriting).

**`Cargo.toml` change:** add `"stream"` to reqwest features:

```toml
reqwest = { version = "0.12", features = ["rustls-tls", "json", "stream"], default-features = false }
```

**Handler change** (segment path only):

```rust
// before
const MAX_BODY: usize = 20 * 1024 * 1024;
let mut collected: Vec<u8> = Vec::new();
loop { /* chunk() loop */ }
let body_bytes = Bytes::from(collected);
// ...
(headers, body_bytes).into_response()

// after
let body = axum::body::Body::from_stream(upstream.bytes_stream());
(headers, body).into_response()
```

The `Bytes` import and the `collected` Vec are removed from the segment path. The playlist path (`is_playlist == true`) is unchanged.

## SSRF Hostname Cache

### New items in `src/ssrf.rs`

```rust
pub type SsrfCache = Arc<RwLock<HashMap<String, std::time::Instant>>>;

pub async fn is_safe_url_cached(url: &str, cache: &SsrfCache) -> Result<(), SsrfError>
```

`is_safe_url_cached` steps:
1. Parse URL; reject if non-http(s) → `UnsupportedScheme`.
2. Extract hostname (same logic as `is_safe_url`).
3. Check cache: if entry exists and `elapsed() < 60 s` → return `Ok(())` immediately.
4. Call `is_safe_url(url)`.
5. On `Ok`: write `(hostname, Instant::now())` to cache; return `Ok(())`.
6. On `Err`: return the error unchanged. Failed hosts are never cached.

Cache key: the hostname string (e.g. `"cdn.example.com"`), consistent with the existing CORS cache key pattern (`extract_manifest_host`).

### `AppState` change (`src/lib.rs`)

```rust
pub ssrf_cache: SsrfCache,
```

### `src/main.rs`

```rust
let ssrf_cache: SsrfCache = Arc::new(RwLock::new(HashMap::new()));
```

### Handler change

Replace `crate::ssrf::is_safe_url(&url)` with `crate::ssrf::is_safe_url_cached(&url, &state.ssrf_cache)` in the redirect loop.

## Timeout Split

`proxy_client` is rebuilt in `src/main.rs` with a separate connect timeout and response timeout:

```rust
let proxy_client = reqwest::Client::builder()
    .redirect(reqwest::redirect::Policy::none())
    .connect_timeout(std::time::Duration::from_secs(5))
    .timeout(std::time::Duration::from_secs(30))
    .build()?;
```

`connect_timeout` fires if the TCP handshake doesn't complete within 5 s (dead host, firewall drop). `timeout` is the overall request timeout from send to last byte — 30 s allows slow CDNs to deliver large segments without being cut off. The same change applies to the test `proxy_client` in `tests/http.rs` and `test_state()` in `player.rs` (can use any reasonable values for tests, e.g. 500 ms connect + 500 ms overall).

## Error Handling

- `is_safe_url_cached`: cache is read-locked for lookup, write-locked only on a successful miss. Under concurrent requests to the same host, two parallel misses may both call `is_safe_url` and both write; the second write is a no-op overwrite of the same hostname — correct and harmless.
- Streaming body errors (CDN drops mid-segment): axum propagates the error to the browser as a broken stream. The browser's HLS player retries the segment. No server-side log needed.

## Testing

**Unit tests in `src/ssrf.rs`:**
- `cache_hit_skips_dns`: call `is_safe_url_cached` twice for a safe IP literal with a fresh cache; second call returns `Ok` without a DNS lookup (verified by checking the cache contains the entry after the first call).
- `cache_miss_on_blocked_host`: call `is_safe_url_cached` for `http://127.0.0.1/`; returns `BlockedAddress` and cache remains empty.
- `cache_expires_after_ttl`: insert a cache entry with `Instant::now() - Duration::from_secs(61)`; call `is_safe_url_cached`; cache entry is refreshed (timestamp updated).

**Integration tests in `tests/http.rs`:** no new integration tests needed — the existing SSRF block tests still pass (they exercise `is_safe_url_cached` via the handler once AppState is updated).

**Streaming:** the segment streaming path cannot be integration-tested without a live upstream — the existing 502/422 tests continue to cover the error paths. Manual verification: load a VOD channel in the browser and confirm segments play.

## Files Changed

| File | Change |
|------|--------|
| `Cargo.toml` | Add `"stream"` to reqwest features |
| `src/ssrf.rs` | Add `SsrfCache` type + `is_safe_url_cached` + 3 unit tests |
| `src/lib.rs` | Add `ssrf_cache: SsrfCache` to `AppState`; export `SsrfCache` |
| `src/main.rs` | Init `ssrf_cache`; rebuild `proxy_client` with split timeouts |
| `src/routes/player.rs` | Segment path: stream body; use `is_safe_url_cached`. `Bytes` import stays (playlist path still uses it) |
| `tests/http.rs` | Add `ssrf_cache` to `AppState` construction in `app()` / `app_with_cors()`; update `proxy_client` timeouts |
| `src/routes/player.rs` (#[cfg(test)]) | Add `ssrf_cache` to `test_state()`; update `proxy_client` timeouts |
