# Behavior Tests Implementation Plan

> **For agentic workers:** REQUIRED SUB-SKILL: Use superpowers:subagent-driven-development (recommended) or superpowers:executing-plans to implement this plan task-by-task. Steps use checkbox (`- [ ]`) syntax for tracking.

**Goal:** Add 15 HTTP integration tests in `tests/http.rs` that verify route wiring, auth middleware, redirect middleware, and player HTTP contracts using `tower::ServiceExt::oneshot` against an in-memory SQLite DB.

**Architecture:** Extract `AppState` and the router into a new `src/lib.rs` so integration tests can import them. `src/main.rs` becomes a thin startup wrapper. Tests call `build_router(state).oneshot(request)` directly — no TCP socket, no port binding.

**Tech Stack:** Rust, Axum 0.7, tower `ServiceExt`, sqlx in-memory SQLite, serde_json

---

### Task 1: Infrastructure — lib.rs, seed fixture, dev deps

**Files:**
- Create: `src/lib.rs`
- Modify: `src/main.rs`
- Modify: `Cargo.toml`
- Create: `tests/fixtures/seed.sql`

---

- [ ] **Step 1: Create `src/lib.rs`**

Move all module declarations, `AppState`, `redirect_trailing_slash`, and the router assembly from `src/main.rs` into a new `src/lib.rs`. This makes them importable from `tests/http.rs`.

Create `src/lib.rs` with this exact content:

```rust
pub mod config;
pub mod db;
mod epg;
pub mod health;
mod media;
mod model;
mod routes;

use axum::{
    extract::Request,
    middleware::{self, Next},
    response::{IntoResponse, Redirect},
    routing::{get, post},
    Router,
};
use sqlx::SqlitePool;
use std::sync::Arc;

#[derive(Clone)]
pub struct AppState {
    pub pool: SqlitePool,
    pub config: Arc<config::Config>,
    pub http_client: reqwest::Client,
}

async fn redirect_trailing_slash(req: Request, next: Next) -> axum::response::Response {
    let path = req.uri().path();
    if path != "/" && path.ends_with('/') {
        let new_path = path.trim_end_matches('/');
        let location = match req.uri().query() {
            Some(q) => format!("{}?{}", new_path, q),
            None => new_path.to_string(),
        };
        return Redirect::permanent(&location).into_response();
    }
    next.run(req).await
}

pub fn build_router(state: AppState) -> Router {
    let admin_router: Router<AppState> = Router::new()
        .route("/", get(routes::admin::admin_index))
        .route(
            "/channels",
            get(routes::admin::channel_list).post(routes::admin::channel_create),
        )
        .route("/channels/new", get(routes::admin::channel_new_form))
        .route(
            "/channels/:id",
            get(routes::admin::channel_detail).post(routes::admin::channel_update),
        )
        .route("/channels/:id/edit", get(routes::admin::channel_edit_form))
        .route("/channels/:id/delete", post(routes::admin::channel_delete))
        .route("/channels/:id/sources", post(routes::admin::source_create))
        .route("/sources/:id/delete", post(routes::admin::source_delete))
        .route("/sources/:id/toggle", post(routes::admin::source_toggle))
        .route("/sources/:id/test", post(routes::admin::source_test))
        .route(
            "/channels/:id/playlist",
            post(routes::admin::playlist_item_create),
        )
        .route(
            "/playlist/:id/delete",
            post(routes::admin::playlist_item_delete),
        )
        .route("/discover", get(routes::admin::discover_page))
        .route("/discover/add-form", post(routes::admin::discover_add_form))
        .route("/discover/add", post(routes::admin::discover_add))
        .route(
            "/discover/m3u/search",
            post(routes::admin::discover_m3u_search),
        )
        .route(
            "/discover/youtube/search",
            post(routes::admin::discover_youtube_search),
        )
        .route(
            "/discover/manual/resolve",
            post(routes::admin::discover_manual_resolve),
        )
        .route_layer(middleware::from_fn_with_state(
            state.clone(),
            routes::admin::basic_auth,
        ));

    Router::new()
        .route("/", get(|| async { Redirect::permanent("/guide") }))
        .route("/health", get(routes::health::health_check))
        .route("/guide", get(routes::guide::guide_page))
        .route("/guide/partial", get(routes::guide::guide_partial))
        .route("/channel/:id/tune", get(routes::player::tune))
        .route("/channel/:id/next", get(routes::player::next))
        .route("/stream-proxy", get(routes::player::stream_proxy))
        .nest("/admin", admin_router)
        .layer(middleware::from_fn(redirect_trailing_slash))
        .with_state(state)
}
```

- [ ] **Step 2: Replace `src/main.rs`**

Replace the entire content of `src/main.rs` with the thin startup wrapper below. The module declarations and `AppState` are gone — they now live in `lib.rs`.

```rust
use anyhow::Result;
use mytv::{build_router, config, db, health, AppState};
use std::sync::Arc;
use tracing_subscriber::EnvFilter;

#[tokio::main]
async fn main() -> Result<()> {
    dotenvy::dotenv().ok();
    tracing_subscriber::fmt()
        .with_env_filter(EnvFilter::from_default_env())
        .init();

    let config = Arc::new(config::Config::from_env()?);
    let pool = db::connect(&config.database_url).await?;
    let http_client = reqwest::Client::builder()
        .timeout(std::time::Duration::from_secs(10))
        .build()?;

    let state = AppState {
        pool,
        config: config.clone(),
        http_client,
    };

    health::start(state.pool.clone(), state.http_client.clone());

    let app = build_router(state);
    let addr = format!("0.0.0.0:{}", config.port);
    tracing::info!("listening on {addr}");
    let listener = tokio::net::TcpListener::bind(&addr).await?;
    axum::serve(listener, app).await?;

    Ok(())
}
```

- [ ] **Step 3: Add dev dependencies to `Cargo.toml`**

Add a `[dev-dependencies]` section at the end of `Cargo.toml`:

```toml
[dev-dependencies]
tower = { version = "0.4", features = ["util"] }
http-body-util = "0.1"
```

- [ ] **Step 4: Create the seed fixture directory and file**

```bash
mkdir -p tests/fixtures
```

Create `tests/fixtures/seed.sql` with this exact content:

```sql
INSERT INTO channels (id, name, category, logo_url, type, sort_order, loop_anchor) VALUES
  (1, 'Live OK',       'test', NULL, 'live',     1, NULL),
  (2, 'All Down',      'test', NULL, 'live',     2, NULL),
  (3, 'Has Fallback',  'test', NULL, 'live',     3, NULL),
  (4, 'VOD Has Items', 'test', NULL, 'vod_loop', 4, '2020-01-01 00:00:00'),
  (5, 'VOD Empty',     'test', NULL, 'vod_loop', 5, '2020-01-01 00:00:00');

INSERT INTO sources (id, channel_id, kind, url, priority, is_active, consecutive_failures) VALUES
  (1, 1, 'hls', 'https://stream.example.com/live.m3u8',    1, 1, 0),
  (2, 2, 'hls', 'https://stream.example.com/down.m3u8',    1, 0, 3),
  (3, 3, 'hls', 'https://stream.example.com/primary.m3u8', 1, 0, 0),
  (4, 3, 'hls', 'https://stream.example.com/backup.m3u8',  2, 1, 0);

INSERT INTO playlist_items (channel_id, title, url, duration_secs, sort_order) VALUES
  (4, 'Episode 1', 'https://vod.example.com/ep1.mp4', 3600, 1),
  (4, 'Episode 2', 'https://vod.example.com/ep2.mp4', 3600, 2);
```

**Seed scenarios:**
- Channel 1 `Live OK`: one active HLS source → happy path tune
- Channel 2 `All Down`: one inactive source (3 failures) → tune returns 503
- Channel 3 `Has Fallback`: primary inactive, backup active → next with backup as failed_url returns 503
- Channel 4 `VOD Has Items`: two playlist items, loop_anchor set → tune returns 200
- Channel 5 `VOD Empty`: no playlist items → tune returns 503

- [ ] **Step 5: Verify it compiles and all existing tests still pass**

```bash
cargo fmt && cargo build 2>&1
```

Expected: `Finished` with no errors.

```bash
cargo test 2>&1 | tail -5
```

Expected: `test result: ok. 102 passed; 0 failed; 2 ignored`

- [ ] **Step 6: Commit**

```bash
git add src/lib.rs src/main.rs Cargo.toml Cargo.lock tests/fixtures/seed.sql
git commit -m "build: extract build_router into lib.rs for integration testing"
```

---

### Task 2: Auth, smoke, and redirect tests

**Files:**
- Create: `tests/http.rs`

---

- [ ] **Step 1: Create `tests/http.rs`** with helpers and 9 tests

Create `tests/http.rs` with this exact content:

```rust
use std::sync::Arc;

use axum::body::Body;
use axum::http::{Request, StatusCode};
use mytv::{build_router, config::Config, db, AppState};
use tower::ServiceExt;

async fn app() -> axum::Router {
    let pool = db::connect("sqlite::memory:").await.unwrap();
    sqlx::query(include_str!("fixtures/seed.sql"))
        .execute(&pool)
        .await
        .unwrap();
    let state = AppState {
        pool,
        config: Arc::new(Config {
            database_url: "sqlite::memory:".to_string(),
            admin_password: "test".to_string(),
            youtube_api_key: None,
            port: 0,
        }),
        http_client: reqwest::Client::new(),
    };
    build_router(state)
}

fn req(uri: &str) -> Request<Body> {
    Request::builder().uri(uri).body(Body::empty()).unwrap()
}

fn authed(uri: &str) -> Request<Body> {
    // "user:test" → base64 → "dXNlcjp0ZXN0"
    Request::builder()
        .uri(uri)
        .header("Authorization", "Basic dXNlcjp0ZXN0")
        .body(Body::empty())
        .unwrap()
}

// ── Auth middleware ───────────────────────────────────────────────────────────

#[tokio::test]
async fn test_admin_no_credentials_returns_401() {
    let response = app().await.oneshot(req("/admin/")).await.unwrap();
    assert_eq!(response.status(), StatusCode::UNAUTHORIZED);
}

#[tokio::test]
async fn test_admin_wrong_password_returns_401() {
    // "user:wrong" → base64 → "dXNlcjp3cm9uZw=="
    let r = Request::builder()
        .uri("/admin/")
        .header("Authorization", "Basic dXNlcjp3cm9uZw==")
        .body(Body::empty())
        .unwrap();
    let response = app().await.oneshot(r).await.unwrap();
    assert_eq!(response.status(), StatusCode::UNAUTHORIZED);
}

#[tokio::test]
async fn test_admin_correct_password_returns_200() {
    let response = app().await.oneshot(authed("/admin/")).await.unwrap();
    assert_eq!(response.status(), StatusCode::OK);
}

// ── Smoke tests ───────────────────────────────────────────────────────────────

#[tokio::test]
async fn test_health_returns_200() {
    let response = app().await.oneshot(req("/health")).await.unwrap();
    assert_eq!(response.status(), StatusCode::OK);
}

#[tokio::test]
async fn test_root_redirects_to_guide() {
    let response = app().await.oneshot(req("/")).await.unwrap();
    assert_eq!(response.status(), StatusCode::PERMANENT_REDIRECT);
    assert_eq!(response.headers().get("location").unwrap(), "/guide");
}

#[tokio::test]
async fn test_guide_returns_200() {
    let response = app().await.oneshot(req("/guide")).await.unwrap();
    assert_eq!(response.status(), StatusCode::OK);
}

#[tokio::test]
async fn test_guide_partial_returns_200() {
    let response = app().await.oneshot(req("/guide/partial")).await.unwrap();
    assert_eq!(response.status(), StatusCode::OK);
}

#[tokio::test]
async fn test_admin_channels_authed_returns_200() {
    let response = app().await.oneshot(authed("/admin/channels")).await.unwrap();
    assert_eq!(response.status(), StatusCode::OK);
}

// ── Redirect middleware ───────────────────────────────────────────────────────

#[tokio::test]
async fn test_trailing_slash_redirects() {
    // Redirect middleware fires before auth, so no credentials needed.
    let response = app()
        .await
        .oneshot(req("/admin/channels/"))
        .await
        .unwrap();
    assert_eq!(response.status(), StatusCode::PERMANENT_REDIRECT);
    assert_eq!(
        response.headers().get("location").unwrap(),
        "/admin/channels"
    );
}
```

- [ ] **Step 2: Run the tests to confirm all 9 pass**

```bash
cargo test --test http 2>&1
```

Expected:
```
running 9 tests
test test_admin_channels_authed_returns_200 ... ok
test test_admin_correct_password_returns_200 ... ok
test test_admin_no_credentials_returns_401 ... ok
test test_admin_wrong_password_returns_401 ... ok
test test_guide_partial_returns_200 ... ok
test test_guide_returns_200 ... ok
test test_health_returns_200 ... ok
test test_root_redirects_to_guide ... ok
test test_trailing_slash_redirects ... ok

test result: ok. 9 passed; 0 failed
```

- [ ] **Step 3: Commit**

```bash
cargo fmt
git add tests/http.rs
git commit -m "test: add HTTP behavior tests for auth, smoke routes, and redirect middleware"
```

---

### Task 3: Player contract tests

**Files:**
- Modify: `tests/http.rs`

---

- [ ] **Step 1: Add the `body_json` helper and 6 player tests to `tests/http.rs`**

Add the following directly after the `authed` helper function (after line `}`), before the auth section comment:

```rust
async fn body_json(response: axum::response::Response) -> serde_json::Value {
    use http_body_util::BodyExt;
    let bytes = response.into_body().collect().await.unwrap().to_bytes();
    serde_json::from_slice(&bytes).unwrap()
}
```

Then append these 6 tests at the end of `tests/http.rs`, after the redirect middleware test:

```rust
// ── Player contract tests ─────────────────────────────────────────────────────

#[tokio::test]
async fn test_tune_live_ok_returns_stream_url() {
    let response = app().await.oneshot(req("/channel/1/tune")).await.unwrap();
    assert_eq!(response.status(), StatusCode::OK);
    let json = body_json(response).await;
    assert_eq!(
        json["url"].as_str().unwrap(),
        "https://stream.example.com/live.m3u8"
    );
}

#[tokio::test]
async fn test_tune_live_all_sources_down_returns_503() {
    let response = app().await.oneshot(req("/channel/2/tune")).await.unwrap();
    assert_eq!(response.status(), StatusCode::SERVICE_UNAVAILABLE);
}

#[tokio::test]
async fn test_next_live_returns_backup_when_no_failed_url() {
    let response = app()
        .await
        .oneshot(req("/channel/3/next"))
        .await
        .unwrap();
    assert_eq!(response.status(), StatusCode::OK);
    let json = body_json(response).await;
    assert_eq!(
        json["url"].as_str().unwrap(),
        "https://stream.example.com/backup.m3u8"
    );
}

#[tokio::test]
async fn test_next_live_all_sources_failed_returns_503() {
    // backup is the only active source; passing it as failed_url leaves nothing
    let response = app()
        .await
        .oneshot(req(
            "/channel/3/next?failed_url=https%3A%2F%2Fstream.example.com%2Fbackup.m3u8",
        ))
        .await
        .unwrap();
    assert_eq!(response.status(), StatusCode::SERVICE_UNAVAILABLE);
}

#[tokio::test]
async fn test_tune_vod_with_playlist_returns_stream_url() {
    let response = app().await.oneshot(req("/channel/4/tune")).await.unwrap();
    assert_eq!(response.status(), StatusCode::OK);
    let json = body_json(response).await;
    // exact episode depends on Utc::now() — assert URL is from the playlist
    assert!(json["url"]
        .as_str()
        .unwrap()
        .contains("vod.example.com/ep"));
}

#[tokio::test]
async fn test_tune_vod_empty_playlist_returns_503() {
    let response = app().await.oneshot(req("/channel/5/tune")).await.unwrap();
    assert_eq!(response.status(), StatusCode::SERVICE_UNAVAILABLE);
}
```

- [ ] **Step 2: Run all HTTP tests to confirm all 15 pass**

```bash
cargo test --test http 2>&1
```

Expected:
```
running 15 tests
test test_admin_channels_authed_returns_200 ... ok
test test_admin_correct_password_returns_200 ... ok
test test_admin_no_credentials_returns_401 ... ok
test test_admin_wrong_password_returns_401 ... ok
test test_guide_partial_returns_200 ... ok
test test_guide_returns_200 ... ok
test test_health_returns_200 ... ok
test test_next_live_all_sources_failed_returns_503 ... ok
test test_next_live_returns_backup_when_no_failed_url ... ok
test test_root_redirects_to_guide ... ok
test test_trailing_slash_redirects ... ok
test test_tune_live_all_sources_down_returns_503 ... ok
test test_tune_live_ok_returns_stream_url ... ok
test test_tune_vod_empty_playlist_returns_503 ... ok
test test_tune_vod_with_playlist_returns_stream_url ... ok

test result: ok. 15 passed; 0 failed
```

- [ ] **Step 3: Run the full suite to confirm nothing regressed**

```bash
cargo test 2>&1 | tail -5
```

Expected: `test result: ok. 117 passed; 0 failed; 2 ignored` (102 existing + 15 new)

- [ ] **Step 4: Commit**

```bash
cargo fmt
git add tests/http.rs
git commit -m "test: add player HTTP contract tests"
```
