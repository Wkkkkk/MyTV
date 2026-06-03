# SSRF Hardening Round 2 — Design Stub

> **Status:** Stub — needs brainstorming before implementation.

## Problem

The initial SSRF hardening (`src/ssrf.rs`) left several bypass vectors open, all reachable from the unauthenticated `/stream-proxy` endpoint.

## Findings to address

### `is_blocked` gaps (HIGH)

1. **IPv4-mapped IPv6** — `http://[::ffff:127.0.0.1]/` bypasses all checks. `Ipv6Addr::is_loopback()` returns `false` for mapped addresses; the fc00::/7 / fe80::/10 segment checks don't cover `::ffff:0:0/96`.
2. **`0.0.0.0`** — `is_loopback()`, `is_private()`, and `is_link_local()` all return `false` for the unspecified address, yet `connect(0.0.0.0)` reaches loopback on Linux.
3. **`100.64.0.0/10` CGNAT** — Fly.io uses this range for its internal Wireguard mesh. `Ipv4Addr::is_private()` does not cover it.

### Unguarded fetches in `hls.rs` (HIGH / MEDIUM)

4. **`find_segment_with_descent`** (`src/media/hls.rs:146-156`) — calls `fetch_text` on a variant playlist URL extracted from attacker-supplied HLS content, with no SSRF check. Reachable via `resolve_direct_segments` → `/stream-proxy`.
5. **`probe_cors`** (`src/media/hls.rs:195-208`) — issues an unauthenticated HEAD request to a segment URL extracted from attacker-supplied content, also without SSRF validation.

### Cache key weakness (MEDIUM)

6. **Hostname-only cache key** (`src/ssrf.rs:75-96`) — `SsrfCache` is keyed by hostname with no port, so a DNS-rebinding attack within the 60-second TTL window can bypass the check for a previously-safe hostname.

## Questions to answer before designing

- Should `find_segment_with_descent` / `probe_cors` get full SSRF checks, or should CORS probing be restricted to a pre-approved host allowlist?
- Is port included in the cache key worth the added complexity, or is narrowing the TTL a simpler mitigation for the rebinding window?
- Should `is_blocked` be ported to use a well-tested crate (e.g., `ipnetwork`) or stay hand-rolled for minimal dependencies?

## Rough approach options

A. Extend `is_blocked` + add SSRF guards at the `hls.rs` fetch sites.
B. Add SSRF guards at the `hls.rs` sites only; leave `is_blocked` for a separate hardening pass.
C. Restrict CORS probing to hosts already present in `cors_cache` (only probe URLs the admin has registered), eliminating the attack surface entirely.
