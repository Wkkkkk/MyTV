# Streaming Proxy — Latency, Memory, DNS Cache, Timeout Split Implementation Plan

> **For agentic workers:** REQUIRED SUB-SKILL: Use superpowers:subagent-driven-development (recommended) or superpowers:executing-plans to implement this plan task-by-task. Steps use checkbox (`- [ ]`) syntax for tracking.

**Goal:** Cut `/stream-proxy` first-byte latency and peak memory for HLS segments by streaming instead of buffering; add a 60 s SSRF hostname cache to skip per-segment DNS lookups; split `proxy_client` connect/read timeouts for CDN resilience.

**Architecture:** Three independent changes applied in dependency order — (1) `is_safe_url_cached` in `ssrf.rs` with its own `SsrfCache` type; (2) `ssrf_cache` wired into `AppState`, all construction sites updated, handler switched to `is_safe_url_cached`, proxy_client timeouts split; (3) `stream` feature added to reqwest and segment path rewritten to `Body::from_stream`.

**Tech Stack:** Rust 1.96, Axum 0.7, reqwest 0.12 (`stream` feature), `tokio::sync::RwLock`, `std::time::Instant`

---

### Task 1: Add `SsrfCache` and `is_safe_url_cached` to `src/ssrf.rs` (TDD)

**Files:**
- Modify: `src/ssrf.rs`

- [ ] **Step 1: Add imports, `SsrfCache` type, and a failing stub to `src/ssrf.rs`**

Add these imports at the top of `src/ssrf.rs` (after the existing `use` lines):

```rust
use std::collections::HashMap;
use std::sync::Arc;
use tokio::sync::RwLock;
```

Then add the type alias and stub function after the existing `is_safe_url` function (before `#[cfg(test)]`):

```rust
pub type SsrfCache = Arc<RwLock<HashMap<String, std::time::Instant>>>;

pub async fn is_safe_url_cached(_url: &str, _cache: &SsrfCache) -> Result<(), SsrfError> {
    Ok(()) // stub — tests will fail
}
```

- [ ] **Step 2: Add three unit tests to the `#[cfg(test)]` block in `src/ssrf.rs`**

Append these tests inside the existing `mod tests { ... }` block:

```rust
#[tokio::test]
async fn cache_hit_returns_ok() {
    let cache: SsrfCache = Arc::new(RwLock::new(HashMap::new()));
    cache
        .write()
        .await
        .insert("1.1.1.1".to_string(), std::time::Instant::now());
    assert!(is_safe_url_cached("http://1.1.1.1/", &cache).await.is_ok());
}

#[tokio::test]
async fn cache_miss_on_blocked_host() {
    let cache: SsrfCache = Arc::new(RwLock::new(HashMap::new()));
    let result = is_safe_url_cached("http://127.0.0.1/", &cache).await;
    assert!(matches!(result, Err(SsrfError::BlockedAddress(_))));
    assert!(cache.read().await.is_empty());
}

#[tokio::test]
async fn cache_expires_and_refreshes() {
    let cache: SsrfCache = Arc::new(RwLock::new(HashMap::new()));
    cache.write().await.insert(
        "1.1.1.1".to_string(),
        std::time::Instant::now() - std::time::Duration::from_secs(61),
    );
    assert!(is_safe_url_cached("http://1.1.1.1/", &cache).await.is_ok());
    let elapsed = cache.read().await.get("1.1.1.1").unwrap().elapsed();
    assert!(elapsed < std::time::Duration::from_secs(1));
}
```

- [ ] **Step 3: Run tests to verify two of three fail**

```bash
cd /Users/kunwu/Workspace/playground/MyTV && cargo test ssrf
```

Expected:
- `cache_hit_returns_ok` — PASS (stub returns `Ok`, matches expectation)
- `cache_miss_on_blocked_host` — FAIL (stub returns `Ok` but test expects `Err`)
- `cache_expires_and_refreshes` — FAIL (stub doesn't write cache so timestamp is not refreshed)

- [ ] **Step 4: Replace the stub with the real implementation**

Replace the stub `is_safe_url_cached` with:

```rust
pub async fn is_safe_url_cached(url: &str, cache: &SsrfCache) -> Result<(), SsrfError> {
    let parsed = reqwest::Url::parse(url).map_err(|_| SsrfError::UnsupportedScheme)?;
    match parsed.scheme() {
        "http" | "https" => {}
        _ => return Err(SsrfError::UnsupportedScheme),
    }
    let host = parsed.host_str().ok_or(SsrfError::UnsupportedScheme)?.to_string();
    {
        let r = cache.read().await;
        if let Some(ts) = r.get(&host) {
            if ts.elapsed() < std::time::Duration::from_secs(60) {
                return Ok(());
            }
        }
    }
    is_safe_url(url).await?;
    cache.write().await.insert(host, std::time::Instant::now());
    Ok(())
}
```

- [ ] **Step 5: Run tests to verify all pass**

```bash
cd /Users/kunwu/Workspace/playground/MyTV && cargo test ssrf
```

Expected: all 13 existing tests + 3 new = 16 ssrf tests pass.

- [ ] **Step 6: Run fmt and commit**

```bash
cd /Users/kunwu/Workspace/playground/MyTV && cargo fmt && git add src/ssrf.rs && git commit -m "feat: add SsrfCache and is_safe_url_cached with 60s hostname TTL"
```

---

### Task 2: Wire `ssrf_cache` into `AppState`, update all construction sites, switch handler, split timeouts

**Files:**
- Modify: `src/lib.rs`
- Modify: `src/main.rs`
- Modify: `tests/http.rs`
- Modify: `src/routes/player.rs`

`AppState` gains a field — every construction site must be updated in one task or the code won't compile.

- [ ] **Step 1: Re-export `SsrfCache` and add `ssrf_cache` field to `AppState` in `src/lib.rs`**

After the `pub type CorsCache = ...` line (line 23), add:

```rust
pub use ssrf::SsrfCache;
```

Then change `AppState` from:

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

to:

```rust
#[derive(Clone)]
pub struct AppState {
    pub pool: SqlitePool,
    pub config: Arc<config::Config>,
    pub http_client: reqwest::Client,
    pub proxy_client: reqwest::Client,
    pub cors_cache: CorsCache,
    pub ssrf_cache: SsrfCache,
}
```

- [ ] **Step 2: Update `src/main.rs`**

Change the import line from:

```rust
use mytv::{build_router, config, db, health, AppState, CorsCache};
```

to:

```rust
use mytv::{build_router, config, db, health, AppState, CorsCache, SsrfCache};
```

Then change the client + state construction block from:

```rust
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

to:

```rust
let proxy_client = reqwest::Client::builder()
    .redirect(reqwest::redirect::Policy::none())
    .connect_timeout(std::time::Duration::from_secs(5))
    .timeout(std::time::Duration::from_secs(30))
    .build()?;

let cors_cache: CorsCache = Arc::new(RwLock::new(HashMap::new()));
let ssrf_cache: SsrfCache = Arc::new(RwLock::new(HashMap::new()));

let state = AppState {
    pool,
    config: config.clone(),
    http_client,
    proxy_client,
    cors_cache,
    ssrf_cache,
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
    proxy_client: reqwest::Client::builder()
        .redirect(reqwest::redirect::Policy::none())
        .timeout(std::time::Duration::from_millis(500))
        .build()
        .unwrap(),
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
        .connect_timeout(std::time::Duration::from_millis(500))
        .timeout(std::time::Duration::from_millis(500))
        .build()
        .unwrap(),
    cors_cache: Arc::new(tokio::sync::RwLock::new(std::collections::HashMap::new())),
    ssrf_cache: Arc::new(tokio::sync::RwLock::new(std::collections::HashMap::new())),
};
```

- [ ] **Step 4: Update `app_with_cors()` in `tests/http.rs`**

Change the `AppState` construction inside `app_with_cors()` from:

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
    cors_cache,
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
        .connect_timeout(std::time::Duration::from_millis(500))
        .timeout(std::time::Duration::from_millis(500))
        .build()
        .unwrap(),
    cors_cache,
    ssrf_cache: Arc::new(tokio::sync::RwLock::new(std::collections::HashMap::new())),
};
```

- [ ] **Step 5: Update `test_state()` in `src/routes/player.rs`**

In the `#[cfg(test)]` block, change `test_state()`:

Change:

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
    ssrf_cache: std::sync::Arc::new(tokio::sync::RwLock::new(
        std::collections::HashMap::new(),
    )),
}
```

- [ ] **Step 6: Switch handler to `is_safe_url_cached` in `src/routes/player.rs`**

In the `stream_proxy` function, change:

```rust
if let Err(e) = crate::ssrf::is_safe_url(&url).await {
```

to:

```rust
if let Err(e) = crate::ssrf::is_safe_url_cached(&url, &state.ssrf_cache).await {
```

- [ ] **Step 7: Verify all tests pass**

```bash
cd /Users/kunwu/Workspace/playground/MyTV && cargo test
```

Expected: all tests pass (no behavior change yet for streaming — proxy_client timeout change is invisible to the test suite since tests use 500 ms timeouts).

- [ ] **Step 8: Run fmt and commit**

```bash
cd /Users/kunwu/Workspace/playground/MyTV && cargo fmt && git add src/lib.rs src/main.rs tests/http.rs src/routes/player.rs && git commit -m "feat: wire ssrf_cache into AppState; split proxy_client connect/read timeouts"
```

---

### Task 3: Segment streaming — reqwest `stream` feature + `Body::from_stream`

**Files:**
- Modify: `Cargo.toml`
- Modify: `src/routes/player.rs`

- [ ] **Step 1: Add `stream` to reqwest features in `Cargo.toml`**

Change:

```toml
reqwest = { version = "0.12", features = ["rustls-tls", "json"], default-features = false }
```

to:

```toml
reqwest = { version = "0.12", features = ["rustls-tls", "json", "stream"], default-features = false }
```

- [ ] **Step 2: Verify compilation after feature addition**

```bash
cd /Users/kunwu/Workspace/playground/MyTV && cargo build
```

Expected: compiles without error.

- [ ] **Step 3: Replace the body-reading block in `stream_proxy`**

The current handler buffers ALL responses (both playlist and segment) in one `chunk()` loop, then branches. Replace everything from `const MAX_BODY` through the closing `}` of `stream_proxy` with a version that branches first and streams the segment path:

Replace (from `const MAX_BODY: usize = 20 * 1024 * 1024;` through the final `}` of `stream_proxy`):

```rust
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

with:

```rust
    let mut headers = HeaderMap::new();
    headers.insert("access-control-allow-origin", HeaderValue::from_static("*"));

    if is_playlist {
        const MAX_BODY: usize = 20 * 1024 * 1024;
        let mut collected: Vec<u8> = Vec::new();
        let mut upstream = upstream;
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
        (headers, axum::body::Body::from_stream(upstream.bytes_stream())).into_response()
    }
}
```

Note: `upstream` in the segment branch no longer needs `mut` since `bytes_stream()` consumes it by value. The `mut` on the outer `let mut upstream` binding is still needed for the playlist branch (re-bound as `let mut upstream = upstream`).

Also note: `Bytes` import (`use axum::body::Bytes`) stays — the playlist branch still uses `Bytes::from(collected)`.

- [ ] **Step 4: Run all tests**

```bash
cd /Users/kunwu/Workspace/playground/MyTV && cargo test
```

Expected: all tests pass. The existing error-path tests (`stream_proxy_blocks_loopback`, `stream_proxy_blocks_link_local_metadata`, `stream_proxy_blocks_private_rfc1918`, `stream_proxy_rejects_non_http_scheme`) still cover the SSRF check paths; those paths return before reaching the stream/buffer branch.

- [ ] **Step 5: Run fmt and commit**

```bash
cd /Users/kunwu/Workspace/playground/MyTV && cargo fmt && git add Cargo.toml src/routes/player.rs && git commit -m "feat: stream segment responses via Body::from_stream; remove 20 MB cap on segment path"
```
