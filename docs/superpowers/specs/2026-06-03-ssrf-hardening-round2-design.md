# SSRF Hardening Round 2 — Design

## Problem

The initial SSRF hardening (`src/ssrf.rs`) left three bypass vectors in `is_blocked` and two unguarded HTTP fetches in `src/media/hls.rs`, all reachable from the unauthenticated `/stream-proxy` endpoint.

## Scope

Fix findings 1–5 from the round-2 audit. Finding 6 (hostname-only cache key / DNS-rebinding window) is deferred.

## Findings addressed

### `is_blocked` gaps (findings 1–3)

| # | Bypass | Root cause |
|---|--------|------------|
| 1 | `0.0.0.0` | `is_loopback()`, `is_private()`, `is_link_local()` all return false for the unspecified address; `connect(0.0.0.0)` reaches loopback on Linux |
| 2 | `100.64.0.0/10` CGNAT | `Ipv4Addr::is_private()` does not cover this range; Fly.io uses it for its internal Wireguard mesh |
| 3 | `::ffff:127.0.0.1` IPv4-mapped IPv6 | `Ipv6Addr::is_loopback()` returns false; the fc00::/7 and fe80::/10 segment checks don't cover `::ffff:0:0/96` |

### Unguarded fetches in `hls.rs` (findings 4–5)

| # | Site | Attack path |
|---|------|-------------|
| 4 | `find_segment_with_descent` line 155 — `fetch_text(client, &variant)` | `/stream-proxy` → `resolve_direct_segments` → `probe_source_cors` → `find_segment_with_descent`. Variant URL is extracted from attacker-supplied HLS content; no SSRF check before the GET. |
| 5 | `probe_cors` line 196 — HEAD request | Same path as above, one step further: segment URL from attacker-supplied content passed to `probe_cors` without validation. |

## Design

### Task 1 — Extend `is_blocked` in `src/ssrf.rs` (TDD)

Three additions; no other changes to `ssrf.rs`:

```rust
IpAddr::V4(v4) => {
    v4.is_loopback()
        || v4.is_private()
        || v4.is_link_local()
        || v4.is_unspecified()                                    // 0.0.0.0
        || (v4.octets()[0] == 100                                 // 100.64.0.0/10 CGNAT
            && v4.octets()[1] >= 64
            && v4.octets()[1] <= 127)
}
IpAddr::V6(v6) => {
    if let Some(v4) = v6.to_ipv4_mapped() {                      // ::ffff:x.x.x.x
        return is_blocked(IpAddr::V4(v4));
    }
    if v6.is_loopback() { return true; }
    let first = v6.segments()[0];
    (first & 0xfe00) == 0xfc00 || (first & 0xffc0) == 0xfe80
}
```

New unit tests (added to the existing `#[cfg(test)]` block, TDD order):

| Test | Input | Asserts |
|------|-------|---------|
| `blocks_unspecified_ipv4` | `http://0.0.0.0/` | `Err(BlockedAddress)` |
| `blocks_cgnat` | `http://100.64.0.1/` | `Err(BlockedAddress)` |
| `blocks_cgnat_upper_bound` | `http://100.127.255.255/` | `Err(BlockedAddress)` |
| `allows_below_cgnat` | `http://100.63.255.255/` | `Ok(())` |
| `allows_above_cgnat` | `http://100.128.0.1/` | `Ok(())` |
| `blocks_ipv4_mapped_loopback` | `http://[::ffff:127.0.0.1]/` | `Err(BlockedAddress)` |
| `blocks_ipv4_mapped_private` | `http://[::ffff:10.0.0.1]/` | `Err(BlockedAddress)` |

All use IP literals — no DNS lookups, no network required.

### Task 2 — Inline SSRF guards in `src/media/hls.rs` (TDD)

No signature changes. Two sites:

**`find_segment_with_descent`** — before `fetch_text(client, &variant)`:

```rust
if crate::ssrf::is_safe_url(&variant).await.is_err() {
    return None;
}
let body = fetch_text(client, &variant).await?;
```

**`probe_cors`** — before the HEAD request:

```rust
if crate::ssrf::is_safe_url(url).await.is_err() {
    return false; // proxy is the safe default
}
```

New unit tests (tokio::test, IP literals only — no network):

| Test | Setup | Asserts |
|------|-------|---------|
| `find_segment_with_descent_blocks_variant_to_loopback` | Master playlist with `http://127.0.0.1/variant.m3u8` as the variant line | Returns `None` |
| `probe_cors_blocks_loopback` | Call `probe_cors(client, "http://127.0.0.1/seg.ts")` | Returns `false` |

## Files changed

| File | Change |
|------|--------|
| `src/ssrf.rs` | Extend `is_blocked` (3 additions) + 7 new unit tests |
| `src/media/hls.rs` | 2 inline SSRF guards + 2 new unit tests |

No other files change. No signature changes, no new dependencies.

## Out of scope

Finding 6 (hostname-only `SsrfCache` key / DNS-rebinding window) — deferred.
