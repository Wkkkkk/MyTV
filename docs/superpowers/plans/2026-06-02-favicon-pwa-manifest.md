# Favicon & PWA Manifest Implementation Plan

> **For agentic workers:** REQUIRED SUB-SKILL: Use superpowers:subagent-driven-development (recommended) or superpowers:executing-plans to implement this plan task-by-task. Steps use checkbox (`- [ ]`) syntax for tracking.

**Goal:** Add a TV+Play SVG favicon and a PWA web app manifest so the browser tab shows a recognizable icon and Chrome/Edge offer an "Install" prompt.

**Architecture:** Three new Axum route handlers serve `/favicon.svg`, `/manifest.json`, and `/favicon.ico` (redirect). The SVG is embedded at compile time via `include_str!`. Both base templates get the three required `<head>` tags. No new dependencies.

**Tech Stack:** Rust/Axum 0.7, `include_str!` for compile-time embedding, Askama templates.

---

### Task 1: Create the SVG icon

**Files:**
- Create: `static/favicon.svg`

- [ ] **Step 1: Create `static/favicon.svg`** (the `static/` directory is new — create it)

```bash
mkdir static
```

```svg
<svg xmlns="http://www.w3.org/2000/svg" viewBox="0 0 32 32">
  <rect x="2" y="6" width="28" height="18" rx="2" fill="none" stroke="#e94560" stroke-width="2.5"/>
  <rect x="10" y="25" width="12" height="2" rx="1" fill="#e94560"/>
  <rect x="13" y="24" width="6" height="1.5" fill="#e94560"/>
  <rect x="5" y="9" width="22" height="12" rx="1" fill="#e94560" opacity="0.15"/>
  <polygon points="13,11 13,19 21,15" fill="#e94560"/>
</svg>
```

- [ ] **Step 2: Commit**

```bash
git add static/favicon.svg
git commit -m "feat: add TV+Play SVG favicon"
```

---

### Task 2: Route handlers (TDD)

**Files:**
- Create: `src/routes/static_files.rs`
- Modify: `src/routes/mod.rs` — add `pub mod static_files;`
- Modify: `src/lib.rs` — add three `.route()` calls
- Modify: `tests/http.rs` — add three integration tests

- [ ] **Step 1: Write the failing tests in `tests/http.rs`**

Add after the existing `fn req()` helper (before any `#[tokio::test]` functions, or anywhere in the file):

```rust
#[tokio::test]
async fn test_favicon_svg() {
    let app = app().await;
    let response = app.oneshot(req("/favicon.svg")).await.unwrap();
    assert_eq!(response.status(), StatusCode::OK);
    assert_eq!(
        response.headers().get("content-type").unwrap(),
        "image/svg+xml",
    );
}

#[tokio::test]
async fn test_manifest_json() {
    let app = app().await;
    let response = app.oneshot(req("/manifest.json")).await.unwrap();
    assert_eq!(response.status(), StatusCode::OK);
    assert_eq!(
        response.headers().get("content-type").unwrap(),
        "application/manifest+json",
    );
}

#[tokio::test]
async fn test_favicon_ico_redirect() {
    let app = app().await;
    let response = app.oneshot(req("/favicon.ico")).await.unwrap();
    assert_eq!(response.status(), StatusCode::PERMANENT_REDIRECT);
    assert_eq!(
        response.headers().get("location").unwrap(),
        "/favicon.svg",
    );
}
```

- [ ] **Step 2: Run tests to verify they fail**

```bash
cargo test test_favicon_svg test_manifest_json test_favicon_ico_redirect 2>&1 | tail -10
```

Expected: compilation error — `static_files` module not found.

- [ ] **Step 3: Create `src/routes/static_files.rs`**

```rust
use axum::{
    http::header,
    response::{IntoResponse, Redirect, Response},
};

const SVG: &str = include_str!("../../static/favicon.svg");

const MANIFEST: &str = r#"{"name":"MyTV","short_name":"MyTV","start_url":"/guide","display":"standalone","background_color":"#0f0f0f","theme_color":"#e94560","icons":[{"src":"/favicon.svg","sizes":"any","type":"image/svg+xml"}]}"#;

pub async fn favicon_svg() -> Response {
    ([(header::CONTENT_TYPE, "image/svg+xml")], SVG).into_response()
}

pub async fn manifest_json() -> Response {
    ([(header::CONTENT_TYPE, "application/manifest+json")], MANIFEST).into_response()
}

pub async fn favicon_ico() -> Redirect {
    Redirect::permanent("/favicon.svg")
}
```

- [ ] **Step 4: Add module to `src/routes/mod.rs`**

Add `pub mod static_files;` at the top of the file, keeping the existing alphabetical order:

```rust
pub mod admin;
pub mod guide;
pub mod health;
pub mod player;
pub mod static_files;
```

- [ ] **Step 5: Wire routes in `src/lib.rs`**

In the `build_router` function, add three routes to the main `Router::new()` chain (after the existing `/stream-proxy` route, before `.nest("/admin", admin_router)`):

```rust
.route("/favicon.svg", get(routes::static_files::favicon_svg))
.route("/manifest.json", get(routes::static_files::manifest_json))
.route("/favicon.ico", get(routes::static_files::favicon_ico))
```

- [ ] **Step 6: Run tests to verify they pass**

```bash
cargo test test_favicon_svg test_manifest_json test_favicon_ico_redirect 2>&1 | tail -10
```

Expected: all three tests `ok`.

- [ ] **Step 7: Run full test suite, fmt, and clippy**

```bash
cargo fmt && cargo clippy -- -D warnings && cargo test 2>&1 | tail -5
```

Expected: `test result: ok. N passed; 0 failed`.

- [ ] **Step 8: Commit**

```bash
git add src/routes/static_files.rs src/routes/mod.rs src/lib.rs tests/http.rs
git commit -m "feat: serve favicon.svg, manifest.json, and favicon.ico routes"
```

---

### Task 3: Add tags to both base templates

**Files:**
- Modify: `templates/base.html`
- Modify: `templates/admin/base.html`

- [ ] **Step 1: Add three tags to `templates/base.html`**

In the `<head>` section, after `<meta name="viewport" ...>` and before `<title>`, insert:

```html
  <link rel="icon" type="image/svg+xml" href="/favicon.svg">
  <link rel="manifest" href="/manifest.json">
  <meta name="theme-color" content="#e94560">
```

- [ ] **Step 2: Add the same three tags to `templates/admin/base.html`**

Same position — after `<meta name="viewport" ...>` and before `<title>`:

```html
  <link rel="icon" type="image/svg+xml" href="/favicon.svg">
  <link rel="manifest" href="/manifest.json">
  <meta name="theme-color" content="#e94560">
```

- [ ] **Step 3: Run full test suite**

```bash
cargo test 2>&1 | tail -5
```

Expected: all tests pass (template changes don't break anything, Askama validates at compile time).

- [ ] **Step 4: Commit**

```bash
git add templates/base.html templates/admin/base.html
git commit -m "feat: link favicon and PWA manifest in HTML templates"
```

---

### Task 4: Update IDEAS.md

**Files:**
- Modify: `docs/IDEAS.md`

- [ ] **Step 1: Mark the idea as done in `docs/IDEAS.md`**

Find the existing line:

```
9. **Favicon and PWA manifest** — add a favicon ...
```

Replace it with:

```
9. ~~**Favicon and PWA manifest**~~ — done: TV+Play SVG favicon at `/favicon.svg`, PWA manifest at `/manifest.json`, both linked from all pages via `<head>` tags.
```

- [ ] **Step 2: Commit**

```bash
git add docs/IDEAS.md
git commit -m "docs: mark favicon and PWA manifest as done"
```
