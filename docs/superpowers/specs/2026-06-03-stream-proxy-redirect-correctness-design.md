# Stream Proxy Redirect Correctness — Design Stub

> **Status:** Stub — needs brainstorming before implementation.

## Problem

The manual redirect loop in `stream_proxy` (`src/routes/player.rs`) handles relative `Location` headers incorrectly.

## Finding (MEDIUM correctness)

**`src/routes/player.rs:220-229`** — When an upstream CDN returns a redirect with a relative `Location` value (e.g. `/newpath` or `../segment.m3u8`), the raw header string is used as the next URL without resolving it against the current URL. `reqwest::Url::parse` fails on a relative reference, causing `is_safe_url_cached` to return `Err(UnsupportedScheme)`, and `stream_proxy` returns HTTP 422 instead of following the redirect.

Relative redirects are common in HLS CDN responses (e.g. `Location: /live/index.m3u8`).

## Questions to answer before designing

- Should the fix resolve relative URLs against the current `url` (standard HTTP behaviour), or reject relative redirects entirely with a clear error?
- Is there any test infrastructure in place to simulate redirect responses (like the raw TCP server used for hop-by-hop header tests)?

## Rough approach options

A. Resolve relative `Location` against current `url` using `reqwest::Url::join` before passing to `is_safe_url_cached`.
B. Reject relative redirects with `StatusCode::BAD_GATEWAY` and log a warning — simpler, but breaks some real-world CDNs.
