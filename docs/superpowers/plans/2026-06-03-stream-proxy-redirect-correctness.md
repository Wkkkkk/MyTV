# Stream Proxy Redirect Correctness Implementation Plan

> **For agentic workers:** REQUIRED SUB-SKILL: Use superpowers:subagent-driven-development (recommended) or superpowers:executing-plans to implement this plan task-by-task. Steps use checkbox (`- [ ]`) syntax for tracking.

**Goal:** Fix `stream_proxy` so that relative `Location` headers (e.g. `/live/index.m3u8`) are resolved against the current URL before the next redirect hop, instead of being passed raw to `is_safe_url_cached` and causing a 422.

**Architecture:** A small private sync helper `resolve_location(location, base_url) -> Option<String>` is added to `src/routes/player.rs`. The redirect loop replaces the single-line `url = location` with a call to this helper (returning `BAD_GATEWAY` on `None`). Task 1 adds the helper with unit tests (TDD). Task 2 adds the integration test first, then wires the helper into the redirect loop to make it pass.

**Tech Stack:** Rust 1.96, `reqwest::Url::join` (already a dep via reqwest 0.12), `tokio::net::TcpListener` for the integration test mock server.

---

### Task 1: `resolve_location` helper + unit tests (TDD)

**Files:**
- Modify: `src/routes/player.rs`

The helper is sync (no `async`), so its unit tests use `#[test]`, not `#[tokio::test]`.

- [ ] **Step 1: Add 4 failing unit tests to `src/routes/player.rs`**

Append these tests inside the existing `#[cfg(test)] mod tests` block (which starts at line 317). Add them after the last existing test in that block:

```rust
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
```

- [ ] **Step 2: Run tests to verify they fail**

```bash
cargo test resolve_location
```

Expected: all 4 FAIL — `resolve_location` is not defined.

- [ ] **Step 3: Add `resolve_location` to `src/routes/player.rs`**

Add this function just above `pub async fn stream_proxy` (around line 190, before `stream_proxy`):

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

- [ ] **Step 4: Run tests to verify all 4 pass**

```bash
cargo test resolve_location
```

Expected: all 4 PASS.

- [ ] **Step 5: Run full suite for regressions**

```bash
cargo test
```

Expected: all tests pass. No behavior change yet — helper exists but is not called.

- [ ] **Step 6: Run fmt and commit**

```bash
cargo fmt
git add src/routes/player.rs
git commit -m "feat: add resolve_location helper for relative HTTP redirect URLs"
```

---

### Task 2: Integration test + redirect loop fix (TDD)

**Files:**
- Modify: `tests/http.rs`
- Modify: `src/routes/player.rs`

The integration test spins up a real `TcpListener` that serves two sequential TCP connections: the first returns a `302` with a relative `Location`, the second returns `200` with HLS content. The test asserts `200 OK`. Without the fix, `stream_proxy` returns `422` (relative URL fails SSRF parse) — so the test fails first, then the fix makes it pass.

- [ ] **Step 1: Add the failing integration test to `tests/http.rs`**

Append this test at the end of `tests/http.rs` (after `stream_proxy_strips_hop_by_hop_headers`):

```rust
#[tokio::test]
async fn stream_proxy_follows_relative_redirect() {
    use tokio::io::{AsyncReadExt, AsyncWriteExt};

    let listener = tokio::net::TcpListener::bind("127.0.0.1:0").await.unwrap();
    let port = listener.local_addr().unwrap().port();

    tokio::spawn(async move {
        let mut buf = [0u8; 512];

        // First connection: 302 with a relative Location header.
        let (mut conn, _) = listener.accept().await.unwrap();
        let _ = conn.read(&mut buf).await;
        conn.write_all(
            b"HTTP/1.1 302 Found\r\n\
              Location: /redirected.m3u8\r\n\
              Content-Length: 0\r\n\
              \r\n",
        )
        .await
        .unwrap();
        drop(conn);

        // Second connection: the resolved redirect target returns HLS content.
        let (mut conn, _) = listener.accept().await.unwrap();
        let _ = conn.read(&mut buf).await;
        conn.write_all(
            b"HTTP/1.1 200 OK\r\n\
              Content-Type: application/vnd.apple.mpegurl\r\n\
              Content-Length: 7\r\n\
              \r\n\
              #EXTM3U",
        )
        .await
        .unwrap();
    });

    // app_with_ssrf_bypass pre-seeds 127.0.0.1 in the ssrf_cache so the SSRF
    // check passes for localhost (same pattern as stream_proxy_strips_hop_by_hop_headers).
    let app = app_with_ssrf_bypass("127.0.0.1").await;
    let url_param = format!("http%3A%2F%2F127.0.0.1%3A{}%2Foriginal.m3u8", port);
    let response = app
        .oneshot(req(&format!("/stream-proxy?url={}", url_param)))
        .await
        .unwrap();

    assert_eq!(
        response.status(),
        StatusCode::OK,
        "stream_proxy must follow a relative Location redirect (got {} instead)",
        response.status()
    );
}
```

- [ ] **Step 2: Run the new test to verify it fails**

```bash
cargo test stream_proxy_follows_relative_redirect
```

Expected: FAIL — current code returns 422 because the raw `/redirected.m3u8` string fails `is_safe_url_cached`.

- [ ] **Step 3: Fix the redirect loop in `src/routes/player.rs`**

In `stream_proxy`, locate this line (around line 228):

```rust
            url = location;
```

Replace it with:

```rust
            url = match resolve_location(&location, &url) {
                Some(resolved) => resolved,
                None => return StatusCode::BAD_GATEWAY.into_response(),
            };
```

The full redirect branch (lines 219–229) now looks like:

```rust
        if resp.status().is_redirection() {
            let location = match resp
                .headers()
                .get(axum::http::header::LOCATION)
                .and_then(|v| v.to_str().ok())
            {
                Some(loc) => loc.to_string(),
                None => return StatusCode::BAD_GATEWAY.into_response(),
            };
            url = match resolve_location(&location, &url) {
                Some(resolved) => resolved,
                None => return StatusCode::BAD_GATEWAY.into_response(),
            };
            continue;
        }
```

- [ ] **Step 4: Run the integration test to verify it passes**

```bash
cargo test stream_proxy_follows_relative_redirect
```

Expected: PASS — proxy follows the relative redirect and returns `200 OK`.

- [ ] **Step 5: Run the full test suite**

```bash
cargo test
```

Expected: all tests pass.

- [ ] **Step 6: Run fmt and commit**

```bash
cargo fmt
git add src/routes/player.rs tests/http.rs
git commit -m "fix: resolve relative Location headers in stream_proxy redirect loop"
```
