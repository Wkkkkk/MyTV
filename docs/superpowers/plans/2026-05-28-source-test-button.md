# Source Test Button Implementation Plan

> **For agentic workers:** REQUIRED SUB-SKILL: Use superpowers:subagent-driven-development (recommended) or superpowers:executing-plans to implement this plan task-by-task. Steps use checkbox (`- [ ]`) syntax for tracking.

**Goal:** Add a "Test" button per source row in the admin channel detail page that checks whether the source is reachable and shows the result inline.

**Architecture:** New `source_test` handler in `src/routes/admin.rs` returns an HTML fragment via HTMX swap. For YouTube/Twitch sources it calls `resolver::resolve_url`; for HLS/IPTV it does an HTTP HEAD request via `state.http_client`. Route registered in `src/main.rs`, button added to `templates/admin/channel_detail.html`.

**Tech Stack:** Rust, Axum 0.7, reqwest 0.12, Askama 0.12, HTMX 1.9

---

## File Map

| File | Change |
|---|---|
| `src/routes/admin.rs` | Add `source_test` handler |
| `src/main.rs` | Register `POST /admin/sources/:id/test` route |
| `templates/admin/channel_detail.html` | Add Test button + result span per source row |

---

### Task 1: Add source_test handler and register route

**Files:**
- Modify: `src/routes/admin.rs`
- Modify: `src/main.rs`

---

- [ ] **Step 1: Add the source_test handler to admin.rs**

Add the following function at the end of `src/routes/admin.rs` (after `playlist_item_delete`):

```rust
pub async fn source_test(
    State(state): State<AppState>,
    Path(source_id): Path<i64>,
) -> Result<Html<String>, StatusCode> {
    let src = sqlx::query_as::<_, source::Source>(
        "SELECT * FROM sources WHERE id = ?",
    )
    .bind(source_id)
    .fetch_optional(&state.pool)
    .await
    .map_err(internal_error)?
    .ok_or(StatusCode::NOT_FOUND)?;

    let ok_html = r#"<span class="badge badge-on">OK</span>"#;
    let fail_html = r#"<span style="color:#e94560;font-size:0.78rem">Failed</span>"#;

    if resolver::needs_resolution(&src.url) {
        return Ok(Html(match resolver::resolve_url(&src.url).await {
            Ok(_) => ok_html.to_string(),
            Err(_) => fail_html.to_string(),
        }));
    }

    Ok(Html(match state
        .http_client
        .head(&src.url)
        .timeout(std::time::Duration::from_secs(10))
        .send()
        .await
    {
        Ok(resp) if resp.status().is_success() || resp.status().is_redirection() => {
            ok_html.to_string()
        }
        Ok(resp) => format!(
            r#"<span style="color:#e94560;font-size:0.78rem">Failed: HTTP {}</span>"#,
            resp.status().as_u16()
        ),
        Err(_) => fail_html.to_string(),
    }))
}
```

Notes:
- Uses the same `sqlx::query_as::<_, source::Source>` pattern as `source_toggle` and `source_delete` — no new import needed.
- `resolver` is already imported on line 12: `use crate::{channel, epg, playlist_item, resolver, source, AppState};`
- `std::time::Duration` used via full path — no new `use` needed.
- Always returns `Ok(Html(...))` (never an error status for the test result) so HTMX always has something to swap. Only source-not-found returns `Err`.

---

- [ ] **Step 2: Register the route in main.rs**

In `src/main.rs`, find the existing source routes (around line 56–57):

```rust
        .route("/sources/:id/delete", post(routes::admin::source_delete))
        .route("/sources/:id/toggle", post(routes::admin::source_toggle))
```

Add the new route immediately after:

```rust
        .route("/sources/:id/delete", post(routes::admin::source_delete))
        .route("/sources/:id/toggle", post(routes::admin::source_toggle))
        .route("/sources/:id/test", post(routes::admin::source_test))
```

---

- [ ] **Step 3: Run tests**

```bash
cd /Users/kunwu/Workspace/playground/MyTV && cargo test
```

Expected: all existing tests pass. If there are compile errors:
- Check `source_test` is `pub async fn` with the correct signature `(State(state): State<AppState>, Path(source_id): Path<i64>) -> Result<Html<String>, StatusCode>`
- Check `Html` is in scope — it's already imported via `use axum::response::{Html, IntoResponse, Redirect, Response};`

---

- [ ] **Step 4: Commit**

```bash
git add src/routes/admin.rs src/main.rs
git commit -m "feat: add source_test handler and route"
```

---

### Task 2: Add Test button and result span to channel_detail.html

**Files:**
- Modify: `templates/admin/channel_detail.html`

---

- [ ] **Step 1: Add the Test button and result span**

In `templates/admin/channel_detail.html`, find the source row's action `<td>` (the last `<td>` in the sources `{% for src in sources %}` loop). It currently reads:

```html
        <td style="white-space:nowrap">
          <form action="/admin/sources/{{ src.id }}/toggle" method="post" style="display:inline-block">
            <button class="btn btn-sm" type="submit">
              {% if src.is_active %}Disable{% else %}Enable{% endif %}
            </button>
          </form>
          <form action="/admin/sources/{{ src.id }}/delete" method="post" style="display:inline-block;margin-left:4px">
            <button class="btn btn-sm btn-danger" type="submit"
                    onclick="return confirm('Remove this source?')">Delete</button>
          </form>
        </td>
```

Replace it with:

```html
        <td style="white-space:nowrap">
          <form action="/admin/sources/{{ src.id }}/toggle" method="post" style="display:inline-block">
            <button class="btn btn-sm" type="submit">
              {% if src.is_active %}Disable{% else %}Enable{% endif %}
            </button>
          </form>
          <form action="/admin/sources/{{ src.id }}/delete" method="post" style="display:inline-block;margin-left:4px">
            <button class="btn btn-sm btn-danger" type="submit"
                    onclick="return confirm('Remove this source?')">Delete</button>
          </form>
          <form hx-post="/admin/sources/{{ src.id }}/test"
                hx-target="#src-test-{{ src.id }}"
                hx-swap="innerHTML"
                style="display:inline-block;margin-left:4px">
            <button class="btn btn-sm" type="submit">Test</button>
          </form>
          <span id="src-test-{{ src.id }}"></span>
        </td>
```

The `<span>` starts empty and receives the result HTML on click. Repeated clicks overwrite the previous result.

---

- [ ] **Step 2: Build to verify**

```bash
cd /Users/kunwu/Workspace/playground/MyTV && cargo build
```

Expected: compiles without errors. Askama templates compile at build time — any syntax error appears here.

---

- [ ] **Step 3: Commit**

```bash
git add templates/admin/channel_detail.html
git commit -m "feat: add Test button to source rows in channel detail"
```
