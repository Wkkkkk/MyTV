# Stream-Proxy SSRF / Open-Proxy Hardening Implementation Plan

> **For agentic workers:** REQUIRED SUB-SKILL: Use superpowers:subagent-driven-development (recommended) or superpowers:executing-plans to implement this plan task-by-task. Steps use checkbox (`- [ ]`) syntax for tracking.

**Goal:** Harden `/stream-proxy` against SSRF attacks by resolving the target hostname before each request and each redirect hop, rejecting private/loopback/link-local IPs, and capping the response body at 20 MB.

**Architecture:** A new `src/ssrf.rs` module provides `is_safe_url` (async DNS-resolve + IP range check) and `SsrfError`. `AppState` gains a `proxy_client` built with `redirect::Policy::none()`. The `stream_proxy` handler loops manually over up to 5 redirects, calling `is_safe_url` before each hop, and stream-reads the body up to 20 MB.

**Tech Stack:** Rust 1.96, Axum 0.7, reqwest 0.12, `tokio::net::lookup_host`

---

### Task 1: Create `src/ssrf.rs` — `SsrfError` + `is_safe_url` + unit tests (TDD)

**Files:**
- Create: `src/ssrf.rs`
- Modify: `src/lib.rs` (declare module — needed for tests to compile)

- [ ] **Step 1: Declare the module in `src/lib.rs`**

Add `pub mod ssrf;` to the module declarations at the top of `src/lib.rs` (after the existing `mod budget;` line):

```rust
mod budget;
pub mod config;
pub mod db;
mod epg;
pub mod health;
mod media;
mod model;
mod routes;
pub mod ssrf;
```

- [ ] **Step 2: Create `src/ssrf.rs` with failing stubs and all unit tests**

Create `src/ssrf.rs` with the full test suite and placeholder implementations that will fail:

```rust
use std::net::IpAddr;
use tokio::net::lookup_host;

#[derive(Debug)]
pub enum SsrfError {
    BlockedAddress(IpAddr),
    DnsFailure(String),
    UnsupportedScheme,
}

impl std::fmt::Display for SsrfError {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        match self {
            SsrfError::BlockedAddress(ip) => write!(f, "blocked address: {ip}"),
            SsrfError::DnsFailure(msg) => write!(f, "DNS failure: {msg}"),
            SsrfError::UnsupportedScheme => write!(f, "unsupported scheme"),
        }
    }
}

fn is_blocked(_ip: IpAddr) -> bool {
    false // placeholder
}

pub async fn is_safe_url(_url: &str) -> Result<(), SsrfError> {
    Ok(()) // placeholder
}

#[cfg(test)]
mod tests {
    use super::*;

    #[tokio::test]
    async fn blocks_loopback_ipv4() {
        assert!(matches!(
            is_safe_url("http://127.0.0.1/").await,
            Err(SsrfError::BlockedAddress(_))
        ));
    }

    #[tokio::test]
    async fn blocks_rfc1918_10() {
        assert!(matches!(
            is_safe_url("http://10.0.0.1/").await,
            Err(SsrfError::BlockedAddress(_))
        ));
    }

    #[tokio::test]
    async fn blocks_rfc1918_172() {
        assert!(matches!(
            is_safe_url("http://172.16.0.1/").await,
            Err(SsrfError::BlockedAddress(_))
        ));
    }

    #[tokio::test]
    async fn blocks_rfc1918_192() {
        assert!(matches!(
            is_safe_url("http://192.168.1.1/").await,
            Err(SsrfError::BlockedAddress(_))
        ));
    }

    #[tokio::test]
    async fn blocks_link_local_metadata() {
        assert!(matches!(
            is_safe_url("http://169.254.169.254/latest/meta-data/").await,
            Err(SsrfError::BlockedAddress(_))
        ));
    }

    #[tokio::test]
    async fn blocks_ipv6_loopback() {
        assert!(matches!(
            is_safe_url("http://[::1]/").await,
            Err(SsrfError::BlockedAddress(_))
        ));
    }

    #[tokio::test]
    async fn blocks_ipv6_ula() {
        assert!(matches!(
            is_safe_url("http://[fc00::1]/").await,
            Err(SsrfError::BlockedAddress(_))
        ));
    }

    #[tokio::test]
    async fn blocks_ipv6_link_local() {
        assert!(matches!(
            is_safe_url("http://[fe80::1]/").await,
            Err(SsrfError::BlockedAddress(_))
        ));
    }
}
```

- [ ] **Step 3: Run tests to verify they fail**

```bash
cargo test ssrf
```

Expected: 8 tests FAIL — `Ok(())` placeholder never returns `Err`.

- [ ] **Step 4: Implement `is_blocked` and `is_safe_url`**

Replace the placeholder `is_blocked` and `is_safe_url` functions in `src/ssrf.rs` with the full implementation (keep the `SsrfError` enum, `Display` impl, and `#[cfg(test)]` block unchanged):

```rust
fn is_blocked(ip: IpAddr) -> bool {
    match ip {
        IpAddr::V4(v4) => v4.is_loopback() || v4.is_private() || v4.is_link_local(),
        IpAddr::V6(v6) => {
            if v6.is_loopback() {
                return true;
            }
            let first = v6.segments()[0];
            // fc00::/7 — IPv6 unique local (includes Fly fdaa::/16)
            // fe80::/10 — IPv6 link-local
            (first & 0xfe00) == 0xfc00 || (first & 0xffc0) == 0xfe80
        }
    }
}

pub async fn is_safe_url(url: &str) -> Result<(), SsrfError> {
    let parsed = reqwest::Url::parse(url).map_err(|_| SsrfError::UnsupportedScheme)?;
    match parsed.scheme() {
        "http" | "https" => {}
        _ => return Err(SsrfError::UnsupportedScheme),
    }
    let host = parsed.host_str().ok_or(SsrfError::UnsupportedScheme)?;
    // IPv6 literals (e.g. "::1") need brackets for the "host:port" lookup format
    let lookup_target = if host.contains(':') {
        format!("[{host}]:80")
    } else {
        format!("{host}:80")
    };
    let addrs = lookup_host(&lookup_target)
        .await
        .map_err(|e| SsrfError::DnsFailure(e.to_string()))?;
    for addr in addrs {
        if is_blocked(addr.ip()) {
            return Err(SsrfError::BlockedAddress(addr.ip()));
        }
    }
    Ok(())
}
```

- [ ] **Step 5: Run tests to verify they pass**

```bash
cargo test ssrf
```

Expected: all 8 tests pass.

- [ ] **Step 6: Run fmt and commit**

```bash
cargo fmt
git add src/ssrf.rs src/lib.rs
git commit -m "feat: add ssrf module with is_safe_url and IP range blocking"
```

---

### Task 2: Add `proxy_client` to `AppState` and update all construction sites

**Files:**
- Modify: `src/lib.rs`
- Modify: `src/main.rs`
- Modify: `tests/http.rs`
- Modify: `src/routes/player.rs` (test helper only)

`AppState` is a struct — adding a field is a compile error everywhere it is constructed. Update all three sites in one task so the codebase stays green.

- [ ] **Step 1: Add `proxy_client` field to `AppState` in `src/lib.rs`**

Change:

```rust
#[derive(Clone)]
pub struct AppState {
    pub pool: SqlitePool,
    pub config: Arc<config::Config>,
    pub http_client: reqwest::Client,
    pub cors_cache: CorsCache,
}
```

to:

```rust
#[derive(Clone)]
pub struct AppState {
    pub pool: SqlitePool,
    pub config: Arc<config::Config>,
    pub http_client: reqwest::Client,
    pub proxy_client: reqwest::Client,
    pub cors_cache: CorsCache,
}
```

- [ ] **Step 2: Build `proxy_client` in `src/main.rs`**

Change:

```rust
let http_client = reqwest::Client::builder()
    .timeout(std::time::Duration::from_secs(10))
    .build()?;

let cors_cache: CorsCache = Arc::new(RwLock::new(HashMap::new()));

let state = AppState {
    pool,
    config: config.clone(),
    http_client,
    cors_cache,
};
```

to:

```rust
let http_client = reqwest::Client::builder()
    .timeout(std::time::Duration::from_secs(10))
    .build()?;

let proxy_client = reqwest::Client::builder()
    .redirect(reqwest::redirect::Policy::none())
    .timeout(std::time::Duration::from_secs(10))
    .build()?;

let cors_cache: CorsCache = Arc::new(RwLock::new(HashMap::new()));

let state = AppState {
    pool,
    config: config.clone(),
    http_client,
    proxy_client,
    cors_cache,
};
```

- [ ] **Step 3: Update `app()` in `tests/http.rs`**

Change:

```rust
let state = AppState {
    pool,
    config: Arc::new(Config {
        database_url: "sqlite::memory:".to_string(),
        admin_password: "test".to_string(),
        youtube_api_key: None,
        port: 0,
    }),
    http_client: test_client(),
    cors_cache: Arc::new(tokio::sync::RwLock::new(std::collections::HashMap::new())),
};
```

to:

```rust
let state = AppState {
    pool,
    config: Arc::new(Config {
        database_url: "sqlite::memory:".to_string(),
        admin_password: "test".to_string(),
        youtube_api_key: None,
        port: 0,
    }),
    http_client: test_client(),
    proxy_client: reqwest::Client::builder()
        .redirect(reqwest::redirect::Policy::none())
        .timeout(std::time::Duration::from_millis(500))
        .build()
        .unwrap(),
    cors_cache: Arc::new(tokio::sync::RwLock::new(std::collections::HashMap::new())),
};
```

- [ ] **Step 4: Update `test_state()` in `src/routes/player.rs`**

In the `#[cfg(test)]` block at the bottom of `src/routes/player.rs`, change the `AppState` construction inside `test_state()`:

Change:

```rust
AppState {
    pool,
    config,
    http_client: reqwest::Client::new(),
    cors_cache: std::sync::Arc::new(tokio::sync::RwLock::new(
        std::collections::HashMap::new(),
    )),
}
```

to:

```rust
AppState {
    pool,
    config,
    http_client: reqwest::Client::new(),
    proxy_client: reqwest::Client::builder()
        .redirect(reqwest::redirect::Policy::none())
        .build()
        .unwrap(),
    cors_cache: std::sync::Arc::new(tokio::sync::RwLock::new(
        std::collections::HashMap::new(),
    )),
}
```

- [ ] **Step 5: Verify all existing tests still pass**

```bash
cargo test
```

Expected: all 117 existing tests pass (no behavior change yet — `proxy_client` is wired but not used).

- [ ] **Step 6: Run fmt and commit**

```bash
cargo fmt
git add src/lib.rs src/main.rs tests/http.rs src/routes/player.rs
git commit -m "feat: add proxy_client to AppState (redirect::Policy::none(), 10s timeout)"
```

---

### Task 3: Rewrite `stream_proxy` handler — SSRF-checked redirect loop + 20 MB body cap (TDD)

**Files:**
- Modify: `tests/http.rs` (integration tests first)
- Modify: `src/routes/player.rs` (handler implementation)

- [ ] **Step 1: Write the 3 failing integration tests in `tests/http.rs`**

Append to the end of `tests/http.rs`:

```rust
#[tokio::test]
async fn stream_proxy_blocks_loopback() {
    let response = app()
        .await
        .oneshot(req("/stream-proxy?url=http://127.0.0.1/foo"))
        .await
        .unwrap();
    assert_eq!(response.status(), StatusCode::UNPROCESSABLE_ENTITY);
}

#[tokio::test]
async fn stream_proxy_blocks_link_local_metadata() {
    let response = app()
        .await
        .oneshot(req(
            "/stream-proxy?url=http://169.254.169.254/latest/meta-data/",
        ))
        .await
        .unwrap();
    assert_eq!(response.status(), StatusCode::UNPROCESSABLE_ENTITY);
}

#[tokio::test]
async fn stream_proxy_blocks_private_rfc1918() {
    let response = app()
        .await
        .oneshot(req("/stream-proxy?url=http://10.0.0.1/foo"))
        .await
        .unwrap();
    assert_eq!(response.status(), StatusCode::UNPROCESSABLE_ENTITY);
}
```

- [ ] **Step 2: Run tests to verify they fail**

```bash
cargo test stream_proxy_blocks
```

Expected: all 3 FAIL — current handler returns 502 (reqwest connection refused before any SSRF check), not 422.

- [ ] **Step 3: Update imports in `src/routes/player.rs`**

The new handler uses `axum::body::Bytes` to build the response body from the streamed chunks. Add `body::Bytes` to the existing axum import:

Change:

```rust
use axum::{
    extract::{Path, Query, State},
    http::{HeaderMap, HeaderValue, StatusCode},
    response::{IntoResponse, Json, Response},
};
```

to:

```rust
use axum::{
    body::Bytes,
    extract::{Path, Query, State},
    http::{HeaderMap, HeaderValue, StatusCode},
    response::{IntoResponse, Json, Response},
};
```

- [ ] **Step 4: Replace the `stream_proxy` function in `src/routes/player.rs`**

Replace the entire `stream_proxy` function (from `pub async fn stream_proxy(` through the closing `}`) with:

```rust
pub async fn stream_proxy(
    State(state): State<AppState>,
    Query(q): Query<StreamProxyQuery>,
) -> Response {
    if !q.url.starts_with("http://") && !q.url.starts_with("https://") {
        return StatusCode::BAD_REQUEST.into_response();
    }

    let mut url = q.url.clone();
    let mut upstream = None;

    for _ in 0..5 {
        if let Err(e) = crate::ssrf::is_safe_url(&url).await {
            tracing::warn!(url = %url, reason = %e, "stream proxy SSRF check failed");
            return StatusCode::UNPROCESSABLE_ENTITY.into_response();
        }
        let resp = match state.proxy_client.get(&url).send().await {
            Ok(r) => r,
            Err(e) => {
                tracing::warn!(url = %url, error = %e, "stream proxy fetch failed");
                return StatusCode::BAD_GATEWAY.into_response();
            }
        };
        if resp.status().is_redirection() {
            let location = match resp
                .headers()
                .get(axum::http::header::LOCATION)
                .and_then(|v| v.to_str().ok())
            {
                Some(loc) => loc.to_string(),
                None => return StatusCode::BAD_GATEWAY.into_response(),
            };
            url = location;
            continue;
        }
        upstream = Some(resp);
        break;
    }

    let mut upstream = match upstream {
        Some(r) => r,
        None => return StatusCode::BAD_GATEWAY.into_response(),
    };

    let ct = upstream
        .headers()
        .get(axum::http::header::CONTENT_TYPE)
        .and_then(|v| v.to_str().ok())
        .unwrap_or("")
        .to_string();

    let is_playlist = ct.contains("mpegurl") || url.contains(".m3u8") || url.contains(".m3u");

    const MAX_BODY: usize = 20 * 1024 * 1024;
    let mut collected: Vec<u8> = Vec::new();
    loop {
        match upstream.chunk().await {
            Ok(Some(chunk)) => {
                if collected.len() + chunk.len() > MAX_BODY {
                    tracing::warn!(url = %url, "stream proxy response exceeds 20 MB cap");
                    return StatusCode::BAD_GATEWAY.into_response();
                }
                collected.extend_from_slice(&chunk);
            }
            Ok(None) => break,
            Err(e) => {
                tracing::warn!(url = %url, error = %e, "stream proxy read failed");
                return StatusCode::BAD_GATEWAY.into_response();
            }
        }
    }
    let body_bytes = Bytes::from(collected);

    let mut headers = HeaderMap::new();
    headers.insert("access-control-allow-origin", HeaderValue::from_static("*"));

    if is_playlist {
        let text = String::from_utf8_lossy(&body_bytes);
        let direct = resolve_direct_segments(&state, &text, &url).await;
        let rewritten = hls::rewrite_hls_urls(&text, &url, direct);
        headers.insert(
            axum::http::header::CONTENT_TYPE,
            HeaderValue::from_static("application/vnd.apple.mpegurl"),
        );
        (headers, rewritten).into_response()
    } else {
        if let Ok(val) = HeaderValue::from_str(&ct) {
            headers.insert(axum::http::header::CONTENT_TYPE, val);
        }
        (headers, body_bytes).into_response()
    }
}
```

- [ ] **Step 5: Run the 3 new tests to verify they pass**

```bash
cargo test stream_proxy_blocks
```

Expected: all 3 PASS.

- [ ] **Step 6: Run the full test suite**

```bash
cargo test
```

Expected: all 120 tests pass (117 existing + 3 new). If any existing test fails, investigate before proceeding — do not commit a red suite.

- [ ] **Step 7: Run fmt and commit**

```bash
cargo fmt
git add src/routes/player.rs tests/http.rs
git commit -m "feat: harden stream-proxy — SSRF DNS pre-check, manual redirect loop (max 5), 20 MB body cap"
```
