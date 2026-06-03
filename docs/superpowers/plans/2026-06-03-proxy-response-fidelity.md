# Proxy Response Fidelity Implementation Plan

> **For agentic workers:** REQUIRED SUB-SKILL: Use superpowers:subagent-driven-development (recommended) or superpowers:executing-plans to implement this plan task-by-task. Steps use checkbox (`- [ ]`) syntax for tracking.

**Goal:** Make `/stream-proxy` a transparent proxy — pass through the upstream HTTP status code, forward all upstream response headers (minus two exceptions), and forward the browser's `Range` header to the CDN on every redirect hop.

**Architecture:** All changes are in the `stream_proxy` function in `src/routes/player.rs`. Add a `HeaderMap` extractor for browser request headers, forward `Range` inside the redirect loop, extract the upstream status before consuming the response, copy all upstream response headers skipping `access-control-allow-origin`, and remove `content-length` on the playlist path (body changes after URL rewriting).

**Tech Stack:** Rust 1.96, Axum 0.7, `axum::http::{HeaderMap, HeaderValue, StatusCode, header}`

---

### Task 1: Rewrite `stream_proxy` for full response fidelity

**Files:**
- Modify: `src/routes/player.rs`

This is the full rewrite of `stream_proxy`. The function currently always returns HTTP 200 and only forwards `Content-Type` and `Access-Control-Allow-Origin`. After this change it passes through the upstream status, forwards all response headers (with two exceptions), and forwards the browser's `Range` header to the CDN.

**Current `stream_proxy` function (lines 190–286 of `src/routes/player.rs`):**

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
        // DNS resolved at check time; a hostile server can rebind between check and connect (TOCTOU).
        if let Err(e) = crate::ssrf::is_safe_url_cached(&url, &state.ssrf_cache).await {
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

    let upstream = match upstream {
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
        (
            headers,
            axum::body::Body::from_stream(upstream.bytes_stream()),
        )
            .into_response()
    }
}
```

- [ ] **Step 1: Replace the full `stream_proxy` function**

Replace the entire function with the version below. Key changes vs. the current code:

1. **Signature**: adds `request_headers: HeaderMap` as a third extractor — Axum extracts the browser's request headers automatically.
2. **Redirect loop**: attaches browser's `Range` header to every CDN request.
3. **Status extraction**: `let status = upstream.status()` before consuming the response.
4. **Header copy**: after setting `access-control-allow-origin: *`, iterate `upstream.headers()` and copy every header except `access-control-allow-origin` (we own it). reqwest/hyper has already stripped hop-by-hop headers (`Transfer-Encoding`, `Connection`) so they never appear.
5. **Playlist path**: removes `content-length` after the copy (rewritten body has a different length); sets `content-type` to `application/vnd.apple.mpegurl`; returns `(status, headers, rewritten)`.
6. **Segment path**: retains all copied headers including `content-length`; returns `(status, headers, Body::from_stream(...))`. The redundant explicit `content-type` insert is removed since it's already copied from upstream.

```rust
pub async fn stream_proxy(
    State(state): State<AppState>,
    Query(q): Query<StreamProxyQuery>,
    request_headers: HeaderMap,
) -> Response {
    if !q.url.starts_with("http://") && !q.url.starts_with("https://") {
        return StatusCode::BAD_REQUEST.into_response();
    }

    let mut url = q.url.clone();
    let mut upstream = None;

    for _ in 0..5 {
        // DNS resolved at check time; a hostile server can rebind between check and connect (TOCTOU).
        if let Err(e) = crate::ssrf::is_safe_url_cached(&url, &state.ssrf_cache).await {
            tracing::warn!(url = %url, reason = %e, "stream proxy SSRF check failed");
            return StatusCode::UNPROCESSABLE_ENTITY.into_response();
        }
        let mut req = state.proxy_client.get(&url);
        if let Some(range) = request_headers.get(axum::http::header::RANGE) {
            req = req.header(axum::http::header::RANGE, range);
        }
        let resp = match req.send().await {
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

    let upstream = match upstream {
        Some(r) => r,
        None => return StatusCode::BAD_GATEWAY.into_response(),
    };

    let status = upstream.status();

    let ct = upstream
        .headers()
        .get(axum::http::header::CONTENT_TYPE)
        .and_then(|v| v.to_str().ok())
        .unwrap_or("")
        .to_string();

    let is_playlist = ct.contains("mpegurl") || url.contains(".m3u8") || url.contains(".m3u");

    let mut headers = HeaderMap::new();
    headers.insert(
        axum::http::header::ACCESS_CONTROL_ALLOW_ORIGIN,
        HeaderValue::from_static("*"),
    );
    for (key, val) in upstream.headers() {
        if key == axum::http::header::ACCESS_CONTROL_ALLOW_ORIGIN {
            continue;
        }
        headers.insert(key.clone(), val.clone());
    }

    if is_playlist {
        headers.remove(axum::http::header::CONTENT_LENGTH);
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
        (status, headers, rewritten).into_response()
    } else {
        (
            status,
            headers,
            axum::body::Body::from_stream(upstream.bytes_stream()),
        )
            .into_response()
    }
}
```

- [ ] **Step 2: Run all tests**

```bash
cd /Users/kunwu/Workspace/playground/MyTV && cargo test
```

Expected: all 178 tests pass. The four SSRF guard tests still return 422/400 before reaching the new header logic.

- [ ] **Step 3: Run fmt and commit**

```bash
cd /Users/kunwu/Workspace/playground/MyTV && cargo fmt && git add src/routes/player.rs && git commit -m "feat: proxy response fidelity — status passthrough, full header forwarding, Range request forwarding"
```
