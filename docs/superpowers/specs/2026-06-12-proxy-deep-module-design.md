# Spec — Collapse the stream-proxy fetch loop into a deep module

_Candidate #4 of the architecture-deepening effort (`docs/architecture/changes-20260612.md` §4).
Created 2026-06-12._

## Problem

`stream_proxy` (in `src/routes/player.rs`, ~150 lines) is one opaque HTTP handler that
interleaves six unrelated concerns:

1. **Scheme validation** — reject non-`http(s)` URLs.
2. **A 5-attempt redirect loop** — each hop re-runs the SSRF check (TOCTOU-aware), forwards
   the client `Range` header, and resolves relative `Location` headers.
3. **Response-header building** — sets `Access-Control-Allow-Origin: *` (we own CORS) and
   strips RFC 7230 §6.1 hop-by-hop headers plus every header named in `Connection`.
4. **Playlist detection** — content-type / extension sniffing.
5. **Playlist buffer-and-rewrite** — collect the body under a 20 MB cap, then rewrite HLS or
   DASH manifest URLs.
6. **Metered streaming** — for non-playlist bodies, stream through an `ActiveStreamGuard` and
   count bytes.

HTTP plumbing and proxy business logic are tangled in the request handler. Adding a manifest
format, or touching the redirect/SSRF/header rules, means editing the handler in place. The
pure helpers (`resolve_location`, `detect_playlist`) are already extracted and unit-tested,
but the header-stripping rules — a genuine RFC concern — are inline and reachable only through
an integration test that needs a live server.

## Solution

Extract a top-level `src/proxy.rs` module that owns the whole proxy operation behind a single
entry point. The handler shrinks to a one-line delegation; redirect-follow, SSRF, header
building, detection, and rewrite become internal seams of the module. The genuinely pure rules
(header building) gain a direct unit-test surface.

### Module location

`src/proxy.rs`, declared `pub mod proxy;` in `src/lib.rs` (sibling of `broadcast`, `health`,
`budget`). It orchestrates `ssrf` + `media::{hls, mpd}` + `metrics` + the proxy HTTP client — a
lifecycle/transport concern, not a media primitive — so a top-level home matches the review's
`proxy::fetch_rewritten` naming and the candidate-#3 precedent (`broadcast.rs`).

### Public surface

One entry point, returning a fully-built `Response` (matches the changes-doc verbatim):

```rust
pub async fn fetch_rewritten(
    state: &AppState,
    url: String,
    request_headers: &HeaderMap,
) -> Response
```

It owns the entire current `stream_proxy` body, scheme validation included. Everything that is
HTTP-plumbing or proxy business logic — the redirect loop, header build, playlist buffering +
rewrite, and the metered streaming with `ActiveStreamGuard` — lives inside the module. Keeping
the streaming/guard inside is what preserves the locality win.

### Caller change (`src/routes/player.rs`)

```rust
pub async fn stream_proxy(
    State(state): State<AppState>,
    Query(q): Query<StreamProxyQuery>,
    request_headers: HeaderMap,
) -> Response {
    crate::proxy::fetch_rewritten(&state, q.url, &request_headers).await
}
```

`StreamProxyQuery` stays in `player.rs` — it is the route's query extractor, bound next to the
handler. `use` imports in `player.rs` that become unused after the move (e.g. `Bytes`,
`StreamExt`, `mpd`, `Ordering`) are removed; imports still needed by other handlers stay.

### Internal seams (all private to `proxy.rs`)

1. **`resolve_location(location: &str, base_url: &str) -> Option<String>`** — moved verbatim
   from `player.rs`. Pure. Brings its 5 existing unit tests.
2. **`detect_playlist(content_type: &str, url: &str) -> bool`** — moved verbatim. Pure. Brings
   its existing unit tests.
3. **`resolve_direct_segments(state: &AppState, base_url: &str) -> bool`** — moved verbatim
   (already takes `&AppState`).
4. **`build_proxy_headers(upstream: &HeaderMap) -> HeaderMap`** — **new** pure extraction of the
   inline CORS-insert + hop-by-hop/Connection-options stripping (today at player.rs:408–434).
   Inserts `Access-Control-Allow-Origin: *`, then copies every upstream header except: the CORS
   header (we own it), `Connection`, `Transfer-Encoding`, `TE`, `Trailer`, `Upgrade`, and every
   header named in the upstream `Connection` value. New unit tests cover these rules directly —
   the testability win.
5. **`follow_redirects(state: &AppState, url: String, request_headers: &HeaderMap) -> Result<(reqwest::Response, String), StatusCode>`**
   — extracts the 5-attempt loop. Preserves the security-critical per-hop SSRF re-check and the
   distinct error codes (`422 UNPROCESSABLE_ENTITY` on SSRF failure, `502 BAD_GATEWAY` on fetch
   error / missing-or-unresolvable `Location` / exhausting all 5 attempts). On success returns
   the final upstream `reqwest::Response` and the resolved final URL (the URL is needed
   downstream for `detect_playlist`, `resolve_direct_segments`, and the rewrite base).

`fetch_rewritten` then reads top-to-bottom: scheme check → `follow_redirects` → `build_proxy_headers`
→ branch (playlist ⇒ buffer-under-cap + `mpd`/`hls` rewrite; else ⇒ metered streaming). The
20 MB-cap body collection and the `proxy_bytes` metric increments stay inline in `fetch_rewritten`.

## Behavior preservation (byte-identical)

- Scheme check rejects non-`http(s)` with `400 BAD_REQUEST`.
- SSRF checked on **every** hop (not just the first); failure ⇒ `422`.
- `Range` header forwarded on every hop.
- Redirect `Location` resolved relative to the current URL; missing/unresolvable ⇒ `502`.
- After 5 attempts without a non-redirect response ⇒ `502`.
- `ACAO: *` always set; hop-by-hop + `Connection`-named headers stripped.
- Playlist branch: 20 MB cap (over ⇒ `502`), read error ⇒ `502`, `proxy_bytes` incremented by
  body length, DASH (`dash+xml` content-type or `.mpd` URL) ⇒ `mpd::rewrite_mpd_urls` +
  `application/dash+xml`, else `hls::rewrite_hls_urls` + `application/vnd.apple.mpegurl`.
- Non-playlist branch: `ActiveStreamGuard` held for the stream's lifetime, `proxy_bytes`
  incremented per chunk, original upstream status + filtered headers preserved.

## Out of scope

- `stream_proxy` handler shell and `StreamProxyQuery` stay in `player.rs`.
- No change to `media::{hls, mpd}`, `ssrf`, `metrics`, or `health`.
- No change to the other `player.rs` handlers (`tune`, `next`, `next_live`, VOD helpers).

## Testing — the win

- **Moved unit tests** — `resolve_location` (5) and `detect_playlist` tests move into
  `proxy.rs`'s `#[cfg(test)]` module unchanged.
- **New `build_proxy_headers` unit tests** (pure, no network):
  1. Sets `Access-Control-Allow-Origin: *` even when upstream omits it.
  2. Overwrites an upstream `Access-Control-Allow-Origin` with `*` (we own it).
  3. Strips `Connection`, `Transfer-Encoding`, `TE`, `Trailer`, `Upgrade`.
  4. Strips every header named in the upstream `Connection` value (e.g. `Connection: X-Foo`
     ⇒ `X-Foo` dropped).
  5. Preserves an ordinary header (e.g. `Content-Type`).
- **Integration tests unchanged** — all `tests/http.rs` stream-proxy tests stay green and
  untouched: `stream_proxy_blocks_loopback`, `stream_proxy_blocks_link_local_metadata`,
  `stream_proxy_blocks_private_rfc1918`, `stream_proxy_rejects_non_http_scheme`,
  `stream_proxy_strips_hop_by_hop_headers`, `stream_proxy_follows_relative_redirect`,
  `test_stream_proxy_rewrites_dash_bbb_manifest`. They are the behavior contract for the parts
  that need a live server (redirect loop, SSRF, streaming, rewrite).

`follow_redirects`, `resolve_direct_segments`, and the streaming branch are not unit-tested in
isolation (they require a live server); the integration tests above cover them.

## Acceptance criteria

1. New `src/proxy.rs` exposes `fetch_rewritten`; declared `pub mod proxy;` in `src/lib.rs`.
2. `stream_proxy` in `player.rs` is a one-line delegation to `proxy::fetch_rewritten`;
   `resolve_location`, `detect_playlist`, and `resolve_direct_segments` no longer live in
   `player.rs`. Now-unused `player.rs` imports are removed.
3. `build_proxy_headers` and `follow_redirects` are private to `proxy.rs`; the proxy's
   externally observable behavior is byte-identical to today.
4. The moved unit tests plus the new `build_proxy_headers` tests pass; all `tests/http.rs`
   stream-proxy integration tests stay green without modification.
5. `cargo test` (all targets, incl. `--no-run` to compile lib `#[cfg(test)]` modules),
   `cargo fmt --check`, `cargo clippy --all-targets -- -D warnings` all green.
