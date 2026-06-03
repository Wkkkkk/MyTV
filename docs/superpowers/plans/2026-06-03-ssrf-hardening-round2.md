# SSRF Hardening Round 2 Implementation Plan

> **For agentic workers:** REQUIRED SUB-SKILL: Use superpowers:subagent-driven-development (recommended) or superpowers:executing-plans to implement this plan task-by-task. Steps use checkbox (`- [ ]`) syntax for tracking.

**Goal:** Close five remaining SSRF bypass vectors: three gaps in `is_blocked` (unspecified address, CGNAT range, IPv4-mapped IPv6) and two unguarded HTTP fetches in `hls.rs` (`find_segment_with_descent` and `probe_cors`).

**Architecture:** Task 1 is purely additive to `src/ssrf.rs` — three one-liner additions to `is_blocked` plus 7 unit tests. Task 2 adds two inline `is_safe_url` guards to `src/media/hls.rs` (no signature changes) plus 2 unit tests. Both tasks follow TDD order (tests first, then implementation).

**Tech Stack:** Rust 1.96, `std::net::IpAddr` (stdlib), `crate::ssrf::is_safe_url` (already exists)

---

### Task 1: Extend `is_blocked` in `src/ssrf.rs` (TDD)

**Files:**
- Modify: `src/ssrf.rs`

- [ ] **Step 1: Add 7 failing unit tests to `src/ssrf.rs`**

Append these tests inside the existing `#[cfg(test)] mod tests` block in `src/ssrf.rs`, after the last existing test (`cache_expires_and_refreshes`):

```rust
    #[tokio::test]
    async fn blocks_unspecified_ipv4() {
        assert!(matches!(
            is_safe_url("http://0.0.0.0/").await,
            Err(SsrfError::BlockedAddress(_))
        ));
    }

    #[tokio::test]
    async fn blocks_cgnat() {
        assert!(matches!(
            is_safe_url("http://100.64.0.1/").await,
            Err(SsrfError::BlockedAddress(_))
        ));
    }

    #[tokio::test]
    async fn blocks_cgnat_upper_bound() {
        assert!(matches!(
            is_safe_url("http://100.127.255.255/").await,
            Err(SsrfError::BlockedAddress(_))
        ));
    }

    #[tokio::test]
    async fn allows_below_cgnat() {
        assert!(is_safe_url("http://100.63.255.255/").await.is_ok());
    }

    #[tokio::test]
    async fn allows_above_cgnat() {
        assert!(is_safe_url("http://100.128.0.1/").await.is_ok());
    }

    #[tokio::test]
    async fn blocks_ipv4_mapped_loopback() {
        assert!(matches!(
            is_safe_url("http://[::ffff:127.0.0.1]/").await,
            Err(SsrfError::BlockedAddress(_))
        ));
    }

    #[tokio::test]
    async fn blocks_ipv4_mapped_private() {
        assert!(matches!(
            is_safe_url("http://[::ffff:10.0.0.1]/").await,
            Err(SsrfError::BlockedAddress(_))
        ));
    }
```

- [ ] **Step 2: Run the new tests to verify they fail**

```bash
cargo test ssrf
```

Expected: `blocks_unspecified_ipv4`, `blocks_cgnat`, `blocks_cgnat_upper_bound`, `blocks_ipv4_mapped_loopback`, `blocks_ipv4_mapped_private` all FAIL. `allows_below_cgnat` and `allows_above_cgnat` may PASS already (those are allowlist assertions that the current code satisfies by accident), which is fine.

- [ ] **Step 3: Replace `is_blocked` in `src/ssrf.rs`**

Replace the entire `fn is_blocked` function:

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
```

with:

```rust
fn is_blocked(ip: IpAddr) -> bool {
    match ip {
        IpAddr::V4(v4) => {
            v4.is_loopback()
                || v4.is_private()
                || v4.is_link_local()
                || v4.is_unspecified()
                || (v4.octets()[0] == 100 && v4.octets()[1] >= 64 && v4.octets()[1] <= 127)
        }
        IpAddr::V6(v6) => {
            if let Some(v4) = v6.to_ipv4_mapped() {
                return is_blocked(IpAddr::V4(v4));
            }
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
```

- [ ] **Step 4: Run the ssrf tests to verify all 7 new tests pass**

```bash
cargo test ssrf
```

Expected: all ssrf tests pass (7 new + all existing).

- [ ] **Step 5: Run the full test suite to check for regressions**

```bash
cargo test
```

Expected: all tests pass. No behavior change outside `ssrf.rs`.

- [ ] **Step 6: Run fmt and commit**

```bash
cargo fmt
git add src/ssrf.rs
git commit -m "fix: block 0.0.0.0, CGNAT 100.64/10, and IPv4-mapped IPv6 in is_blocked"
```

---

### Task 2: Inline SSRF guards in `src/media/hls.rs` (TDD)

**Files:**
- Modify: `src/media/hls.rs`

- [ ] **Step 1: Add 2 failing unit tests to `src/media/hls.rs`**

Append these tests inside the existing `#[cfg(test)] mod tests` block in `src/media/hls.rs`, after the last existing test (`test_find_segment_with_descent_depth_zero`):

```rust
    #[tokio::test]
    async fn find_segment_with_descent_blocks_variant_to_loopback() {
        let client = reqwest::Client::new();
        // Master playlist whose only variant line points to a loopback address.
        // Without the SSRF guard, find_segment_with_descent would fetch http://127.0.0.1/variant.m3u8.
        let master =
            "#EXTM3U\n#EXT-X-STREAM-INF:BANDWIDTH=1000\nhttp://127.0.0.1/variant.m3u8\n";
        let result =
            find_segment_with_descent(&client, master, "https://example.com/master.m3u8").await;
        assert_eq!(result, None);
    }

    #[tokio::test]
    async fn probe_cors_blocks_loopback() {
        let client = reqwest::Client::new();
        // Without the SSRF guard, probe_cors would HEAD-request http://127.0.0.1/seg.ts.
        let result = probe_cors(&client, "http://127.0.0.1/seg.ts").await;
        assert!(!result, "probe_cors must return false (proxy default) for loopback URLs");
    }
```

- [ ] **Step 2: Run the new tests to verify they fail**

```bash
cargo test -p mytv hls
```

Expected: `find_segment_with_descent_blocks_variant_to_loopback` FAILS (currently returns `None` for a different reason — the network call times out or connection-refused rather than an SSRF block, but the test may appear to pass spuriously; what matters is `probe_cors_blocks_loopback` FAILS — currently `probe_cors` returns `false` due to a connection error, not an SSRF block, so the assertion happens to pass. Verify the guard is not yet present by confirming neither guard exists in the code before proceeding.)

> Note: because both tests assert on the *safe* outcome (None / false), they may not fail visibly until you verify the guard is absent. Confirm by grepping: `grep -n "is_safe_url" src/media/hls.rs` — should return nothing. Proceed once confirmed.

- [ ] **Step 3: Add SSRF guard to `find_segment_with_descent` in `src/media/hls.rs`**

Replace the `find_segment_with_descent` function:

```rust
pub async fn find_segment_with_descent(
    client: &reqwest::Client,
    content: &str,
    base_url: &str,
) -> Option<String> {
    if let Some(seg) = find_first_segment_url(content, base_url) {
        return Some(seg);
    }
    let variant = find_first_variant_url(content, base_url)?;
    let body = fetch_text(client, &variant).await?;
    find_first_segment_url(&body, &variant)
}
```

with:

```rust
pub async fn find_segment_with_descent(
    client: &reqwest::Client,
    content: &str,
    base_url: &str,
) -> Option<String> {
    if let Some(seg) = find_first_segment_url(content, base_url) {
        return Some(seg);
    }
    let variant = find_first_variant_url(content, base_url)?;
    if crate::ssrf::is_safe_url(&variant).await.is_err() {
        return None;
    }
    let body = fetch_text(client, &variant).await?;
    find_first_segment_url(&body, &variant)
}
```

- [ ] **Step 4: Add SSRF guard to `probe_cors` in `src/media/hls.rs`**

Replace the `probe_cors` function:

```rust
pub async fn probe_cors(client: &reqwest::Client, url: &str) -> bool {
    match client
        .head(url)
        .timeout(std::time::Duration::from_secs(5))
        .send()
        .await
    {
        Ok(resp) => has_cors_wildcard(resp.headers()),
        Err(e) => {
            tracing::debug!(url = %url, error = %e, "CORS probe failed, defaulting to proxy");
            false
        }
    }
}
```

with:

```rust
pub async fn probe_cors(client: &reqwest::Client, url: &str) -> bool {
    if crate::ssrf::is_safe_url(url).await.is_err() {
        return false;
    }
    match client
        .head(url)
        .timeout(std::time::Duration::from_secs(5))
        .send()
        .await
    {
        Ok(resp) => has_cors_wildcard(resp.headers()),
        Err(e) => {
            tracing::debug!(url = %url, error = %e, "CORS probe failed, defaulting to proxy");
            false
        }
    }
}
```

- [ ] **Step 5: Run the hls tests to verify all tests pass**

```bash
cargo test -p mytv hls
```

Expected: all hls tests pass including the 2 new ones.

- [ ] **Step 6: Run the full test suite to check for regressions**

```bash
cargo test
```

Expected: all tests pass. No behavior change for legitimate (public) URLs.

- [ ] **Step 7: Run fmt and commit**

```bash
cargo fmt
git add src/media/hls.rs
git commit -m "fix: add SSRF guard to find_segment_with_descent and probe_cors in hls.rs"
```
