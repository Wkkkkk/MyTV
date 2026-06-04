# Test Coverage & Minor Fixes — Design

## Scope

Three independent, small fixes. No behaviour change for findings 2 and 3. Finding 1 adds one integration test.

---

## Finding 1: Missing integration test for `GET /admin/channels/:id`

### Problem

`channel_detail` (`src/routes/admin/channels.rs`) is the only admin handler that calls `apply_budget` for both source rows and playlist item rows — the main new behaviour from the VOD budget badge feature. No integration test exercises this endpoint.

### Design

Add one test in `tests/http.rs`:

```rust
#[tokio::test]
async fn channel_detail_returns_200_with_budget_badge() {
    let response = app()
        .await
        .oneshot(authed("/admin/channels/1"))
        .await
        .unwrap();
    assert_eq!(response.status(), StatusCode::OK);
    let body = body_string(response).await;
    // apply_budget must have run: at least one budget-class attribute present
    assert!(
        body.contains("budget-direct") || body.contains("budget-proxy") || body.contains("budget-unknown"),
        "channel detail page must contain a budget badge class"
    );
}
```

Seed channel 1 ("Live OK") has one active HLS source — `apply_budget` will set a badge class on its source row.

---

## Finding 2: Admin-panel XSS in YouTube search error

### Problem

`src/routes/admin/discover/mod.rs:263` — the YouTube API error message is interpolated raw into an HTML response:

```rust
return Html(format!(
    "<p class=\"empty-state\" style=\"color:#f77\">YouTube search failed: {}.</p>",
    e
));
```

A crafted YouTube API response (or MITM on the API call) could inject HTML/JS into the admin UI. Behind `BasicAuth`, but still exploitable by an attacker who controls the YouTube API response.

### Design

Add a private `html_escape` helper immediately before its call site in `discover/mod.rs`:

```rust
fn html_escape(s: &str) -> String {
    s.replace('&', "&amp;")
     .replace('<', "&lt;")
     .replace('>', "&gt;")
     .replace('"', "&quot;")
}
```

Change the format call:

```rust
// Before
return Html(format!(
    "<p class=\"empty-state\" style=\"color:#f77\">YouTube search failed: {}.</p>",
    e
));

// After
return Html(format!(
    "<p class=\"empty-state\" style=\"color:#f77\">YouTube search failed: {}.</p>",
    html_escape(&e.to_string())
));
```

No new dependencies. No behaviour change for legitimate error messages (which contain no HTML special characters).

---

## Finding 3: Misleading fallthrough in `playlist_item::current_position`

### Problem

`src/model/playlist_item.rs` lines 116–122 — the post-loop path after `current_position`'s for-loop returns `(last_index, last_item.duration_secs)`. This looks like a valid fallback, but it is unreachable: `rem_euclid` guarantees `elapsed < total`, and the loop accumulates exactly `total`, so a match is always found. The fallthrough misleads readers into thinking `elapsed >= total` is a handled case.

### Design

Replace lines 116–122 with:

```rust
unreachable!(
    "elapsed ({elapsed}) < total ({total}) guaranteed by rem_euclid, \
     but for-loop found no matching item"
)
```

Panics loudly in debug if the invariant is ever violated; no change to release behaviour since the path is provably unreachable.

---

## Files changed

| File | Change |
|------|--------|
| `tests/http.rs` | Add `channel_detail_returns_200_with_budget_badge` test |
| `src/routes/admin/discover/mod.rs` | Add `html_escape` helper; escape error in format call |
| `src/model/playlist_item.rs` | Replace fallthrough with `unreachable!` |
