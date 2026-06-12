# Stream-Proxy Deep Module Implementation Plan

> **For agentic workers:** REQUIRED SUB-SKILL: Use superpowers:subagent-driven-development (recommended) or superpowers:executing-plans to implement this plan task-by-task. Steps use checkbox (`- [ ]`) syntax for tracking.

**Goal:** Collapse the ~150-line `stream_proxy` HTTP handler into a top-level `src/proxy.rs` deep module behind one entry point, `fetch_rewritten`, so the handler is a one-line delegation and the redirect/SSRF/header/detect/rewrite concerns become internal seams.

**Architecture:** New `src/proxy.rs` (sibling of `broadcast`/`health`/`budget`) exposes `pub async fn fetch_rewritten(state, url, request_headers) -> Response`, owning the full proxy operation. Private internal seams: `follow_redirects` (5-attempt loop + per-hop SSRF), `build_proxy_headers` (CORS + RFC 7230 hop-by-hop strip — newly pure and unit-tested), plus the moved-verbatim `resolve_location`, `detect_playlist`, `resolve_direct_segments`. The `stream_proxy` handler in `player.rs` shrinks to delegating to it.

**Tech Stack:** Rust 1.96, Axum 0.7, reqwest, futures_util, tokio.

**Behavior is byte-identical** — see the spec (`docs/superpowers/specs/2026-06-12-proxy-deep-module-design.md`) §"Behavior preservation". All `tests/http.rs` stream-proxy integration tests must stay green **without modification**; they are the behavior contract.

**Sequencing:** Task 1 adds `src/proxy.rs` as a complete parallel implementation (it is `pub`, its seams are used by `fetch_rewritten`, so no dead-code warnings) without touching `player.rs` — stays green. Task 2 rewires the handler, deletes the now-orphaned `player.rs` code, and cleans imports — stays green, now routing the integration tests through `fetch_rewritten`.

**Always:** `cargo fmt` before each commit; `cargo clippy --all-targets -- -D warnings` clean; commit messages end with the `Co-Authored-By: Claude Opus 4.8 (1M context) <noreply@anthropic.com>` trailer.

---

## Task 1: Create `src/proxy.rs` with `fetch_rewritten` + internal seams

**Files:**
- Create: `src/proxy.rs`
- Modify: `src/lib.rs` (add `pub mod proxy;`)

Do **not** touch `src/routes/player.rs` in this task. The functions `resolve_location`/`detect_playlist`/`resolve_direct_segments` will transiently exist in both files — that is expected and removed in Task 2.

- [ ] **Step 1: Register the module**

In `src/lib.rs`, add the module declaration. It sits with the other top-level modules; place it alphabetically near `broadcast`/`budget` (exact neighbour line ordering is cosmetic — `cargo fmt` will not reorder it, so just add it next to `pub mod broadcast;`):

```rust
pub mod proxy;
```

- [ ] **Step 2: Write `src/proxy.rs` in full**

Create `src/proxy.rs` with exactly this content:

```rust
use axum::{
    body::Bytes,
    http::{HeaderMap, HeaderValue, StatusCode},
    response::{IntoResponse, Response},
};
use futures_util::StreamExt;
use std::sync::atomic::Ordering;

use crate::media::{hls, mpd};
use crate::AppState;

/// Proxies `url`, following redirects, rewriting HLS/DASH manifests, and
/// streaming non-playlist bodies. Owns the entire stream-proxy operation;
/// the HTTP handler is a one-line delegation.
pub async fn fetch_rewritten(
    state: &AppState,
    url: String,
    request_headers: &HeaderMap,
) -> Response {
    if !url.starts_with("http://") && !url.starts_with("https://") {
        return StatusCode::BAD_REQUEST.into_response();
    }

    let (mut upstream, url) = match follow_redirects(state, url, request_headers).await {
        Ok(pair) => pair,
        Err(code) => return code.into_response(),
    };

    let status = upstream.status();

    let ct = upstream
        .headers()
        .get(axum::http::header::CONTENT_TYPE)
        .and_then(|v| v.to_str().ok())
        .unwrap_or("")
        .to_owned();

    let is_playlist = detect_playlist(&ct, &url);

    let mut headers = build_proxy_headers(upstream.headers());

    if is_playlist {
        headers.remove(axum::http::header::CONTENT_LENGTH);
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
        state
            .metrics
            .proxy_bytes
            .fetch_add(body_bytes.len() as u64, Ordering::Relaxed);
        let text = String::from_utf8_lossy(&body_bytes);
        let direct = resolve_direct_segments(state, &url).await;
        let is_dash_playlist = ct.contains("dash+xml") || url.to_lowercase().ends_with(".mpd");
        let (rewritten, content_type) = if is_dash_playlist {
            (
                mpd::rewrite_mpd_urls(&text, &url, direct),
                "application/dash+xml",
            )
        } else {
            (
                hls::rewrite_hls_urls(&text, &url, direct),
                "application/vnd.apple.mpegurl",
            )
        };
        headers.insert(
            axum::http::header::CONTENT_TYPE,
            HeaderValue::from_static(content_type),
        );
        (status, headers, rewritten).into_response()
    } else {
        // Guard lives inside the closure, so active_streams decrements when the
        // client drops the body stream, not when this handler returns.
        let guard = crate::metrics::ActiveStreamGuard::new(state.metrics.clone());
        let metrics = state.metrics.clone();
        let counted = upstream.bytes_stream().inspect(move |chunk| {
            // Referencing guard makes the move-closure capture it for the stream's lifetime.
            let _hold = &guard;
            if let Ok(c) = chunk {
                metrics
                    .proxy_bytes
                    .fetch_add(c.len() as u64, Ordering::Relaxed);
            }
        });
        (status, headers, axum::body::Body::from_stream(counted)).into_response()
    }
}

/// Follows up to 5 redirects, re-running the SSRF check on every hop (DNS is
/// resolved at check time; a hostile server can rebind between check and
/// connect — TOCTOU). Forwards the client `Range` header. Returns the final
/// upstream response and resolved URL, or the appropriate error status:
/// `422` on SSRF failure, `502` on fetch error / bad `Location` / 5 attempts
/// exhausted.
async fn follow_redirects(
    state: &AppState,
    mut url: String,
    request_headers: &HeaderMap,
) -> Result<(reqwest::Response, String), StatusCode> {
    for _ in 0..5 {
        if let Err(e) = crate::ssrf::is_safe_url_cached(&url, &state.ssrf_cache).await {
            tracing::warn!(url = %url, reason = %e, "stream proxy SSRF check failed");
            return Err(StatusCode::UNPROCESSABLE_ENTITY);
        }
        let mut req = state.proxy_client.get(&url);
        if let Some(range) = request_headers.get(axum::http::header::RANGE) {
            req = req.header(axum::http::header::RANGE, range);
        }
        let resp = match req.send().await {
            Ok(r) => r,
            Err(e) => {
                tracing::warn!(url = %url, error = %e, "stream proxy fetch failed");
                return Err(StatusCode::BAD_GATEWAY);
            }
        };
        if resp.status().is_redirection() {
            let location = match resp
                .headers()
                .get(axum::http::header::LOCATION)
                .and_then(|v| v.to_str().ok())
            {
                Some(loc) => loc.to_string(),
                None => return Err(StatusCode::BAD_GATEWAY),
            };
            url = match resolve_location(&location, &url) {
                Some(resolved) => resolved,
                None => return Err(StatusCode::BAD_GATEWAY),
            };
            continue;
        }
        return Ok((resp, url));
    }
    Err(StatusCode::BAD_GATEWAY)
}

/// Builds the response header map: sets the `Access-Control-Allow-Origin: *`
/// header we own, and strips hop-by-hop headers (RFC 7230 §6.1) plus every
/// header named in the upstream `Connection` value.
fn build_proxy_headers(upstream: &HeaderMap) -> HeaderMap {
    // RFC 7230 §6.1: collect headers listed in Connection so we can strip them too.
    let connection_options: Vec<String> = upstream
        .get(axum::http::header::CONNECTION)
        .and_then(|v| v.to_str().ok())
        .map(|s| s.split(',').map(|t| t.trim().to_lowercase()).collect())
        .unwrap_or_default();

    let mut headers = HeaderMap::new();
    headers.insert(
        axum::http::header::ACCESS_CONTROL_ALLOW_ORIGIN,
        HeaderValue::from_static("*"),
    );
    for (key, val) in upstream {
        // Never forward CORS header (we own it) or hop-by-hop headers (RFC 7230 §6.1).
        if key == axum::http::header::ACCESS_CONTROL_ALLOW_ORIGIN
            || key == axum::http::header::CONNECTION
            || key == axum::http::header::TRANSFER_ENCODING
            || key == axum::http::header::TE
            || key == axum::http::header::TRAILER
            || key == axum::http::header::UPGRADE
            || connection_options.iter().any(|o| o == key.as_str())
        {
            continue;
        }
        headers.append(key.clone(), val.clone());
    }
    headers
}

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

fn detect_playlist(content_type: &str, url: &str) -> bool {
    let lower = url.to_lowercase();
    let path = &lower[..lower.find('?').unwrap_or(lower.len())];
    let is_dash = content_type.contains("dash+xml") || path.ends_with(".mpd");
    is_dash || content_type.contains("mpegurl") || path.ends_with(".m3u8") || path.ends_with(".m3u")
}

async fn resolve_direct_segments(state: &AppState, base_url: &str) -> bool {
    let host_key = crate::media::hls::extract_manifest_host(base_url);
    {
        let cache = state.cors_cache.read().await;
        if let Some(&cached) = cache.get(&host_key) {
            return cached;
        }
    }
    // Cache miss: delegate to health::probe_and_cache_cors.
    // Re-fetches the manifest internally; cache misses are rare (once per host per session).
    crate::health::probe_and_cache_cors(&state.http_client, &state.cors_cache, base_url)
        .await
        .unwrap_or(false)
}

#[cfg(test)]
mod tests {
    use super::*;

    // ── build_proxy_headers ────────────────────────────────────────────────

    #[test]
    fn build_proxy_headers_sets_cors_when_upstream_omits_it() {
        let out = build_proxy_headers(&HeaderMap::new());
        assert_eq!(
            out.get(axum::http::header::ACCESS_CONTROL_ALLOW_ORIGIN)
                .unwrap(),
            "*"
        );
    }

    #[test]
    fn build_proxy_headers_overwrites_upstream_cors() {
        let mut upstream = HeaderMap::new();
        upstream.insert(
            axum::http::header::ACCESS_CONTROL_ALLOW_ORIGIN,
            HeaderValue::from_static("https://evil.example"),
        );
        let out = build_proxy_headers(&upstream);
        let values: Vec<_> = out
            .get_all(axum::http::header::ACCESS_CONTROL_ALLOW_ORIGIN)
            .iter()
            .collect();
        assert_eq!(values, vec!["*"]);
    }

    #[test]
    fn build_proxy_headers_strips_standard_hop_by_hop() {
        let mut upstream = HeaderMap::new();
        upstream.insert(
            axum::http::header::TRANSFER_ENCODING,
            HeaderValue::from_static("chunked"),
        );
        upstream.insert(axum::http::header::TE, HeaderValue::from_static("trailers"));
        upstream.insert(
            axum::http::header::TRAILER,
            HeaderValue::from_static("Expires"),
        );
        upstream.insert(axum::http::header::UPGRADE, HeaderValue::from_static("h2c"));
        upstream.insert(
            axum::http::header::CONNECTION,
            HeaderValue::from_static("keep-alive"),
        );
        let out = build_proxy_headers(&upstream);
        assert!(out.get(axum::http::header::TRANSFER_ENCODING).is_none());
        assert!(out.get(axum::http::header::TE).is_none());
        assert!(out.get(axum::http::header::TRAILER).is_none());
        assert!(out.get(axum::http::header::UPGRADE).is_none());
        assert!(out.get(axum::http::header::CONNECTION).is_none());
    }

    #[test]
    fn build_proxy_headers_strips_connection_named_headers() {
        let mut upstream = HeaderMap::new();
        upstream.insert(
            axum::http::header::CONNECTION,
            HeaderValue::from_static("X-Custom-Hop"),
        );
        upstream.insert("x-custom-hop", HeaderValue::from_static("secret"));
        let out = build_proxy_headers(&upstream);
        assert!(out.get("x-custom-hop").is_none());
    }

    #[test]
    fn build_proxy_headers_preserves_ordinary_header() {
        let mut upstream = HeaderMap::new();
        upstream.insert(
            axum::http::header::CONTENT_TYPE,
            HeaderValue::from_static("video/mp2t"),
        );
        let out = build_proxy_headers(&upstream);
        assert_eq!(
            out.get(axum::http::header::CONTENT_TYPE).unwrap(),
            "video/mp2t"
        );
    }

    // ── resolve_location ───────────────────────────────────────────────────

    #[test]
    fn resolve_location_absolute_passthrough() {
        assert_eq!(
            resolve_location(
                "https://cdn.example.com/new.m3u8",
                "https://origin.example.com/old.m3u8",
            ),
            Some("https://cdn.example.com/new.m3u8".to_string())
        );
    }

    #[test]
    fn resolve_location_root_relative() {
        assert_eq!(
            resolve_location("/live/index.m3u8", "https://cdn.example.com/old/path.m3u8"),
            Some("https://cdn.example.com/live/index.m3u8".to_string())
        );
    }

    #[test]
    fn resolve_location_relative_path() {
        assert_eq!(
            resolve_location("index.m3u8", "https://cdn.example.com/live/master.m3u8"),
            Some("https://cdn.example.com/live/index.m3u8".to_string())
        );
    }

    #[test]
    fn resolve_location_unparseable_base_returns_none() {
        assert_eq!(resolve_location("/path", "not-a-url"), None);
    }

    #[test]
    fn resolve_location_protocol_relative() {
        assert_eq!(
            resolve_location(
                "//cdn.example.com/stream.m3u8",
                "https://origin.example.com/master.m3u8",
            ),
            Some("https://cdn.example.com/stream.m3u8".to_string())
        );
    }

    // ── detect_playlist ────────────────────────────────────────────────────

    #[test]
    fn test_detect_playlist_hls_manifest_by_extension() {
        assert!(detect_playlist(
            "application/octet-stream",
            "https://cdn.example.com/live/index.m3u8"
        ));
    }

    #[test]
    fn test_detect_playlist_hls_manifest_by_content_type() {
        assert!(detect_playlist(
            "application/vnd.apple.mpegurl",
            "https://cdn.example.com/live/stream"
        ));
    }

    #[test]
    fn test_detect_playlist_dash_manifest() {
        assert!(detect_playlist(
            "application/dash+xml",
            "https://cdn.example.com/stream.mpd"
        ));
    }

    #[test]
    fn test_detect_playlist_youtube_live_segment_not_playlist() {
        // YouTube live segment URL has .m3u8 as an intermediate path component,
        // not the final path element — must NOT be treated as a playlist.
        let seg_url = "https://rr1---sn-cgxqc5oqufv-5gos.googlevideo.com/videoplayback/\
            id/abc123/itag/300/source/yt_live_broadcast/\
            playlist/index.m3u8/sq/174385/file/seg.ts";
        assert!(!detect_playlist("application/octet-stream", seg_url));
    }

    #[test]
    fn test_detect_playlist_youtube_live_segment_with_query_not_playlist() {
        let seg_url = "https://cdn.example.com/playlist/index.m3u8/sq/123/file/seg.ts?expire=999";
        assert!(!detect_playlist("application/octet-stream", seg_url));
    }

    #[test]
    fn test_detect_playlist_plain_ts_segment_not_playlist() {
        assert!(!detect_playlist(
            "application/octet-stream",
            "https://cdn.example.com/hls/seg1.ts"
        ));
    }
}
```

- [ ] **Step 3: Format**

Run: `cargo fmt`
Expected: no errors.

- [ ] **Step 4: Compile all targets (incl. lib test modules)**

Run: `cargo test --no-run`
Expected: compiles cleanly. (Note: after creating a new module file, an LSP "unresolved module" / E0583 diagnostic can appear spuriously — trust `cargo test --no-run`, which compiles the real thing. If it compiles, the module is wired.)

- [ ] **Step 5: Run the new + moved unit tests**

Run: `cargo test --lib proxy::`
Expected: PASS — 5 `build_proxy_headers_*`, 5 `resolve_location_*`, 6 `*detect_playlist*` (16 tests).

- [ ] **Step 6: Full green check**

Run: `cargo test && cargo fmt --check && cargo clippy --all-targets -- -D warnings`
Expected: all pass, no warnings. `player.rs` still has its own copies of the helpers and the fat handler — that is intended for this task; both compile.

- [ ] **Step 7: Commit**

```bash
cargo fmt
git add src/proxy.rs src/lib.rs
git commit -m "$(cat <<'EOF'
feat(proxy): add fetch_rewritten deep module with internal seams

Co-Authored-By: Claude Opus 4.8 (1M context) <noreply@anthropic.com>
EOF
)"
```

---

## Task 2: Rewire the handler and delete the orphaned `player.rs` code

**Files:**
- Modify: `src/routes/player.rs` (shrink `stream_proxy`; delete moved helpers + their tests; clean imports)

- [ ] **Step 1: Replace the `stream_proxy` handler body**

In `src/routes/player.rs`, replace the entire `stream_proxy` function (currently `pub async fn stream_proxy(...) { ... }`, from its signature through its closing brace — the ~150-line body ending just before `#[cfg(test)]`) with this thin delegation:

```rust
pub async fn stream_proxy(
    State(state): State<AppState>,
    Query(q): Query<StreamProxyQuery>,
    request_headers: HeaderMap,
) -> Response {
    crate::proxy::fetch_rewritten(&state, q.url, &request_headers).await
}
```

Keep the `// ── stream proxy ──` section comment and the `StreamProxyQuery` struct above it as they are.

- [ ] **Step 2: Delete the three moved helper functions**

Delete these three functions in their entirety from `player.rs` (they now live in `proxy.rs`):
- `fn resolve_location(location: &str, base_url: &str) -> Option<String> { ... }`
- `fn detect_playlist(content_type: &str, url: &str) -> bool { ... }`
- `async fn resolve_direct_segments(state: &AppState, base_url: &str) -> bool { ... }`

- [ ] **Step 3: Delete the moved unit tests**

In `player.rs`'s `#[cfg(test)] mod tests`, delete the test blocks now duplicated in `proxy.rs`: the `// ── resolve_location ──` section (5 tests: `resolve_location_absolute_passthrough`, `_root_relative`, `_relative_path`, `_unparseable_base_returns_none`, `_protocol_relative`) and the `// ── detect_playlist ──` section (6 tests: `test_detect_playlist_hls_manifest_by_extension`, `_by_content_type`, `_dash_manifest`, `_youtube_live_segment_not_playlist`, `_youtube_live_segment_with_query_not_playlist`, `_plain_ts_segment_not_playlist`), including their section-comment headers.

- [ ] **Step 4: Clean the now-unused imports**

In `player.rs`'s top `use` block, remove the imports that only the moved code used. The result must be exactly:

```rust
use axum::{
    extract::{Path, Query, State},
    http::{HeaderMap, StatusCode},
    response::{Json, Response},
};
use serde::{Deserialize, Serialize};

use crate::{
    media::resolver,
    model::{
        channel::{self, ChannelType},
        playlist_item, source,
    },
    AppState,
};
```

Removed: `body::Bytes`, `http::HeaderValue`, `response::IntoResponse`, `use futures_util::StreamExt;`, `use std::sync::atomic::Ordering;`, and `hls`/`mpd` from the `media` import (only `resolver` remains). `HeaderMap` stays (the thin handler uses it).

- [ ] **Step 5: Format**

Run: `cargo fmt`
Expected: no errors.

- [ ] **Step 6: Compile all targets**

Run: `cargo test --no-run`
Expected: compiles cleanly with no unused-import warnings.

- [ ] **Step 7: Run the proxy integration tests (the behavior contract)**

Run: `cargo test --test http stream_proxy && cargo test --test http test_stream_proxy_rewrites_dash`
Expected: PASS — `stream_proxy_blocks_loopback`, `stream_proxy_blocks_link_local_metadata`, `stream_proxy_blocks_private_rfc1918`, `stream_proxy_rejects_non_http_scheme`, `stream_proxy_strips_hop_by_hop_headers`, `stream_proxy_follows_relative_redirect`, `test_stream_proxy_rewrites_dash_bbb_manifest`. They are unchanged and now route through `fetch_rewritten`.

- [ ] **Step 8: Full green check**

Run: `cargo test && cargo fmt --check && cargo clippy --all-targets -- -D warnings`
Expected: all pass, no warnings.

- [ ] **Step 9: Commit**

```bash
cargo fmt
git add src/routes/player.rs
git commit -m "$(cat <<'EOF'
refactor(player): delegate stream_proxy to proxy::fetch_rewritten

Co-Authored-By: Claude Opus 4.8 (1M context) <noreply@anthropic.com>
EOF
)"
```

---

## Final verification (after both tasks)

- [ ] `cargo test` — all pass, 0 failed (expect the same total as before: the 11 moved unit tests relocated, +5 new `build_proxy_headers` tests, net +5).
- [ ] `cargo fmt --check` — clean.
- [ ] `cargo clippy --all-targets -- -D warnings` — clean.
- [ ] `git grep -n "fn resolve_location\|fn detect_playlist\|fn resolve_direct_segments" src/routes/player.rs` returns nothing (helpers fully moved).
- [ ] `stream_proxy` in `player.rs` is a one-line delegation to `crate::proxy::fetch_rewritten`.
