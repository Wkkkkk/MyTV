# Test Coverage & Minor Fixes Implementation Plan

> **For agentic workers:** REQUIRED SUB-SKILL: Use superpowers:subagent-driven-development (recommended) or superpowers:executing-plans to implement this plan task-by-task. Steps use checkbox (`- [ ]`) syntax for tracking.

**Goal:** Add the missing `GET /admin/channels/:id` integration test, HTML-escape the YouTube search error response, and replace the misleading post-loop fallthrough in `current_position` with `unreachable!`.

**Architecture:** Three independent tasks, each touching one file, each committed separately. No behaviour change for tasks 2 and 3; task 1 adds one test exercising the `apply_budget` path that was previously untested at the HTTP level.

**Tech Stack:** Rust 1.96, Axum 0.7, `tower::ServiceExt::oneshot` integration test pattern.

---

### Task 1: Integration test for `GET /admin/channels/:id`

**Files:**
- Modify: `tests/http.rs`

The `channel_detail` handler (`src/routes/admin/channels.rs`) calls `apply_budget` on both source rows and playlist item rows — the core behaviour introduced by the VOD budget badge feature. This path has never been tested at the HTTP level. The test uses seed channel 1 ("Live OK"), which has one active HLS source; `apply_budget` will assign a budget badge class to that source row.

Existing test helpers available in `tests/http.rs`:
- `authed(uri: &str) -> Request<Body>` — builds a GET request with `Authorization: Basic dXNlcjp0ZXN0` (user:test)
- `body_text(response) -> String` — reads the response body as a `String`
- `app() -> axum::Router` — builds the test app with in-memory DB seeded from `tests/fixtures/seed.sql`

Budget CSS classes emitted by `apply_budget` (from `src/budget.rs`): `"budget-direct"`, `"budget-proxied"`, `"budget-unknown"`.

- [ ] **Step 1: Add the test to `tests/http.rs`**

Append this test at the end of the admin section (after `channel_edit_form_returns_200` or any convenient admin test):

```rust
#[tokio::test]
async fn channel_detail_returns_200_with_budget_badge() {
    let response = app()
        .await
        .oneshot(authed("/admin/channels/1"))
        .await
        .unwrap();
    assert_eq!(response.status(), StatusCode::OK);
    let body = body_text(response).await;
    assert!(
        body.contains("budget-direct")
            || body.contains("budget-proxied")
            || body.contains("budget-unknown"),
        "channel detail page must contain a budget badge class from apply_budget"
    );
}
```

- [ ] **Step 2: Run the new test to verify it passes**

```bash
cargo test channel_detail_returns_200_with_budget_badge
```

Expected: PASS. (The handler and `apply_budget` already exist; this test just verifies the path runs end-to-end.)

- [ ] **Step 3: Run the full test suite**

```bash
cargo test
```

Expected: all tests pass.

- [ ] **Step 4: Run fmt and commit**

```bash
cargo fmt
git add tests/http.rs
git commit -m "test: add GET /admin/channels/:id integration test covering apply_budget path"
```

---

### Task 2: HTML-escape YouTube search error

**Files:**
- Modify: `src/routes/admin/discover/mod.rs`

The YouTube API error message is currently interpolated directly into raw HTML via `format!`, allowing a crafted API response to inject HTML/JS into the admin UI. The fix adds a tiny private `html_escape` helper and uses it at the one call site.

The current code (around lines 260–265 of `src/routes/admin/discover/mod.rs`):

```rust
            Err(e) => {
                tracing::error!("YouTube API error: {e}");
                return Html(format!(
                    "<p class=\"empty-state\" style=\"color:#f77\">YouTube search failed: {}.</p>",
                    e
                ));
            }
```

- [ ] **Step 1: Add `html_escape` and fix the format call**

In `src/routes/admin/discover/mod.rs`, add the following private function. Place it just before the handler that uses it (grep for `fn discover_youtube` or `fn youtube_search` to find the right location, then add it immediately above):

```rust
fn html_escape(s: &str) -> String {
    s.replace('&', "&amp;")
        .replace('<', "&lt;")
        .replace('>', "&gt;")
        .replace('"', "&quot;")
}
```

Then change the error format call from:

```rust
                return Html(format!(
                    "<p class=\"empty-state\" style=\"color:#f77\">YouTube search failed: {}.</p>",
                    e
                ));
```

to:

```rust
                return Html(format!(
                    "<p class=\"empty-state\" style=\"color:#f77\">YouTube search failed: {}.</p>",
                    html_escape(&e.to_string())
                ));
```

- [ ] **Step 2: Run the test suite**

```bash
cargo test
```

Expected: all tests pass. No behaviour change for legitimate error messages.

- [ ] **Step 3: Run clippy to confirm no warnings**

```bash
cargo clippy -- -D warnings
```

Expected: clean.

- [ ] **Step 4: Run fmt and commit**

```bash
cargo fmt
git add src/routes/admin/discover/mod.rs
git commit -m "fix: HTML-escape YouTube API error message in discover search response"
```

---

### Task 3: Replace unreachable fallthrough in `current_position`

**Files:**
- Modify: `src/model/playlist_item.rs`

`current_position` (lines 95–123) has a post-loop fallthrough that returns `(last_index, last_item.duration_secs)`. This path is unreachable: `rem_euclid(total)` guarantees `elapsed ∈ [0, total)`, and the for-loop accumulates exactly `total`, so a match is always found before the loop ends. The existing fallthrough misleads readers into thinking `elapsed >= total` is a valid case.

- [ ] **Step 1: Replace lines 116–122 in `src/model/playlist_item.rs`**

Replace:

```rust
    Some((
        items.len() - 1,
        items
            .last()
            .expect("non-empty: checked by is_empty guard above")
            .duration_secs,
    ))
```

with:

```rust
    unreachable!(
        "elapsed ({elapsed}) < total ({total}) guaranteed by rem_euclid, \
         but for-loop found no matching item"
    )
```

- [ ] **Step 2: Run the test suite**

```bash
cargo test
```

Expected: all tests pass. The `unreachable!` path is never hit by any test (or in production).

- [ ] **Step 3: Run fmt and commit**

```bash
cargo fmt
git add src/model/playlist_item.rs
git commit -m "refactor: replace unreachable fallthrough in current_position with unreachable!()"
```
