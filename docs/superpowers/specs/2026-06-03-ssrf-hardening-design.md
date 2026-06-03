# SSRF / Open-Proxy Hardening for `/stream-proxy`

## Problem

`/stream-proxy` is on the public, unauthenticated router and fetches any `?url=` after only
checking the `http(s)://` prefix. Anyone on the internet can use the Fly instance as an open
proxy to probe the Fly internal network or cloud metadata endpoints
(`169.254.169.254`, `127.0.0.1`, `[::1]`, Fly's `fdaa::/16` subnet, etc.).

Two additional gaps:
- `reqwest::Client` follows up to 10 redirects automatically — a redirect chain can bypass a
  pre-flight IP check (TOCTOU).
- Response bodies are fully buffered with no size cap — a large upstream response can OOM
  the process.

## Fix: Option A — Manual redirect loop with pre-flight DNS check

Before each request (initial URL + every redirect hop) resolve the hostname and reject if any
returned IP falls in a private/reserved range. Use a dedicated proxy client with
`redirect::Policy::none()` so redirects are never followed automatically; loop manually up to
5 hops. Cap buffered body at 20 MB.

## New module: `src/ssrf.rs`

Two public items:

```rust
pub enum SsrfError { BlockedAddress(IpAddr), DnsFailure(String), UnsupportedScheme }

pub async fn is_safe_url(url: &str) -> Result<(), SsrfError>
```

`is_safe_url` steps:
1. Parse the URL; reject if scheme is not `http` or `https` → `UnsupportedScheme`.
2. Extract hostname; resolve with `tokio::net::lookup_host("<host>:80")`.
3. Reject if DNS lookup returns an error → `DnsFailure`.
4. Reject if **any** resolved `IpAddr` is in a blocked range → `BlockedAddress`.

Blocked ranges:

| Range | Description |
|-------|-------------|
| `127.0.0.0/8` | IPv4 loopback |
| `::1/128` | IPv6 loopback |
| `10.0.0.0/8` | RFC 1918 private |
| `172.16.0.0/12` | RFC 1918 private |
| `192.168.0.0/16` | RFC 1918 private |
| `169.254.0.0/16` | Link-local / AWS+GCP metadata |
| `fc00::/7` | IPv6 unique local (includes Fly `fdaa::/16`) |
| `fe80::/10` | IPv6 link-local |

Range checks use `IpAddr::is_loopback()`, `is_private()` (stable in Rust 1.96 for `Ipv4Addr`),
`is_link_local()`, and manual prefix checks for IPv6 ULA (`(u & 0xfe00) == 0xfc00`) and
link-local (`(u & 0xffc0) == 0xfe80`).

## `AppState` change

Add `proxy_client: reqwest::Client` built with `redirect::Policy::none()` and the same 10 s
timeout. The existing `http_client` is unchanged — health checks and CORS probes need
redirect-following.

## `stream_proxy` handler — new flow

```
1. Validate scheme (existing check) → 400 on fail.
2. loop (max 5 hops):
   a. is_safe_url(&url) → 422 on SsrfError (log warn).
   b. proxy_client.get(&url).send() → 502 on network error.
   c. If 3xx:
      - Extract Location header → 502 if missing/invalid.
      - url = location; continue loop.
   d. On 2xx (or other): break loop, proceed to body read.
3. If loop exhausted without a non-3xx response → 502.
4. Read body: stream up to 20 MB; return 502 if exceeded.
5. Pass through content-type and rewrite HLS URLs as today.
```

## Error mapping

| Condition | Status |
|-----------|--------|
| Non-http(s) scheme | 400 |
| Private/loopback IP resolved | 422 |
| DNS resolution fails | 422 |
| Redirect target fails `is_safe_url` | 422 |
| More than 5 redirect hops | 502 |
| Upstream network error | 502 |
| Body exceeds 20 MB | 502 |

All SSRF-blocked requests emit `tracing::warn!(url, reason)`.

## Testing

**Unit tests in `src/ssrf.rs`** (no real network; IP literals resolve instantly via OS):
- `is_safe_url("http://127.0.0.1/")` → `BlockedAddress`
- `is_safe_url("http://10.0.0.1/")` → `BlockedAddress`
- `is_safe_url("http://172.16.0.1/")` → `BlockedAddress`
- `is_safe_url("http://192.168.1.1/")` → `BlockedAddress`
- `is_safe_url("http://169.254.169.254/latest/meta-data/")` → `BlockedAddress`
- `is_safe_url("http://[::1]/")` → `BlockedAddress`
- `is_safe_url("http://[fc00::1]/")` → `BlockedAddress`
- `is_safe_url("http://[fe80::1]/")` → `BlockedAddress`

**Integration tests in `tests/http.rs`** (oneshot, no TCP):
- `GET /stream-proxy?url=http://127.0.0.1/foo` → 422
- `GET /stream-proxy?url=http://169.254.169.254/latest/meta-data/` → 422
- `GET /stream-proxy?url=http://10.0.0.1/foo` → 422

## Files changed

| File | Change |
|------|--------|
| `src/ssrf.rs` | New module: `is_safe_url`, `SsrfError`, IP range checks |
| `src/lib.rs` | Export `ssrf` module; add `proxy_client` to `AppState` |
| `src/main.rs` | Build `proxy_client` with `redirect::Policy::none()` |
| `src/routes/player.rs` | Replace single `.get().send()` with SSRF-checked redirect loop + 20 MB body cap |
| `tests/http.rs` | Three new integration tests for blocked IPs |
