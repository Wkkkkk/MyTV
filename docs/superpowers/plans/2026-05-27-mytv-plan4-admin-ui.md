# MyTV Plan 4: Admin UI Implementation Plan

> **For agentic workers:** REQUIRED SUB-SKILL: Use superpowers:subagent-driven-development (recommended) or superpowers:executing-plans to implement this plan task-by-task. Steps use checkbox (`- [ ]`) syntax for tracking.

**Goal:** Build a password-protected `/admin` UI for managing channels, sources, and playlist items via server-rendered HTML forms.

**Architecture:** All admin routes live under a single Axum sub-router nested at `/admin`. An HTTP Basic Auth middleware layer checks every admin request against `ADMIN_PASSWORD` from config. Channel CRUD lives on `admin.rs` route handlers. After each mutation the handler redirects (POST/Redirect/GET pattern). Three Askama templates cover the channel list, the create/edit form, and the channel detail page (sources + playlist items). No new crates needed except `base64 = "0.22"` for decoding the Authorization header.

**Tech Stack:** Rust, Axum 0.7, Askama 0.12, sqlx 0.7 (SQLite), `base64 = "0.22"`, `chrono` (loop_anchor parsing)

---

## File Structure

```
Cargo.toml                     — (modify) add base64 = "0.22"
src/
  channel.rs                   — (modify) add UpdateChannel + update()
  routes/
    admin.rs                   — NEW: check_basic_auth(), basic_auth middleware,
                                       all admin handlers
    mod.rs                     — (modify) add pub mod admin
  main.rs                      — (modify) nest /admin sub-router with auth layer
templates/
  admin/
    base.html                  — NEW: admin HTML shell (no player JS)
    channels.html              — NEW: channel list
    channel_form.html          — NEW: create/edit channel (shared)
    channel_detail.html        — NEW: channel sources + playlist items
```

---

## Task 1: channel::update + auth middleware + routing scaffold

**Files:**
- Modify: `Cargo.toml`
- Modify: `src/channel.rs`
- Create: `src/routes/admin.rs`
- Modify: `src/routes/mod.rs`
- Modify: `src/main.rs`

- [ ] **Step 1: Add base64 dependency to Cargo.toml**

Add one line to `[dependencies]` in `Cargo.toml`:

```toml
[dependencies]
axum = "0.7"
tokio = { version = "1", features = ["full"] }
sqlx = { version = "0.7", features = ["runtime-tokio-rustls", "sqlite", "migrate", "chrono"] }
tower-http = { version = "0.5", features = ["trace"] }
serde = { version = "1", features = ["derive"] }
serde_json = "1"
chrono = { version = "0.4", features = ["serde"] }
dotenvy = "0.15"
tracing = "0.1"
tracing-subscriber = { version = "0.3", features = ["env-filter"] }
anyhow = "1"
askama = "0.12"
base64 = "0.22"
```

- [ ] **Step 2: Write failing tests for channel::update**

Add to the `#[cfg(test)]` block at the bottom of `src/channel.rs`:

```rust
    #[tokio::test]
    async fn test_update_channel_name_and_category() {
        let pool = test_pool().await;
        let ch = create(&pool, live("CNN", "news")).await.unwrap();

        let updated = update(
            &pool,
            ch.id,
            UpdateChannel {
                name: "CNN International".to_string(),
                category: "world".to_string(),
                logo_url: None,
                channel_type: "live".to_string(),
                sort_order: 1,
                loop_anchor: None,
            },
        )
        .await
        .unwrap()
        .unwrap();

        assert_eq!(updated.name, "CNN International");
        assert_eq!(updated.category, "world");
        assert_eq!(updated.sort_order, 1);
    }

    #[tokio::test]
    async fn test_update_nonexistent_channel_returns_none() {
        let pool = test_pool().await;
        let result = update(
            &pool,
            9999,
            UpdateChannel {
                name: "Ghost".to_string(),
                category: "none".to_string(),
                logo_url: None,
                channel_type: "live".to_string(),
                sort_order: 0,
                loop_anchor: None,
            },
        )
        .await
        .unwrap();
        assert!(result.is_none());
    }
```

- [ ] **Step 3: Run the tests to see them fail**

```bash
cd /Users/kunwu/Workspace/playground/MyTV && cargo test channel::tests::test_update 2>&1 | head -20
```

Expected: compile error — `UpdateChannel` and `update` not defined yet.

- [ ] **Step 4: Implement UpdateChannel + update() in src/channel.rs**

Add after the `NewChannel` struct and `create` function (around line 59):

```rust
pub struct UpdateChannel {
    pub name: String,
    pub category: String,
    pub logo_url: Option<String>,
    pub channel_type: String,
    pub sort_order: i64,
    pub loop_anchor: Option<DateTime<Utc>>,
}

pub async fn update(pool: &SqlitePool, id: i64, input: UpdateChannel) -> Result<Option<Channel>> {
    let rows = sqlx::query(
        "UPDATE channels SET name = ?, category = ?, logo_url = ?, type = ?, sort_order = ?, loop_anchor = ? WHERE id = ?",
    )
    .bind(&input.name)
    .bind(&input.category)
    .bind(&input.logo_url)
    .bind(&input.channel_type)
    .bind(input.sort_order)
    .bind(input.loop_anchor)
    .bind(id)
    .execute(pool)
    .await?
    .rows_affected();

    if rows == 0 {
        return Ok(None);
    }
    get(pool, id).await
}
```

- [ ] **Step 5: Run the update tests to confirm they pass**

```bash
cd /Users/kunwu/Workspace/playground/MyTV && cargo test channel::tests::test_update 2>&1
```

Expected: both tests pass.

- [ ] **Step 6: Write failing tests for check_basic_auth**

Create `src/routes/admin.rs` with just the pure auth function and its tests:

```rust
use axum::{
    extract::{Request, State},
    http::{header, StatusCode},
    middleware::Next,
    response::{IntoResponse, Response},
};
use base64::{engine::general_purpose, Engine as _};

use crate::AppState;

/// Returns true if the Authorization: Basic header value has the correct password.
/// Username is ignored — any username with the correct password is accepted.
pub fn check_basic_auth(header_value: &str, expected_password: &str) -> bool {
    header_value
        .strip_prefix("Basic ")
        .and_then(|b64| general_purpose::STANDARD.decode(b64).ok())
        .and_then(|bytes| String::from_utf8(bytes).ok())
        .map(|credentials| {
            credentials.splitn(2, ':').nth(1).unwrap_or("") == expected_password
        })
        .unwrap_or(false)
}

pub async fn basic_auth(
    State(state): State<AppState>,
    request: Request,
    next: Next,
) -> Response {
    let authorized = request
        .headers()
        .get(header::AUTHORIZATION)
        .and_then(|v| v.to_str().ok())
        .map(|v| check_basic_auth(v, &state.config.admin_password))
        .unwrap_or(false);

    if authorized {
        next.run(request).await
    } else {
        (
            StatusCode::UNAUTHORIZED,
            [(header::WWW_AUTHENTICATE, "Basic realm=\"MyTV Admin\"")],
            "Unauthorized",
        )
            .into_response()
    }
}

pub async fn admin_index() -> impl IntoResponse {
    axum::response::Redirect::to("/admin/channels")
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn test_check_basic_auth_valid_credentials() {
        // base64("user:secret") = "dXNlcjpzZWNyZXQ="
        assert!(check_basic_auth("Basic dXNlcjpzZWNyZXQ=", "secret"));
    }

    #[test]
    fn test_check_basic_auth_wrong_password() {
        // base64("user:wrong") = "dXNlcjp3cm9uZw=="
        assert!(!check_basic_auth("Basic dXNlcjp3cm9uZw==", "secret"));
    }

    #[test]
    fn test_check_basic_auth_malformed_no_basic_prefix() {
        assert!(!check_basic_auth("Bearer sometoken", "secret"));
    }

    #[test]
    fn test_check_basic_auth_empty_header() {
        assert!(!check_basic_auth("", "secret"));
    }

    #[test]
    fn test_check_basic_auth_no_colon_in_credentials() {
        // base64("passwordonly") = "cGFzc3dvcmRvbmx5"
        assert!(!check_basic_auth("Basic cGFzc3dvcmRvbmx5", "passwordonly"));
    }
}
```

- [ ] **Step 7: Run the auth tests to confirm they pass**

```bash
cd /Users/kunwu/Workspace/playground/MyTV && cargo test admin::tests 2>&1
```

Expected: 5 tests pass.

- [ ] **Step 8: Add pub mod admin to src/routes/mod.rs**

Replace `src/routes/mod.rs` with:

```rust
pub mod admin;
pub mod guide;
pub mod health;
pub mod player;
```

- [ ] **Step 9: Wire the admin sub-router in src/main.rs**

Replace `src/main.rs` with:

```rust
mod channel;
mod config;
mod db;
mod epg;
mod playlist_item;
mod resolver;
mod routes;
mod source;

use anyhow::Result;
use axum::{middleware, routing::get, Router};
use sqlx::SqlitePool;
use std::sync::Arc;
use tracing_subscriber::EnvFilter;

#[derive(Clone)]
pub struct AppState {
    pub pool: SqlitePool,
    pub config: Arc<config::Config>,
}

#[tokio::main]
async fn main() -> Result<()> {
    dotenvy::dotenv().ok();
    tracing_subscriber::fmt()
        .with_env_filter(EnvFilter::from_default_env())
        .init();

    let config = Arc::new(config::Config::from_env()?);
    let pool = db::connect(&config.database_url).await?;

    let state = AppState {
        pool,
        config: config.clone(),
    };

    let admin_router: Router<AppState> = Router::new()
        .route("/", get(routes::admin::admin_index))
        .route_layer(middleware::from_fn_with_state(
            state.clone(),
            routes::admin::basic_auth,
        ));

    let app = Router::new()
        .route("/health", get(routes::health::health_check))
        .route("/guide", get(routes::guide::guide_page))
        .route("/guide/partial", get(routes::guide::guide_partial))
        .route("/channel/:id/tune", get(routes::player::tune))
        .route("/channel/:id/next", get(routes::player::next))
        .nest("/admin", admin_router)
        .with_state(state);

    let addr = format!("0.0.0.0:{}", config.port);
    tracing::info!("listening on {addr}");
    let listener = tokio::net::TcpListener::bind(&addr).await?;
    axum::serve(listener, app).await?;

    Ok(())
}
```

- [ ] **Step 10: Build to confirm no compile errors**

```bash
cd /Users/kunwu/Workspace/playground/MyTV && cargo build 2>&1 | grep -E "^error"
```

Expected: no output.

- [ ] **Step 11: Run full test suite**

```bash
cd /Users/kunwu/Workspace/playground/MyTV && cargo test -- --test-threads=1 2>&1 | tail -5
```

Expected: all tests pass (now includes 7 new tests: 2 channel::update + 5 admin::auth).

- [ ] **Step 12: Smoke test auth**

```bash
cd /Users/kunwu/Workspace/playground/MyTV && DATABASE_URL=sqlite:mytv.db RUST_LOG=error cargo run &
sleep 3

# Without auth → 401
curl -s -o /dev/null -w "%{http_code}" http://localhost:3000/admin/
echo ""

# With wrong password → 401
curl -s -o /dev/null -w "%{http_code}" -u admin:wrong http://localhost:3000/admin/
echo ""

# With correct default password → 302 redirect to /admin/channels
curl -s -o /dev/null -w "%{http_code}" -u admin:admin http://localhost:3000/admin/
echo ""

kill %1 2>/dev/null || true
```

Expected output:
```
401
401
302
```

- [ ] **Step 13: Commit**

```bash
cd /Users/kunwu/Workspace/playground/MyTV && \
git add Cargo.toml Cargo.lock src/channel.rs src/routes/admin.rs src/routes/mod.rs src/main.rs && \
git commit -m "feat: add channel::update and admin auth middleware scaffold"
```

---

## Task 2: Channel CRUD handlers + templates

**Files:**
- Modify: `src/routes/admin.rs` (add display types + channel CRUD handlers)
- Modify: `src/main.rs` (expand admin router with channel routes)
- Create: `templates/admin/base.html`
- Create: `templates/admin/channels.html`
- Create: `templates/admin/channel_form.html`

- [ ] **Step 1: Create templates/admin/base.html**

```bash
mkdir -p /Users/kunwu/Workspace/playground/MyTV/templates/admin
```

Create `/Users/kunwu/Workspace/playground/MyTV/templates/admin/base.html`:

```html
<!DOCTYPE html>
<html lang="en">
<head>
  <meta charset="utf-8">
  <meta name="viewport" content="width=device-width, initial-scale=1">
  <title>MyTV Admin</title>
  <style>
    *{box-sizing:border-box;margin:0;padding:0}
    body{background:#0f0f0f;color:#e0e0e0;font-family:system-ui,sans-serif;min-height:100vh}
    a{color:#e94560;text-decoration:none}
    a:hover{text-decoration:underline}

    .site-header{background:#111;padding:10px 16px;display:flex;align-items:center;gap:16px;border-bottom:1px solid #222}
    .site-header h1{font-size:1.1rem;color:#e94560;letter-spacing:1px}
    .site-header nav{display:flex;gap:12px;font-size:0.85rem}
    .site-header nav a{color:#999}
    .site-header nav a:hover{color:#e94560}

    main{max-width:960px;margin:0 auto;padding:24px 16px}
    h2{font-size:1.1rem;margin-bottom:16px;color:#ddd}
    h3{font-size:0.95rem;margin-bottom:12px;color:#bbb;margin-top:0}

    table{width:100%;border-collapse:collapse;font-size:0.85rem}
    th{text-align:left;padding:6px 10px;border-bottom:2px solid #222;color:#777;font-weight:600;white-space:nowrap}
    td{padding:6px 10px;border-bottom:1px solid #1a1a1a;vertical-align:middle}
    tr:hover td{background:#111}

    .btn{display:inline-block;padding:4px 12px;border-radius:3px;border:1px solid #333;
         background:#1a1a1a;color:#ccc;cursor:pointer;font-size:0.82rem;text-decoration:none;
         line-height:1.5;vertical-align:middle}
    .btn:hover{background:#222;text-decoration:none;color:#fff}
    .btn-primary{background:#e94560;color:#fff;border-color:#e94560}
    .btn-primary:hover{background:#c73050;color:#fff}
    .btn-danger{background:#3a1015;color:#f77;border-color:#5a2025}
    .btn-danger:hover{background:#5a1020;color:#f99}
    .btn-sm{padding:2px 8px;font-size:0.78rem}

    .form-row{margin-bottom:14px}
    label{display:block;font-size:0.8rem;color:#888;margin-bottom:4px}
    input[type=text],input[type=number],input[type=datetime-local],select{
      width:100%;padding:7px 10px;background:#111;border:1px solid #2a2a2a;
      color:#e0e0e0;border-radius:3px;font-size:0.85rem}
    input:focus,select:focus{outline:none;border-color:#e94560}
    .form-actions{margin-top:20px;display:flex;gap:10px;align-items:center}

    .badge{display:inline-block;padding:1px 8px;border-radius:10px;font-size:0.72rem;font-weight:700;letter-spacing:0.3px}
    .badge-live{background:#0a2a0a;color:#6d6;border:1px solid #1a4a1a}
    .badge-vod{background:#0a0a2a;color:#88f;border:1px solid #1a1a4a}
    .badge-on{background:#0a2a0a;color:#6d6}
    .badge-off{background:#2a0a0a;color:#f77}

    .section{margin-top:32px;padding-top:24px;border-top:1px solid #1c1c1c}
    .page-header{display:flex;justify-content:space-between;align-items:center;margin-bottom:16px}
    .empty-state{color:#555;font-size:0.85rem;padding:12px 0}
  </style>
</head>
<body>
  <header class="site-header">
    <h1>MyTV Admin</h1>
    <nav>
      <a href="/guide">← Guide</a>
      <a href="/admin/channels">Channels</a>
    </nav>
  </header>
  <main>
    {% block content %}{% endblock %}
  </main>
</body>
</html>
```

- [ ] **Step 2: Create templates/admin/channels.html**

Create `/Users/kunwu/Workspace/playground/MyTV/templates/admin/channels.html`:

```html
{% extends "admin/base.html" %}
{% block content %}
<div class="page-header">
  <h2>Channels</h2>
  <a class="btn btn-primary" href="/admin/channels/new">+ New Channel</a>
</div>

{% if channels.is_empty() %}
<p class="empty-state">No channels yet. <a href="/admin/channels/new">Add the first one.</a></p>
{% else %}
<table>
  <thead>
    <tr>
      <th>Name</th>
      <th>Category</th>
      <th>Type</th>
      <th>Sort</th>
      <th></th>
    </tr>
  </thead>
  <tbody>
    {% for ch in channels %}
    <tr>
      <td><a href="/admin/channels/{{ ch.id }}">{{ ch.name }}</a></td>
      <td>{{ ch.category }}</td>
      <td>
        {% if ch.type_str.as_str() == "live" %}
        <span class="badge badge-live">live</span>
        {% else %}
        <span class="badge badge-vod">vod loop</span>
        {% endif %}
      </td>
      <td>{{ ch.sort_order }}</td>
      <td style="white-space:nowrap">
        <a class="btn btn-sm" href="/admin/channels/{{ ch.id }}/edit">Edit</a>
        <form action="/admin/channels/{{ ch.id }}/delete" method="post" style="display:inline-block;margin-left:4px">
          <button class="btn btn-sm btn-danger" type="submit"
                  onclick="return confirm('Delete this channel? All its sources and playlist items will also be removed.')">Delete</button>
        </form>
      </td>
    </tr>
    {% endfor %}
  </tbody>
</table>
{% endif %}
{% endblock %}
```

- [ ] **Step 3: Create templates/admin/channel_form.html**

Create `/Users/kunwu/Workspace/playground/MyTV/templates/admin/channel_form.html`:

```html
{% extends "admin/base.html" %}
{% block content %}
{% if is_edit %}
<h2>Edit Channel</h2>
<form action="/admin/channels/{{ channel_id }}" method="post">
{% else %}
<h2>New Channel</h2>
<form action="/admin/channels" method="post">
{% endif %}
  <div class="form-row">
    <label for="name">Name</label>
    <input id="name" type="text" name="name" required value="{{ name }}">
  </div>
  <div class="form-row">
    <label for="category">Category</label>
    <input id="category" type="text" name="category" required value="{{ category }}"
           placeholder="e.g. news, sports, movies">
  </div>
  <div class="form-row">
    <label for="channel_type">Type</label>
    <select id="channel_type" name="channel_type">
      <option value="live"{% if channel_type.as_str() == "live" %} selected{% endif %}>Live stream</option>
      <option value="vod_loop"{% if channel_type.as_str() == "vod_loop" %} selected{% endif %}>VOD loop</option>
    </select>
  </div>
  <div class="form-row">
    <label for="sort_order">Sort Order</label>
    <input id="sort_order" type="number" name="sort_order" value="{{ sort_order }}" style="width:120px">
  </div>
  <div class="form-row">
    <label for="logo_url">Logo URL <span style="color:#555">(optional)</span></label>
    <input id="logo_url" type="text" name="logo_url" value="{{ logo_url }}" placeholder="https://...">
  </div>
  <div class="form-row">
    <label for="loop_anchor">Loop Anchor <span style="color:#555">(UTC — required for VOD loop; leave blank to use current time)</span></label>
    <input id="loop_anchor" type="datetime-local" name="loop_anchor" value="{{ loop_anchor }}" style="width:240px">
  </div>
  <div class="form-actions">
    <button class="btn btn-primary" type="submit">{% if is_edit %}Save Changes{% else %}Create Channel{% endif %}</button>
    <a class="btn" href="/admin/channels">Cancel</a>
  </div>
</form>
{% endblock %}
```

- [ ] **Step 4: Add display types and channel CRUD handlers to src/routes/admin.rs**

Replace the entire `src/routes/admin.rs` with:

```rust
use askama::Template;
use axum::{
    extract::{Form, Path, Request, State},
    http::{header, StatusCode},
    middleware::Next,
    response::{Html, IntoResponse, Redirect, Response},
};
use base64::{engine::general_purpose, Engine as _};
use chrono::{DateTime, NaiveDateTime, Utc};
use serde::Deserialize;

use crate::{channel, AppState};

// ── auth ───────────────────────────────────────────────────────────────────

/// Returns true if `Authorization: Basic` header has the correct password.
/// Username is ignored.
pub fn check_basic_auth(header_value: &str, expected_password: &str) -> bool {
    header_value
        .strip_prefix("Basic ")
        .and_then(|b64| general_purpose::STANDARD.decode(b64).ok())
        .and_then(|bytes| String::from_utf8(bytes).ok())
        .map(|credentials| {
            credentials.splitn(2, ':').nth(1).unwrap_or("") == expected_password
        })
        .unwrap_or(false)
}

pub async fn basic_auth(
    State(state): State<AppState>,
    request: Request,
    next: Next,
) -> Response {
    let authorized = request
        .headers()
        .get(header::AUTHORIZATION)
        .and_then(|v| v.to_str().ok())
        .map(|v| check_basic_auth(v, &state.config.admin_password))
        .unwrap_or(false);

    if authorized {
        next.run(request).await
    } else {
        (
            StatusCode::UNAUTHORIZED,
            [(header::WWW_AUTHENTICATE, "Basic realm=\"MyTV Admin\"")],
            "Unauthorized",
        )
            .into_response()
    }
}

// ── display types ──────────────────────────────────────────────────────────

pub struct AdminChannelRow {
    pub id: i64,
    pub name: String,
    pub category: String,
    pub type_str: String,
    pub sort_order: i64,
}

// ── template types ─────────────────────────────────────────────────────────

#[derive(Template)]
#[template(path = "admin/channels.html")]
struct ChannelListTemplate {
    channels: Vec<AdminChannelRow>,
}

#[derive(Template)]
#[template(path = "admin/channel_form.html")]
struct ChannelFormTemplate {
    is_edit: bool,
    channel_id: i64,
    name: String,
    category: String,
    channel_type: String,
    sort_order: i64,
    logo_url: String,
    loop_anchor: String,
}

// ── form input types ───────────────────────────────────────────────────────

#[derive(Deserialize)]
pub struct ChannelForm {
    pub name: String,
    pub category: String,
    pub channel_type: String,
    pub sort_order: String,
    pub logo_url: String,
    pub loop_anchor: String,
}

// ── helpers ────────────────────────────────────────────────────────────────

fn parse_loop_anchor(s: &str) -> Option<DateTime<Utc>> {
    let trimmed = s.trim();
    if trimmed.is_empty() {
        return None;
    }
    NaiveDateTime::parse_from_str(trimmed, "%Y-%m-%dT%H:%M")
        .ok()
        .map(|dt| DateTime::from_naive_utc_and_offset(dt, Utc))
}

fn render<T: askama::Template>(t: T) -> Result<Html<String>, StatusCode> {
    t.render()
        .map(Html)
        .map_err(|e| {
            tracing::error!("template render error: {e}");
            StatusCode::INTERNAL_SERVER_ERROR
        })
}

fn internal_error<E: std::fmt::Display>(e: E) -> StatusCode {
    tracing::error!("admin error: {e}");
    StatusCode::INTERNAL_SERVER_ERROR
}

// ── handlers ───────────────────────────────────────────────────────────────

pub async fn admin_index() -> impl IntoResponse {
    Redirect::to("/admin/channels")
}

pub async fn channel_list(
    State(state): State<AppState>,
) -> Result<Html<String>, StatusCode> {
    let all = channel::list(&state.pool).await.map_err(internal_error)?;
    let channels = all
        .into_iter()
        .map(|ch| AdminChannelRow {
            id: ch.id,
            name: ch.name,
            category: ch.category,
            type_str: ch.r#type,
            sort_order: ch.sort_order,
        })
        .collect();
    render(ChannelListTemplate { channels })
}

pub async fn channel_new_form() -> Result<Html<String>, StatusCode> {
    render(ChannelFormTemplate {
        is_edit: false,
        channel_id: 0,
        name: String::new(),
        category: String::new(),
        channel_type: "live".to_string(),
        sort_order: 0,
        logo_url: String::new(),
        loop_anchor: String::new(),
    })
}

pub async fn channel_create(
    State(state): State<AppState>,
    Form(form): Form<ChannelForm>,
) -> Result<impl IntoResponse, StatusCode> {
    let sort_order: i64 = form.sort_order.trim().parse().unwrap_or(0);
    let logo_url = if form.logo_url.trim().is_empty() {
        None
    } else {
        Some(form.logo_url.trim().to_string())
    };
    let loop_anchor = if form.channel_type.as_str() == "vod_loop" {
        parse_loop_anchor(&form.loop_anchor).or_else(|| Some(Utc::now()))
    } else {
        None
    };

    channel::create(
        &state.pool,
        channel::NewChannel {
            name: form.name.trim().to_string(),
            category: form.category.trim().to_string(),
            logo_url,
            channel_type: form.channel_type.clone(),
            sort_order,
            loop_anchor,
        },
    )
    .await
    .map_err(internal_error)?;

    Ok(Redirect::to("/admin/channels"))
}

pub async fn channel_edit_form(
    State(state): State<AppState>,
    Path(id): Path<i64>,
) -> Result<Html<String>, StatusCode> {
    let ch = channel::get(&state.pool, id)
        .await
        .map_err(internal_error)?
        .ok_or(StatusCode::NOT_FOUND)?;

    render(ChannelFormTemplate {
        is_edit: true,
        channel_id: ch.id,
        name: ch.name,
        category: ch.category,
        channel_type: ch.r#type,
        sort_order: ch.sort_order,
        logo_url: ch.logo_url.unwrap_or_default(),
        loop_anchor: ch
            .loop_anchor
            .map(|dt| dt.format("%Y-%m-%dT%H:%M").to_string())
            .unwrap_or_default(),
    })
}

pub async fn channel_update(
    State(state): State<AppState>,
    Path(id): Path<i64>,
    Form(form): Form<ChannelForm>,
) -> Result<impl IntoResponse, StatusCode> {
    let existing = channel::get(&state.pool, id)
        .await
        .map_err(internal_error)?
        .ok_or(StatusCode::NOT_FOUND)?;

    let sort_order: i64 = form.sort_order.trim().parse().unwrap_or(0);
    let logo_url = if form.logo_url.trim().is_empty() {
        None
    } else {
        Some(form.logo_url.trim().to_string())
    };
    let loop_anchor = if form.channel_type.as_str() == "vod_loop" {
        parse_loop_anchor(&form.loop_anchor).or(existing.loop_anchor)
    } else {
        None
    };

    channel::update(
        &state.pool,
        id,
        channel::UpdateChannel {
            name: form.name.trim().to_string(),
            category: form.category.trim().to_string(),
            logo_url,
            channel_type: form.channel_type.clone(),
            sort_order,
            loop_anchor,
        },
    )
    .await
    .map_err(internal_error)?;

    Ok(Redirect::to("/admin/channels"))
}

pub async fn channel_delete(
    State(state): State<AppState>,
    Path(id): Path<i64>,
) -> Result<impl IntoResponse, StatusCode> {
    channel::delete(&state.pool, id)
        .await
        .map_err(internal_error)?;
    Ok(Redirect::to("/admin/channels"))
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn test_check_basic_auth_valid_credentials() {
        // base64("user:secret") = "dXNlcjpzZWNyZXQ="
        assert!(check_basic_auth("Basic dXNlcjpzZWNyZXQ=", "secret"));
    }

    #[test]
    fn test_check_basic_auth_wrong_password() {
        // base64("user:wrong") = "dXNlcjp3cm9uZw=="
        assert!(!check_basic_auth("Basic dXNlcjp3cm9uZw==", "secret"));
    }

    #[test]
    fn test_check_basic_auth_malformed_no_basic_prefix() {
        assert!(!check_basic_auth("Bearer sometoken", "secret"));
    }

    #[test]
    fn test_check_basic_auth_empty_header() {
        assert!(!check_basic_auth("", "secret"));
    }

    #[test]
    fn test_check_basic_auth_no_colon_in_credentials() {
        // base64("passwordonly") = "cGFzc3dvcmRvbmx5"
        assert!(!check_basic_auth("Basic cGFzc3dvcmRvbmx5", "passwordonly"));
    }
}
```

- [ ] **Step 5: Expand the admin router in src/main.rs**

Replace the `admin_router` block in `src/main.rs`:

```rust
    let admin_router: Router<AppState> = Router::new()
        .route("/", get(routes::admin::admin_index))
        .route("/channels", get(routes::admin::channel_list))
        .route(
            "/channels/new",
            get(routes::admin::channel_new_form),
        )
        .route("/channels", post(routes::admin::channel_create))
        .route(
            "/channels/:id/edit",
            get(routes::admin::channel_edit_form),
        )
        .route("/channels/:id", post(routes::admin::channel_update))
        .route(
            "/channels/:id/delete",
            post(routes::admin::channel_delete),
        )
        .route_layer(middleware::from_fn_with_state(
            state.clone(),
            routes::admin::basic_auth,
        ));
```

Also add `post` to the axum routing import at the top of main.rs — replace:
```rust
use axum::{middleware, routing::get, Router};
```
with:
```rust
use axum::{middleware, routing::{get, post}, Router};
```

- [ ] **Step 6: Build to confirm templates and routes compile**

```bash
cd /Users/kunwu/Workspace/playground/MyTV && cargo build 2>&1 | grep -E "^error"
```

Expected: no output. Askama validates templates at compile time.

- [ ] **Step 7: Run full test suite**

```bash
cd /Users/kunwu/Workspace/playground/MyTV && cargo test -- --test-threads=1 2>&1 | tail -5
```

Expected: all tests pass.

- [ ] **Step 8: Smoke test channel CRUD**

```bash
cd /Users/kunwu/Workspace/playground/MyTV && DATABASE_URL=sqlite:mytv.db RUST_LOG=error cargo run &
sleep 3

# List channels (authenticated) → 200 with HTML
STATUS=$(curl -s -o /dev/null -w "%{http_code}" -u admin:admin http://localhost:3000/admin/channels)
echo "channel list: $STATUS"

# New channel form → 200
STATUS=$(curl -s -o /dev/null -w "%{http_code}" -u admin:admin http://localhost:3000/admin/channels/new)
echo "new form: $STATUS"

# Edit existing channel 1 → 200
STATUS=$(curl -s -o /dev/null -w "%{http_code}" -u admin:admin http://localhost:3000/admin/channels/1/edit)
echo "edit form ch1: $STATUS"

# Edit nonexistent channel → 404
STATUS=$(curl -s -o /dev/null -w "%{http_code}" -u admin:admin http://localhost:3000/admin/channels/9999/edit)
echo "edit form ch9999: $STATUS"

# Create a new channel → 302
STATUS=$(curl -s -o /dev/null -w "%{http_code}" -u admin:admin \
  -d "name=Test+Channel&category=test&channel_type=live&sort_order=99&logo_url=&loop_anchor=" \
  http://localhost:3000/admin/channels)
echo "create channel: $STATUS"

kill %1 2>/dev/null || true
```

Expected:
```
channel list: 200
new form: 200
edit form ch1: 200
edit form ch9999: 404
create channel: 302
```

- [ ] **Step 9: Commit**

```bash
cd /Users/kunwu/Workspace/playground/MyTV && \
git add src/routes/admin.rs src/main.rs templates/admin/ && \
git commit -m "feat: add admin channel CRUD with basic auth"
```

---

## Task 3: Channel detail — sources + playlist items

**Files:**
- Modify: `src/routes/admin.rs` (add source + playlist handlers + channel_detail handler)
- Modify: `src/main.rs` (add source and playlist routes to admin router)
- Create: `templates/admin/channel_detail.html`

- [ ] **Step 1: Create templates/admin/channel_detail.html**

Create `/Users/kunwu/Workspace/playground/MyTV/templates/admin/channel_detail.html`:

```html
{% extends "admin/base.html" %}
{% block content %}
<div class="page-header">
  <div>
    <h2>{{ channel_name }}</h2>
    <span style="font-size:0.8rem;color:#666">
      {% if channel_type.as_str() == "live" %}
      <span class="badge badge-live">live</span>
      {% else %}
      <span class="badge badge-vod">vod loop</span>
      {% endif %}
      &nbsp;id: {{ channel_id }}
    </span>
  </div>
  <div style="display:flex;gap:8px">
    <a class="btn btn-sm" href="/admin/channels/{{ channel_id }}/edit">Edit Channel</a>
    <a class="btn btn-sm" href="/admin/channels">← All Channels</a>
  </div>
</div>

<!-- Sources -->
<div class="section">
  <h3>Sources</h3>
  {% if sources.is_empty() %}
  <p class="empty-state">No sources. Add one below.</p>
  {% else %}
  <table style="margin-bottom:16px">
    <thead>
      <tr><th>Kind</th><th>URL</th><th>Priority</th><th>Active</th><th></th></tr>
    </thead>
    <tbody>
      {% for src in sources %}
      <tr>
        <td>{{ src.kind }}</td>
        <td style="word-break:break-all;max-width:400px;font-size:0.78rem">{{ src.url }}</td>
        <td>{{ src.priority }}</td>
        <td>
          {% if src.is_active %}
          <span class="badge badge-on">on</span>
          {% else %}
          <span class="badge badge-off">off</span>
          {% endif %}
        </td>
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
      </tr>
      {% endfor %}
    </tbody>
  </table>
  {% endif %}

  <form action="/admin/channels/{{ channel_id }}/sources" method="post"
        style="display:flex;gap:10px;flex-wrap:wrap;align-items:flex-end">
    <div class="form-row" style="margin:0;min-width:110px">
      <label>Kind</label>
      <select name="kind">
        <option value="hls">hls</option>
        <option value="youtube_live">youtube_live</option>
        <option value="iptv">iptv</option>
      </select>
    </div>
    <div class="form-row" style="margin:0;flex:1;min-width:240px">
      <label>URL</label>
      <input type="text" name="url" required placeholder="https://...">
    </div>
    <div class="form-row" style="margin:0;width:90px">
      <label>Priority</label>
      <input type="number" name="priority" value="1" min="1">
    </div>
    <button class="btn btn-primary btn-sm" type="submit" style="margin-bottom:1px">Add Source</button>
  </form>
</div>

<!-- Playlist items (vod_loop only) -->
{% if channel_type.as_str() == "vod_loop" %}
<div class="section">
  <h3>Playlist</h3>
  {% if playlist_items.is_empty() %}
  <p class="empty-state">No playlist items. Add one below.</p>
  {% else %}
  <table style="margin-bottom:16px">
    <thead>
      <tr><th>#</th><th>Title</th><th>URL</th><th>Duration</th><th></th></tr>
    </thead>
    <tbody>
      {% for item in playlist_items %}
      <tr>
        <td style="color:#555">{{ item.sort_order }}</td>
        <td>{{ item.title }}</td>
        <td style="word-break:break-all;max-width:360px;font-size:0.78rem">{{ item.url }}</td>
        <td style="white-space:nowrap">{{ item.duration_secs }}s</td>
        <td>
          <form action="/admin/playlist/{{ item.id }}/delete" method="post">
            <button class="btn btn-sm btn-danger" type="submit"
                    onclick="return confirm('Remove this item?')">Delete</button>
          </form>
        </td>
      </tr>
      {% endfor %}
    </tbody>
  </table>
  {% endif %}

  <form action="/admin/channels/{{ channel_id }}/playlist" method="post"
        style="display:flex;gap:10px;flex-wrap:wrap;align-items:flex-end">
    <div class="form-row" style="margin:0;min-width:160px">
      <label>Title</label>
      <input type="text" name="title" required placeholder="Episode title">
    </div>
    <div class="form-row" style="margin:0;flex:1;min-width:240px">
      <label>URL</label>
      <input type="text" name="url" required placeholder="https://...">
    </div>
    <div class="form-row" style="margin:0;width:110px">
      <label>Duration (secs)</label>
      <input type="number" name="duration_secs" required min="1" placeholder="3600">
    </div>
    <button class="btn btn-primary btn-sm" type="submit" style="margin-bottom:1px">Add Item</button>
  </form>
</div>
{% endif %}
{% endblock %}
```

- [ ] **Step 2: Add source + playlist display types to src/routes/admin.rs**

Add these structs after the existing `AdminChannelRow` struct in `src/routes/admin.rs`:

```rust
pub struct AdminSourceRow {
    pub id: i64,
    pub kind: String,
    pub url: String,
    pub priority: i64,
    pub is_active: bool,
}

pub struct AdminPlaylistItemRow {
    pub id: i64,
    pub title: String,
    pub url: String,
    pub duration_secs: i64,
    pub sort_order: i64,
}
```

- [ ] **Step 3: Add ChannelDetailTemplate to src/routes/admin.rs**

Add after the existing `ChannelFormTemplate` struct:

```rust
#[derive(Template)]
#[template(path = "admin/channel_detail.html")]
struct ChannelDetailTemplate {
    channel_id: i64,
    channel_name: String,
    channel_type: String,
    sources: Vec<AdminSourceRow>,
    playlist_items: Vec<AdminPlaylistItemRow>,
}
```

- [ ] **Step 4: Add form input types for sources and playlist items to src/routes/admin.rs**

Add after the existing `ChannelForm` struct:

```rust
#[derive(Deserialize)]
pub struct SourceForm {
    pub kind: String,
    pub url: String,
    pub priority: String,
}

#[derive(Deserialize)]
pub struct PlaylistItemForm {
    pub title: String,
    pub url: String,
    pub duration_secs: String,
}
```

- [ ] **Step 5: Add source + playlist handlers + channel_detail to src/routes/admin.rs**

Add to the imports at the top of `src/routes/admin.rs` (add `playlist_item` and `source` to existing `use crate::...`):

```rust
use crate::{channel, playlist_item, source, AppState};
```

Also add `playlist_item::NewPlaylistItem` and `source::NewSource` imports as needed (they are re-exported from the module, accessible as `playlist_item::NewPlaylistItem` and `source::NewSource`).

Add these handler functions after `channel_delete`:

```rust
pub async fn channel_detail(
    State(state): State<AppState>,
    Path(id): Path<i64>,
) -> Result<Html<String>, StatusCode> {
    let ch = channel::get(&state.pool, id)
        .await
        .map_err(internal_error)?
        .ok_or(StatusCode::NOT_FOUND)?;

    let srcs = source::list_for_channel(&state.pool, id)
        .await
        .map_err(internal_error)?;

    let items = playlist_item::list_for_channel(&state.pool, id)
        .await
        .map_err(internal_error)?;

    render(ChannelDetailTemplate {
        channel_id: ch.id,
        channel_name: ch.name,
        channel_type: ch.r#type,
        sources: srcs
            .into_iter()
            .map(|s| AdminSourceRow {
                id: s.id,
                kind: s.kind,
                url: s.url,
                priority: s.priority,
                is_active: s.is_active,
            })
            .collect(),
        playlist_items: items
            .into_iter()
            .map(|i| AdminPlaylistItemRow {
                id: i.id,
                title: i.title,
                url: i.url,
                duration_secs: i.duration_secs,
                sort_order: i.sort_order,
            })
            .collect(),
    })
}

pub async fn source_create(
    State(state): State<AppState>,
    Path(channel_id): Path<i64>,
    Form(form): Form<SourceForm>,
) -> Result<impl IntoResponse, StatusCode> {
    let priority: i64 = form.priority.trim().parse().unwrap_or(1);
    source::create(
        &state.pool,
        source::NewSource {
            channel_id,
            kind: form.kind.clone(),
            url: form.url.trim().to_string(),
            priority,
        },
    )
    .await
    .map_err(internal_error)?;
    Ok(Redirect::to(&format!("/admin/channels/{channel_id}")))
}

pub async fn source_delete(
    State(state): State<AppState>,
    Path(source_id): Path<i64>,
) -> Result<impl IntoResponse, StatusCode> {
    // Look up channel_id before deleting so we can redirect back
    let srcs = sqlx::query_as::<_, crate::source::Source>(
        "SELECT * FROM sources WHERE id = ?",
    )
    .bind(source_id)
    .fetch_optional(&state.pool)
    .await
    .map_err(internal_error)?;

    let channel_id = srcs.map(|s| s.channel_id).unwrap_or(0);
    source::delete(&state.pool, source_id)
        .await
        .map_err(internal_error)?;
    Ok(Redirect::to(&format!("/admin/channels/{channel_id}")))
}

pub async fn source_toggle(
    State(state): State<AppState>,
    Path(source_id): Path<i64>,
) -> Result<impl IntoResponse, StatusCode> {
    let src = sqlx::query_as::<_, crate::source::Source>(
        "SELECT * FROM sources WHERE id = ?",
    )
    .bind(source_id)
    .fetch_optional(&state.pool)
    .await
    .map_err(internal_error)?
    .ok_or(StatusCode::NOT_FOUND)?;

    source::set_active(&state.pool, source_id, !src.is_active)
        .await
        .map_err(internal_error)?;
    Ok(Redirect::to(&format!("/admin/channels/{}", src.channel_id)))
}

pub async fn playlist_item_create(
    State(state): State<AppState>,
    Path(channel_id): Path<i64>,
    Form(form): Form<PlaylistItemForm>,
) -> Result<impl IntoResponse, StatusCode> {
    let duration_secs: i64 = form.duration_secs.trim().parse().unwrap_or(0);

    let existing = playlist_item::list_for_channel(&state.pool, channel_id)
        .await
        .map_err(internal_error)?;
    let sort_order = existing.len() as i64;

    playlist_item::create(
        &state.pool,
        playlist_item::NewPlaylistItem {
            channel_id,
            title: form.title.trim().to_string(),
            url: form.url.trim().to_string(),
            duration_secs,
            sort_order,
        },
    )
    .await
    .map_err(internal_error)?;
    Ok(Redirect::to(&format!("/admin/channels/{channel_id}")))
}

pub async fn playlist_item_delete(
    State(state): State<AppState>,
    Path(item_id): Path<i64>,
) -> Result<impl IntoResponse, StatusCode> {
    let item = sqlx::query_as::<_, crate::playlist_item::PlaylistItem>(
        "SELECT * FROM playlist_items WHERE id = ?",
    )
    .bind(item_id)
    .fetch_optional(&state.pool)
    .await
    .map_err(internal_error)?;

    let channel_id = item.map(|i| i.channel_id).unwrap_or(0);
    playlist_item::delete(&state.pool, item_id)
        .await
        .map_err(internal_error)?;
    Ok(Redirect::to(&format!("/admin/channels/{channel_id}")))
}
```

- [ ] **Step 6: Expand admin router in src/main.rs with source and playlist routes**

Replace the `admin_router` block in `src/main.rs` with:

```rust
    let admin_router: Router<AppState> = Router::new()
        .route("/", get(routes::admin::admin_index))
        .route("/channels", get(routes::admin::channel_list).post(routes::admin::channel_create))
        .route("/channels/new", get(routes::admin::channel_new_form))
        .route("/channels/:id", get(routes::admin::channel_detail).post(routes::admin::channel_update))
        .route("/channels/:id/edit", get(routes::admin::channel_edit_form))
        .route("/channels/:id/delete", post(routes::admin::channel_delete))
        .route("/channels/:id/sources", post(routes::admin::source_create))
        .route("/sources/:id/delete", post(routes::admin::source_delete))
        .route("/sources/:id/toggle", post(routes::admin::source_toggle))
        .route("/channels/:id/playlist", post(routes::admin::playlist_item_create))
        .route("/playlist/:id/delete", post(routes::admin::playlist_item_delete))
        .route_layer(middleware::from_fn_with_state(
            state.clone(),
            routes::admin::basic_auth,
        ));
```

- [ ] **Step 7: Build to confirm templates and routes compile**

```bash
cd /Users/kunwu/Workspace/playground/MyTV && cargo build 2>&1 | grep -E "^error"
```

Expected: no output.

- [ ] **Step 8: Run full test suite**

```bash
cd /Users/kunwu/Workspace/playground/MyTV && cargo test -- --test-threads=1 2>&1 | tail -5
```

Expected: all tests pass.

- [ ] **Step 9: Full smoke test**

```bash
cd /Users/kunwu/Workspace/playground/MyTV && DATABASE_URL=sqlite:mytv.db RUST_LOG=error cargo run &
sleep 3

# Channel detail page for existing channel 1
STATUS=$(curl -s -o /dev/null -w "%{http_code}" -u admin:admin http://localhost:3000/admin/channels/1)
echo "channel detail ch1: $STATUS"

# Channel detail for Big Buck Bunny (vod_loop — should show playlist section)
STATUS=$(curl -s -o /dev/null -w "%{http_code}" -u admin:admin http://localhost:3000/admin/channels/5)
echo "channel detail ch5 (vod): $STATUS"

# Add a source to channel 1
STATUS=$(curl -s -o /dev/null -w "%{http_code}" -u admin:admin \
  -d "kind=hls&url=https%3A%2F%2Fbackup.example.com%2Fstream.m3u8&priority=2" \
  http://localhost:3000/admin/channels/1/sources)
echo "add source: $STATUS"

# Add a playlist item to channel 5
STATUS=$(curl -s -o /dev/null -w "%{http_code}" -u admin:admin \
  -d "title=Big+Buck+Bunny+2&url=https%3A%2F%2Ftest-streams.mux.dev%2Fx36xhzz%2Fx36xhzz.m3u8&duration_secs=596" \
  http://localhost:3000/admin/channels/5/playlist)
echo "add playlist item: $STATUS"

kill %1 2>/dev/null || true
```

Expected:
```
channel detail ch1: 200
channel detail ch5 (vod): 200
add source: 302
add playlist item: 302
```

- [ ] **Step 10: Commit**

```bash
cd /Users/kunwu/Workspace/playground/MyTV && \
git add src/routes/admin.rs src/main.rs templates/admin/channel_detail.html && \
git commit -m "feat: add admin channel detail with sources and playlist items"
```

---

## Self-Review

**Spec coverage:**

| Requirement | Task |
|---|---|
| Password-protected `/admin` routes | ✅ Task 1 — HTTP Basic Auth middleware on all `/admin/*` |
| Channel list | ✅ Task 2 — `GET /admin/channels` |
| Create channel | ✅ Task 2 — `GET /admin/channels/new` + `POST /admin/channels` |
| Edit channel | ✅ Task 2 — `GET /admin/channels/:id/edit` + `POST /admin/channels/:id` |
| Delete channel | ✅ Task 2 — `POST /admin/channels/:id/delete` |
| Source CRUD per channel | ✅ Task 3 — add/delete/toggle |
| Playlist items CRUD per channel (vod_loop) | ✅ Task 3 — add/delete |
| POST/Redirect/GET after mutations | ✅ Tasks 2+3 — all handlers return `Redirect::to(...)` |
| Askama templates for all pages | ✅ Tasks 2+3 — channels.html, channel_form.html, channel_detail.html |

**Placeholder scan:** All steps have complete code. No TBDs.

**Type consistency:**
- `AdminChannelRow.type_str: String` — used in template as `ch.type_str.as_str() == "live"` ✓
- `ChannelDetailTemplate.channel_type: String` — used in template as `channel_type.as_str() == "vod_loop"` ✓
- `ChannelFormTemplate.channel_type: String` — used in template as `channel_type.as_str() == "live"` ✓
- `parse_loop_anchor(s: &str) -> Option<DateTime<Utc>>` — used in `channel_create` and `channel_update` ✓
- `render<T: askama::Template>(t: T) -> Result<Html<String>, StatusCode>` — used in all HTML-returning handlers ✓
- `internal_error<E>(e: E) -> StatusCode` — used in all `map_err` calls ✓

---

## Next Plans

- **Plan 5:** Discovery tools — YouTube Data API search, iptv-org M3U import, manual URL entry with auto duration fetch via yt-dlp
