# Test Coverage & Minor Fixes — Design Stub

> **Status:** Stub — needs brainstorming before implementation.

## Problem

A handful of low-severity findings from the PR bug review that don't fit the other plans: a missing test, an admin-panel XSS, and two theoretical correctness issues.

## Findings to address

### Missing test coverage (LOW)

**`src/routes/admin/channels.rs:230`** — `GET /admin/channels/:id` is the only admin handler that calls `apply_budget` for both sources and playlist items, but there is no integration test for it. The budget-badge population path (the main new behaviour from the VOD budget badge feature) is completely untested at the HTTP level.

### Admin-panel XSS (LOW)

**`src/routes/admin/discover/mod.rs:257-260`** — The YouTube API error `message` field is interpolated directly into an HTML response via `format!` without HTML-escaping:

```rust
return Html(format!(
    "<p ...>YouTube search failed: {}.</p>",
    e
));
```

This is behind `BasicAuth`, so it requires admin credentials to trigger, but a crafted YouTube API response (or a MITM on the API call) could inject HTML/JS into the admin UI.

### Theoretical offset bug (MEDIUM, unreachable)

**`src/model/playlist_item.rs:116-122`** — The fallthrough path after the for-loop (reached only if `rem_euclid` somehow returns `>= total`) returns `items.last().duration_secs` (the full duration of the last item) rather than the position within it. The `rem_euclid` invariant makes this unreachable in practice, but the code is misleading.

### Inline DB queries in guide handler (LOW)

**`src/routes/guide/data.rs:74-84`** — Two `sqlx::query_scalar` calls that fetch distinct `channel_id` sets from `sources` are inlined in the guide handler. They belong in `model/source.rs` alongside `list_active_for_channel`.

## Questions to answer before designing

- For the XSS: use `askama::filters::escape` (already a dep) or the `html-escape` crate?
- For the missing test: should `GET /admin/channels/:id` be tested for badge class presence in the HTML body, or just for HTTP 200 + correct structure?
- Is the offset fallthrough worth fixing (rename `expect` message, add a comment) or worth a defensive assertion that panics loudly in debug?
