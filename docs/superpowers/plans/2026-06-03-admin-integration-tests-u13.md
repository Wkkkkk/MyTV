# Admin Mutation Route Integration Tests: U13

> **For agentic workers:** REQUIRED SUB-SKILL: Use superpowers:subagent-driven-development (recommended) or superpowers:executing-plans to implement this plan task-by-task. Steps use checkbox (`- [ ]`) syntax for tracking.

**Goal:** Add integration tests covering the admin mutation routes that currently have no HTTP-level test coverage: channel CRUD, source create/delete/toggle, and the discover page GET.

**Architecture:** All new tests go in `tests/http.rs` using the existing `oneshot` pattern. A new helper `authed_form_post(uri, body)` handles form-encoded POST bodies. Seed data (channel 1 = "Live OK", source 1 active) is used where IDs are needed.

**Tech Stack:** Rust 1.96, Axum 0.7, `tower::ServiceExt::oneshot`, `http_body_util`

---

### Task 1: Add `authed_form_post` helper and channel CRUD tests

**Files:**
- Modify: `tests/http.rs`

- [ ] **Step 1: Add `authed_form_post` helper**

After the existing `authed_post` function in `tests/http.rs`, add:

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

- [ ] **Step 2: Add channel_create tests**

Append to `tests/http.rs`:

```rust
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

- [ ] **Step 3: Add channel_update and channel_delete tests**

```rust
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

- [ ] **Step 4: Run tests**

```bash
cd /Users/kunwu/Workspace/playground/MyTV && cargo test channel_create && cargo test channel_update && cargo test channel_delete
```

Expected: all new tests pass.

- [ ] **Step 5: Commit**

```bash
cd /Users/kunwu/Workspace/playground/MyTV && cargo fmt && git add tests/http.rs && git commit -m "test: add integration tests for channel create/update/delete routes"
```

---

### Task 2: Add source create/delete/toggle tests

**Files:**
- Modify: `tests/http.rs`

- [ ] **Step 1: Add source_create tests**

Append to `tests/http.rs`:

```rust
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

- [ ] **Step 2: Add source_delete and source_toggle tests**

```rust
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

- [ ] **Step 3: Run tests**

```bash
cd /Users/kunwu/Workspace/playground/MyTV && cargo test source_create && cargo test source_delete && cargo test source_toggle
```

Expected: all new tests pass.

- [ ] **Step 4: Commit**

```bash
cd /Users/kunwu/Workspace/playground/MyTV && cargo fmt && git add tests/http.rs && git commit -m "test: add integration tests for source create/delete/toggle routes"
```

---

### Task 3: Add channel_edit_form and discover page GET tests

**Files:**
- Modify: `tests/http.rs`

- [ ] **Step 1: Add GET route tests**

Append to `tests/http.rs`:

```rust
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

- [ ] **Step 2: Run all tests**

```bash
cd /Users/kunwu/Workspace/playground/MyTV && cargo test
```

Expected: all tests pass. Final integration test count should be 28 + new tests added across Tasks 1–3.

- [ ] **Step 3: Commit**

```bash
cd /Users/kunwu/Workspace/playground/MyTV && cargo fmt && git add tests/http.rs && git commit -m "test: add integration tests for channel_edit_form and discover page"
```
