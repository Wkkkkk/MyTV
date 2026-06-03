# Admin Mutation Route Integration Tests (U13) Implementation Plan

> **For agentic workers:** REQUIRED SUB-SKILL: Use superpowers:subagent-driven-development (recommended) or superpowers:executing-plans to implement this plan task-by-task. Steps use checkbox (`- [ ]`) syntax for tracking.

**Goal:** Add 17 HTTP-level integration tests covering the admin mutation routes that currently have no test coverage: channel CRUD, source create/delete/toggle, and the discover/edit-form GET routes.

**Architecture:** All tests live in `tests/http.rs`. A new `authed_form_post` helper handles form-encoded POST bodies. Tests use the existing `oneshot` pattern, seed data (channel 1 = "Live OK", source 1 = active), and the same `Basic dXNlcjp0ZXN0` auth header used throughout the file.

**Tech Stack:** Rust 1.96, Axum 0.7, `tower::ServiceExt::oneshot`, `http_body_util`

---

### Task 1: Add `authed_form_post` helper and channel create tests

**Files:**
- Modify: `tests/http.rs`

- [ ] **Step 1: Add `authed_form_post` helper**

In `tests/http.rs`, find the `authed_post` function (around line 64) and add the following immediately after it:

```rust
fn authed_form_post(uri: &str, body: &str) -> Request<Body> {
    Request::builder()
        .method("POST")
        .uri(uri)
        .header("Authorization", "Basic dXNlcjp0ZXN0")
        .header("content-type", "application/x-www-form-urlencoded")
        .body(Body::from(body.to_string()))
        .unwrap()
}
```

- [ ] **Step 2: Add a `// ── Admin mutations ──` section header and channel create tests**

At the end of `tests/http.rs`, add:

```rust
// ── Admin mutations ──────────────────────────────────────────────────────────

// Channel create

#[tokio::test]
async fn channel_create_redirects_on_success() {
    let response = app()
        .await
        .oneshot(authed_form_post(
            "/admin/channels",
            "name=Test+Channel&category=test&channel_type=live&sort_order=0&logo_url=&loop_anchor=",
        ))
        .await
        .unwrap();
    assert_eq!(response.status(), StatusCode::SEE_OTHER);
    assert_eq!(
        response.headers().get("location").unwrap(),
        "/admin/channels"
    );
}

#[tokio::test]
async fn channel_create_rejects_invalid_type() {
    let response = app()
        .await
        .oneshot(authed_form_post(
            "/admin/channels",
            "name=Test&category=test&channel_type=invalid&sort_order=0&logo_url=&loop_anchor=",
        ))
        .await
        .unwrap();
    assert_eq!(response.status(), StatusCode::UNPROCESSABLE_ENTITY);
}

#[tokio::test]
async fn channel_create_requires_auth() {
    let response = app()
        .await
        .oneshot(
            Request::builder()
                .method("POST")
                .uri("/admin/channels")
                .header("content-type", "application/x-www-form-urlencoded")
                .body(Body::from(
                    "name=Test&category=test&channel_type=live&sort_order=0&logo_url=&loop_anchor=",
                ))
                .unwrap(),
        )
        .await
        .unwrap();
    assert_eq!(response.status(), StatusCode::UNAUTHORIZED);
}
```

- [ ] **Step 3: Run the new tests**

```bash
cd /Users/kunwu/Workspace/playground/MyTV && cargo test channel_create
```

Expected: 3 tests pass (`channel_create_redirects_on_success`, `channel_create_rejects_invalid_type`, `channel_create_requires_auth`).

- [ ] **Step 4: Commit**

```bash
cd /Users/kunwu/Workspace/playground/MyTV && cargo fmt && git add tests/http.rs && git commit -m "test: add integration tests for channel create route"
```

---

### Task 2: Add channel update and delete tests

**Files:**
- Modify: `tests/http.rs`

- [ ] **Step 1: Append channel update tests**

At the end of `tests/http.rs`, add:

```rust
// Channel update

#[tokio::test]
async fn channel_update_redirects_on_success() {
    // Channel 1 ("Live OK") exists in seed data
    let response = app()
        .await
        .oneshot(authed_form_post(
            "/admin/channels/1",
            "name=Updated+Channel&category=test&channel_type=live&sort_order=1&logo_url=&loop_anchor=",
        ))
        .await
        .unwrap();
    assert_eq!(response.status(), StatusCode::SEE_OTHER);
}

#[tokio::test]
async fn channel_update_rejects_invalid_type() {
    let response = app()
        .await
        .oneshot(authed_form_post(
            "/admin/channels/1",
            "name=Updated+Channel&category=test&channel_type=invalid&sort_order=1&logo_url=&loop_anchor=",
        ))
        .await
        .unwrap();
    assert_eq!(response.status(), StatusCode::UNPROCESSABLE_ENTITY);
}

#[tokio::test]
async fn channel_update_returns_404_for_missing_channel() {
    let response = app()
        .await
        .oneshot(authed_form_post(
            "/admin/channels/9999",
            "name=Ghost&category=test&channel_type=live&sort_order=0&logo_url=&loop_anchor=",
        ))
        .await
        .unwrap();
    assert_eq!(response.status(), StatusCode::NOT_FOUND);
}
```

- [ ] **Step 2: Append channel delete tests**

```rust
// Channel delete

#[tokio::test]
async fn channel_delete_redirects_on_success() {
    // Channel 1 exists in seed data
    let response = app()
        .await
        .oneshot(authed_post("/admin/channels/1/delete"))
        .await
        .unwrap();
    assert_eq!(response.status(), StatusCode::SEE_OTHER);
}

#[tokio::test]
async fn channel_delete_returns_404_for_missing_channel() {
    let response = app()
        .await
        .oneshot(authed_post("/admin/channels/9999/delete"))
        .await
        .unwrap();
    assert_eq!(response.status(), StatusCode::NOT_FOUND);
}
```

- [ ] **Step 3: Run the new tests**

```bash
cd /Users/kunwu/Workspace/playground/MyTV && cargo test channel_update && cargo test channel_delete
```

Expected: 5 tests pass (`channel_update_redirects_on_success`, `channel_update_rejects_invalid_type`, `channel_update_returns_404_for_missing_channel`, `channel_delete_redirects_on_success`, `channel_delete_returns_404_for_missing_channel`).

- [ ] **Step 4: Commit**

```bash
cd /Users/kunwu/Workspace/playground/MyTV && cargo fmt && git add tests/http.rs && git commit -m "test: add integration tests for channel update/delete routes"
```

---

### Task 3: Add source create, delete, and toggle tests

**Files:**
- Modify: `tests/http.rs`

- [ ] **Step 1: Append source create tests**

At the end of `tests/http.rs`, add:

```rust
// Source create

#[tokio::test]
async fn source_create_redirects_on_success() {
    // Channel 1 exists in seed data
    let response = app()
        .await
        .oneshot(authed_form_post(
            "/admin/channels/1/sources",
            "kind=hls&url=https%3A%2F%2Fexample.com%2Ftest.m3u8&priority=5",
        ))
        .await
        .unwrap();
    assert_eq!(response.status(), StatusCode::SEE_OTHER);
    assert_eq!(
        response.headers().get("location").unwrap(),
        "/admin/channels/1"
    );
}

#[tokio::test]
async fn source_create_rejects_invalid_kind() {
    let response = app()
        .await
        .oneshot(authed_form_post(
            "/admin/channels/1/sources",
            "kind=rtmp&url=https%3A%2F%2Fexample.com%2Fstream&priority=1",
        ))
        .await
        .unwrap();
    assert_eq!(response.status(), StatusCode::UNPROCESSABLE_ENTITY);
}

#[tokio::test]
async fn source_create_rejects_empty_url() {
    let response = app()
        .await
        .oneshot(authed_form_post(
            "/admin/channels/1/sources",
            "kind=hls&url=&priority=1",
        ))
        .await
        .unwrap();
    assert_eq!(response.status(), StatusCode::UNPROCESSABLE_ENTITY);
}
```

- [ ] **Step 2: Append source delete and toggle tests**

```rust
// Source delete

#[tokio::test]
async fn source_delete_redirects_on_success() {
    // Source 1 (channel 1, "https://live.example.com/live.m3u8") exists in seed
    let response = app()
        .await
        .oneshot(authed_post("/admin/sources/1/delete"))
        .await
        .unwrap();
    assert_eq!(response.status(), StatusCode::SEE_OTHER);
}

#[tokio::test]
async fn source_delete_returns_404_for_missing_source() {
    let response = app()
        .await
        .oneshot(authed_post("/admin/sources/9999/delete"))
        .await
        .unwrap();
    assert_eq!(response.status(), StatusCode::NOT_FOUND);
}

// Source toggle

#[tokio::test]
async fn source_toggle_redirects_on_success() {
    // Source 1 exists in seed
    let response = app()
        .await
        .oneshot(authed_post("/admin/sources/1/toggle"))
        .await
        .unwrap();
    assert_eq!(response.status(), StatusCode::SEE_OTHER);
}

#[tokio::test]
async fn source_toggle_returns_404_for_missing_source() {
    let response = app()
        .await
        .oneshot(authed_post("/admin/sources/9999/toggle"))
        .await
        .unwrap();
    assert_eq!(response.status(), StatusCode::NOT_FOUND);
}
```

- [ ] **Step 3: Run the new tests**

```bash
cd /Users/kunwu/Workspace/playground/MyTV && cargo test source_create && cargo test source_delete && cargo test source_toggle
```

Expected: 7 tests pass.

- [ ] **Step 4: Commit**

```bash
cd /Users/kunwu/Workspace/playground/MyTV && cargo fmt && git add tests/http.rs && git commit -m "test: add integration tests for source create/delete/toggle routes"
```

---

### Task 4: Add channel edit form and discover page tests, run full suite

**Files:**
- Modify: `tests/http.rs`

- [ ] **Step 1: Append channel edit form and discover page tests**

At the end of `tests/http.rs`, add:

```rust
// Channel edit form

#[tokio::test]
async fn channel_edit_form_returns_200() {
    // Channel 1 exists in seed
    let response = app()
        .await
        .oneshot(authed("/admin/channels/1/edit"))
        .await
        .unwrap();
    assert_eq!(response.status(), StatusCode::OK);
}

#[tokio::test]
async fn channel_edit_form_returns_404_for_missing_channel() {
    let response = app()
        .await
        .oneshot(authed("/admin/channels/9999/edit"))
        .await
        .unwrap();
    assert_eq!(response.status(), StatusCode::NOT_FOUND);
}

// Discover page

#[tokio::test]
async fn admin_discover_page_returns_200() {
    let response = app()
        .await
        .oneshot(authed("/admin/discover"))
        .await
        .unwrap();
    assert_eq!(response.status(), StatusCode::OK);
}

#[tokio::test]
async fn admin_discover_page_requires_auth() {
    let response = app()
        .await
        .oneshot(req("/admin/discover"))
        .await
        .unwrap();
    assert_eq!(response.status(), StatusCode::UNAUTHORIZED);
}
```

- [ ] **Step 2: Run the full test suite**

```bash
cd /Users/kunwu/Workspace/playground/MyTV && cargo test
```

Expected: all tests pass. The 17 new tests bring integration test count from 28 to 45.

- [ ] **Step 3: Commit**

```bash
cd /Users/kunwu/Workspace/playground/MyTV && cargo fmt && git add tests/http.rs && git commit -m "test: add integration tests for channel_edit_form and discover page"
```
