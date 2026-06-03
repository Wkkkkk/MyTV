# Stream Proxy Redirect Correctness — Design

## Problem

The manual redirect loop in `stream_proxy` (`src/routes/player.rs`) uses the raw `Location` header string as the next URL without resolving it against the current URL. When an upstream CDN returns a relative `Location` value (e.g. `/live/index.m3u8` or `../segment.m3u8`), `reqwest::Url::parse` fails on the relative reference, `is_safe_url_cached` returns `Err(UnsupportedScheme)`, and `stream_proxy` returns HTTP 422 instead of following the redirect.

Relative `Location` headers are common in HLS CDN responses.

## Root cause

`src/routes/player.rs` redirect loop (lines ~220–229):

```rust
url = location;   // BUG: location may be "/live/index.m3u8" — not parseable as absolute URL
```

`is_safe_url_cached` then calls `reqwest::Url::parse(&url)` on the relative string and returns `Err(UnsupportedScheme)` → 422.

## Design

### `resolve_location` helper

Add a small private sync helper in `src/routes/player.rs`:

```rust
fn resolve_location(location: &str, base_url: &str) -> Option<String> {
    if location.starts_with("http://") || location.starts_with("https://") {
        return Some(location.to_string());
    }
    reqwest::Url::parse(base_url)
        .ok()?
        .join(location)
        .ok()
        .map(|u| u.to_string())
}
```

Handles all three `Location` forms:
- Absolute (`http://cdn.example.com/new`) — fast-path passthrough
- Root-relative (`/newpath`) — joins against `scheme://host` of current URL
- Relative (`../segment.m3u8`) — resolves against current URL path via `reqwest::Url::join`

Returns `None` only if `base_url` itself is unparseable (should not happen in practice since it passed `is_safe_url_cached`).

### Redirect loop fix

Replace the single assignment in `stream_proxy`:

```rust
// Before
url = location;

// After
url = match resolve_location(&location, &url) {
    Some(resolved) => resolved,
    None => return StatusCode::BAD_GATEWAY.into_response(),
};
```

No other changes to `stream_proxy`.

### Testing

**Unit tests** (sync, no network) in `src/routes/player.rs` `#[cfg(test)]` block:

| Test | Input | Expected |
|------|-------|----------|
| `resolve_location_absolute_passthrough` | absolute `https://cdn.example.com/new.m3u8`, any base | same URL returned |
| `resolve_location_root_relative` | `/live/index.m3u8`, `https://cdn.example.com/old.m3u8` | `https://cdn.example.com/live/index.m3u8` |
| `resolve_location_relative_path` | `index.m3u8`, `https://cdn.example.com/live/master.m3u8` | `https://cdn.example.com/live/index.m3u8` |
| `resolve_location_unparseable_base` | `/path`, `not-a-url` | `None` |

**Integration test** in `tests/http.rs`:

- Spin up a `tokio::net::TcpListener` on an ephemeral port
- Server task handles two sequential requests in order:
  1. Any request → `HTTP/1.1 302 Found\r\nLocation: /redirected.m3u8\r\n...`
  2. Any request → `HTTP/1.1 200 OK\r\nContent-Type: application/vnd.apple.mpegurl\r\n...` with minimal HLS body
- Use `app_with_ssrf_bypass` (already in `tests/http.rs`) so the SSRF check passes for localhost
- Assert the proxy response is `200 OK` (not `422 Unprocessable Entity`)

## Files changed

| File | Change |
|------|--------|
| `src/routes/player.rs` | Add `resolve_location` helper; one-line fix in redirect loop; 4 unit tests |
| `tests/http.rs` | 1 integration test |

No new dependencies. No signature changes.
