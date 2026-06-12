# Admin Automation (JSON API + mytvctl CLI) Implementation Plan

> **For agentic workers:** REQUIRED SUB-SKILL: Use superpowers:subagent-driven-development (recommended) or superpowers:executing-plans to implement this plan task-by-task. Steps use checkbox (`- [ ]`) syntax for tracking.

**Goal:** Add a JSON `/api/admin` API (full CRUD incl. edit + test for channels, sources, playlist items) and a `mytvctl` CLI client that drives a remote instance over HTTP.

**Architecture:** A new `src/routes/api/` module of thin JSON handlers reusing the existing `model::*` layer, mounted under `/api/admin` behind the existing `basic_auth` route-layer. Responses serialize the existing `Channel`/`Source`/`PlaylistItem` structs; requests use small string-friendly DTOs that convert to model types. A separate `src/bin/mytvctl.rs` binary maps CLI args to HTTP requests and prints the raw JSON response.

**Tech Stack:** Rust, Axum 0.7, SQLx (SQLite), serde/serde_json, reqwest (async, already a dep), clap (new dep), `tower::ServiceExt::oneshot` integration tests.

**Spec:** `docs/superpowers/specs/2026-06-12-admin-automation-design.md`

---

## File Structure

- `src/model/source.rs` — Task 1: add `UpdateSource` + `update()`.
- `src/model/playlist_item.rs` — Task 1: add `UpdatePlaylistItem` + `update()`.
- `src/routes/api/mod.rs` — Task 2: `ApiError`, request DTOs, `api_router()`.
- `src/routes/api/channels.rs` — Task 3: channel handlers.
- `src/routes/api/sources.rs` — Task 4: source handlers.
- `src/routes/api/playlist.rs` — Task 5: playlist-item handlers.
- `src/routes/mod.rs` — Task 2: declare `pub mod api;`.
- `src/routes/admin/channels.rs` — Task 3: make `parse_loop_anchor` `pub(crate)`.
- `src/lib.rs` — Task 2: mount `/api/admin`; import `patch`/`delete` routing fns.
- `src/bin/mytvctl.rs` — Task 6: the CLI binary (with inline unit tests).
- `Cargo.toml` — Task 6: add `clap`.
- `tests/api.rs` — Tasks 2-5: integration tests (new file).

Reference facts (verified against current code):
- Model structs `Channel`, `Source`, `PlaylistItem` already `#[derive(Serialize, Deserialize, FromRow)]`.
- `channel::update(pool, id, UpdateChannel) -> Result<Option<Channel>>` is the pattern to mirror.
- `SourceKind::from_str` accepts `hls|youtube_live|youtube_vod|iptv|dash`; `SourceKind::detect(url)` returns `YoutubeLive` for youtube URLs (`as_str()` = `"youtube_live"`), `Dash` for `.mpd`, `Hls` for `.m3u8`, else `Iptv`.
- `channel::ChannelType::from_str` accepts `live|vod_loop`, errors otherwise.
- Probes: `health::probe_source(&pool, &http_client, &cors_cache, &live_cache, &src).await` and `health::probe_playlist_item(&pool, &http_client, &cors_cache, &item).await`.
- Test harness in `tests/http.rs`: `app()` builds a router on an in-memory DB seeded from `tests/fixtures/seed.sql`; auth header for `user:test` is `Basic dXNlcjp0ZXN0`. Seed: channel 1 (live, source id 1 = `https://stream.example.com/live.m3u8`), channel 4 (vod_loop, two active items), channel 5 (vod_loop, empty).

---

## Task 1: Model `update` functions for sources & playlist items

**Files:**
- Modify: `src/model/source.rs` (add after `set_active`, ~line 169)
- Modify: `src/model/playlist_item.rs` (add after `set_active`, ~line 115)

- [ ] **Step 1: Write failing unit tests**

Append to the existing `#[cfg(test)] mod tests` in `src/model/source.rs` (if none exists, add one at end of file). Use the same in-memory pool pattern other model tests use — check the bottom of the file; if there is a `tests` module, mirror its setup helper. If there is NO existing tests module in this file, add this complete module:

```rust
#[cfg(test)]
mod update_tests {
    use super::*;

    async fn pool() -> SqlitePool {
        let pool = crate::db::connect("sqlite::memory:").await.unwrap();
        sqlx::query(include_str!("../../tests/fixtures/seed.sql"))
            .execute(&pool)
            .await
            .unwrap();
        pool
    }

    #[tokio::test]
    async fn update_changes_url_and_priority() {
        let pool = pool().await;
        let updated = update(
            &pool,
            1,
            UpdateSource { url: "https://new.example.com/x.m3u8".into(), priority: 9 },
        )
        .await
        .unwrap()
        .expect("source 1 exists");
        assert_eq!(updated.url, "https://new.example.com/x.m3u8");
        assert_eq!(updated.priority, 9);
    }

    #[tokio::test]
    async fn update_unknown_id_returns_none() {
        let pool = pool().await;
        let r = update(
            &pool,
            999999,
            UpdateSource { url: "https://x".into(), priority: 1 },
        )
        .await
        .unwrap();
        assert!(r.is_none());
    }
}
```

NOTE: confirm `crate::db::connect` is the connection helper (per CLAUDE.md `db::connect()` runs migrations). If model tests in this crate use a different setup, mirror that instead.

- [ ] **Step 2: Run to verify it fails**

Run: `cargo test --lib source::update_tests`
Expected: FAIL — `UpdateSource` / `update` not defined.

- [ ] **Step 3: Implement `UpdateSource` + `update` in `src/model/source.rs`**

Add after `set_active` (after ~line 169):

```rust
/// Input for updating an existing source (editable fields only).
pub struct UpdateSource {
    pub url: String,
    pub priority: i64,
}

/// Update a source's url/priority by id; returns None if not found.
pub async fn update(pool: &SqlitePool, id: i64, input: UpdateSource) -> Result<Option<Source>> {
    let rows = sqlx::query("UPDATE sources SET url = ?, priority = ? WHERE id = ?")
        .bind(&input.url)
        .bind(input.priority)
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

- [ ] **Step 4: Write failing unit tests for playlist_item**

Append to `src/model/playlist_item.rs` (add a tests module if none exists):

```rust
#[cfg(test)]
mod update_tests {
    use super::*;

    async fn pool() -> SqlitePool {
        let pool = crate::db::connect("sqlite::memory:").await.unwrap();
        sqlx::query(include_str!("../../tests/fixtures/seed.sql"))
            .execute(&pool)
            .await
            .unwrap();
        pool
    }

    #[tokio::test]
    async fn update_changes_fields() {
        let pool = pool().await;
        // seed inserts items for channel 4; id 1 is the first ("Episode 1").
        let updated = update(
            &pool,
            1,
            UpdatePlaylistItem {
                title: "Renamed".into(),
                url: "https://vod.example.com/new.mp4".into(),
                duration_secs: 123,
                sort_order: 7,
            },
        )
        .await
        .unwrap()
        .expect("item 1 exists");
        assert_eq!(updated.title, "Renamed");
        assert_eq!(updated.url, "https://vod.example.com/new.mp4");
        assert_eq!(updated.duration_secs, 123);
        assert_eq!(updated.sort_order, 7);
    }

    #[tokio::test]
    async fn update_unknown_id_returns_none() {
        let pool = pool().await;
        let r = update(
            &pool,
            999999,
            UpdatePlaylistItem {
                title: "x".into(),
                url: "y".into(),
                duration_secs: 1,
                sort_order: 1,
            },
        )
        .await
        .unwrap();
        assert!(r.is_none());
    }
}
```

- [ ] **Step 5: Run to verify it fails**

Run: `cargo test --lib playlist_item::update_tests`
Expected: FAIL — `UpdatePlaylistItem` / `update` not defined.

- [ ] **Step 6: Implement `UpdatePlaylistItem` + `update` in `src/model/playlist_item.rs`**

Add after `set_active` (~line 115):

```rust
/// Input for updating an existing playlist item.
pub struct UpdatePlaylistItem {
    pub title: String,
    pub url: String,
    pub duration_secs: i64,
    pub sort_order: i64,
}

/// Update a playlist item by id; returns None if not found.
pub async fn update(
    pool: &SqlitePool,
    id: i64,
    input: UpdatePlaylistItem,
) -> Result<Option<PlaylistItem>> {
    let rows = sqlx::query(
        "UPDATE playlist_items SET title = ?, url = ?, duration_secs = ?, sort_order = ? WHERE id = ?",
    )
    .bind(&input.title)
    .bind(&input.url)
    .bind(input.duration_secs)
    .bind(input.sort_order)
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

- [ ] **Step 7: Run tests to verify they pass**

Run: `cargo test --lib source::update_tests playlist_item::update_tests`
Expected: PASS (4 tests).

- [ ] **Step 8: Format, lint, commit**

```bash
cargo fmt
cargo clippy -- -D warnings
git add src/model/source.rs src/model/playlist_item.rs
git commit -m "feat: add source::update and playlist_item::update model fns"
```

Expected: clippy exits 0.

---

## Task 2: API scaffolding — `ApiError`, router, mount, first endpoint

**Files:**
- Create: `src/routes/api/mod.rs`
- Create: `src/routes/api/channels.rs` (list handler only for now)
- Modify: `src/routes/mod.rs` (add `pub mod api;`)
- Modify: `src/lib.rs` (import `patch`,`delete`; mount `/api/admin`)
- Create: `tests/api.rs`

- [ ] **Step 1: Write the failing tests** — create `tests/api.rs`:

```rust
use http_body_util::BodyExt;
use mytv::{build_router, config::Config, metrics, AppState};
use std::sync::Arc;
use tower::ServiceExt;

use axum::{
    body::Body,
    http::{Request, StatusCode},
};

async fn app() -> axum::Router {
    let pool = mytv::db::connect("sqlite::memory:").await.unwrap();
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
        http_client: reqwest::Client::builder()
            .connect_timeout(std::time::Duration::from_secs(1))
            .timeout(std::time::Duration::from_secs(2))
            .build()
            .unwrap(),
        proxy_client: reqwest::Client::builder()
            .redirect(reqwest::redirect::Policy::none())
            .build()
            .unwrap(),
        cors_cache: Arc::new(tokio::sync::RwLock::new(std::collections::HashMap::new())),
        ssrf_cache: Arc::new(tokio::sync::RwLock::new(std::collections::HashMap::new())),
        metrics: Arc::new(metrics::Metrics::new()),
        live_cache: Arc::new(tokio::sync::RwLock::new(std::collections::HashMap::new())),
    };
    build_router(state)
}

// Basic auth header for user:test → base64("user:test") = dXNlcjp0ZXN0
const AUTH: &str = "Basic dXNlcjp0ZXN0";

fn get(uri: &str) -> Request<Body> {
    Request::builder().uri(uri).body(Body::empty()).unwrap()
}

fn authed_get(uri: &str) -> Request<Body> {
    Request::builder()
        .uri(uri)
        .header("Authorization", AUTH)
        .body(Body::empty())
        .unwrap()
}

fn authed_json(method: &str, uri: &str, body: serde_json::Value) -> Request<Body> {
    Request::builder()
        .method(method)
        .uri(uri)
        .header("Authorization", AUTH)
        .header("content-type", "application/json")
        .body(Body::from(body.to_string()))
        .unwrap()
}

async fn body_json(response: axum::response::Response) -> serde_json::Value {
    let bytes = response.into_body().collect().await.unwrap().to_bytes();
    serde_json::from_slice(&bytes).unwrap()
}

#[tokio::test]
async fn channels_list_requires_auth() {
    let r = app().await.oneshot(get("/api/admin/channels")).await.unwrap();
    assert_eq!(r.status(), StatusCode::UNAUTHORIZED);
}

#[tokio::test]
async fn channels_list_returns_seeded_channels() {
    let r = app()
        .await
        .oneshot(authed_get("/api/admin/channels"))
        .await
        .unwrap();
    assert_eq!(r.status(), StatusCode::OK);
    let json = body_json(r).await;
    let arr = json.as_array().expect("array");
    assert!(arr.len() >= 5);
    assert!(arr.iter().any(|c| c["name"] == "Live OK"));
}
```

NOTE: confirm the `AppState` field list against `src/lib.rs` (the struct definition) before running — if a field name differs, mirror reality. The `tests/http.rs` `app_for_network()` builder (lines ~51-78) is the authoritative shape; copy it.

- [ ] **Step 2: Run to verify it fails**

Run: `cargo test --test api`
Expected: FAIL to compile (no `/api/admin` route; `mytv::db` maybe needs `pub`). If `mytv::db`/`config` aren't public, that's expected — fix in Step 3/wiring.

- [ ] **Step 3: Create `src/routes/api/mod.rs`**

```rust
//! JSON admin API under /api/admin. Thin handlers over the model layer,
//! sharing the form admin's basic_auth. Responses serialize model structs;
//! requests use the DTOs below.

mod channels;

use axum::{
    http::StatusCode,
    response::{IntoResponse, Json, Response},
    routing::get,
    Router,
};
use serde::Serialize;

use crate::AppState;

/// Unified JSON error: renders `{ "error": "<msg>" }` with a status code.
pub enum ApiError {
    NotFound,
    Validation(String),
    Internal,
}

impl IntoResponse for ApiError {
    fn into_response(self) -> Response {
        let (status, msg) = match self {
            ApiError::NotFound => (StatusCode::NOT_FOUND, "not found".to_string()),
            ApiError::Validation(m) => (StatusCode::UNPROCESSABLE_ENTITY, m),
            ApiError::Internal => (
                StatusCode::INTERNAL_SERVER_ERROR,
                "internal error".to_string(),
            ),
        };
        #[derive(Serialize)]
        struct ErrorBody {
            error: String,
        }
        (status, Json(ErrorBody { error: msg })).into_response()
    }
}

/// Map any model/db error to a logged 500.
pub(crate) fn internal<E: std::fmt::Display>(e: E) -> ApiError {
    tracing::error!("api error: {e}");
    ApiError::Internal
}

pub fn api_router() -> Router<AppState> {
    Router::new().route("/channels", get(channels::list))
}
```

- [ ] **Step 4: Create `src/routes/api/channels.rs` (list only)**

```rust
use axum::{extract::State, response::Json};

use super::{internal, ApiError};
use crate::{model::channel, AppState};

pub async fn list(
    State(state): State<AppState>,
) -> Result<Json<Vec<channel::Channel>>, ApiError> {
    let channels = channel::list(&state.pool).await.map_err(internal)?;
    Ok(Json(channels))
}
```

- [ ] **Step 5: Declare the module — in `src/routes/mod.rs` add after `pub mod admin;`:**

```rust
pub mod api;
```

- [ ] **Step 6: Mount the router in `src/lib.rs`**

Change the routing import (line 17) from:
```rust
    routing::{get, post},
```
to:
```rust
    routing::{delete, get, patch, post},
```

Add the API router behind the same auth, right after the `admin_router` definition block (after line 117, before the top-level `Router::new()`):

```rust
    let api_router: Router<AppState> = routes::api::api_router().route_layer(
        middleware::from_fn_with_state(state.clone(), routes::admin::basic_auth),
    );
```

Then nest it in the top-level router, right after `.nest("/admin", admin_router)` (line 130):

```rust
        .nest("/api/admin", api_router)
```

If `db` / `config` are not already `pub` in `lib.rs` (needed by `tests/api.rs`), confirm: `pub mod db;` and `pub mod config;` already exist (per lib.rs lines 2-3). `mytv::metrics`, `mytv::AppState`, `mytv::build_router` are public. `Config` fields are public.

- [ ] **Step 7: Run tests to verify they pass**

Run: `cargo test --test api`
Expected: PASS (2 tests). Also run `cargo test --test http` to confirm no regression to the form admin.

- [ ] **Step 8: Format, lint, commit**

```bash
cargo fmt
cargo clippy -- -D warnings
git add src/routes/api/mod.rs src/routes/api/channels.rs src/routes/mod.rs src/lib.rs tests/api.rs
git commit -m "feat: scaffold /api/admin JSON router with ApiError and channels list"
```

---

## Task 3: Channel endpoints (create/get/update/delete)

**Files:**
- Modify: `src/routes/api/channels.rs` (add handlers + DTOs)
- Modify: `src/routes/api/mod.rs` (add routes)
- Modify: `src/routes/admin/channels.rs` (make `parse_loop_anchor` `pub(crate)`)
- Modify: `tests/api.rs` (add tests)

- [ ] **Step 1: Write the failing tests** — append to `tests/api.rs`:

```rust
#[tokio::test]
async fn channel_crud_round_trip() {
    let app = app().await;

    // create
    let r = app
        .clone()
        .oneshot(authed_json(
            "POST",
            "/api/admin/channels",
            serde_json::json!({
                "name": "API Made",
                "category": "test",
                "type": "live",
                "sort_order": 99
            }),
        ))
        .await
        .unwrap();
    assert_eq!(r.status(), StatusCode::CREATED);
    let created = body_json(r).await;
    let id = created["id"].as_i64().unwrap();
    assert_eq!(created["name"], "API Made");
    assert_eq!(created["channel_type"], "live");

    // get
    let r = app
        .clone()
        .oneshot(authed_get(&format!("/api/admin/channels/{id}")))
        .await
        .unwrap();
    assert_eq!(r.status(), StatusCode::OK);
    assert_eq!(body_json(r).await["name"], "API Made");

    // update
    let r = app
        .clone()
        .oneshot(authed_json(
            "PATCH",
            &format!("/api/admin/channels/{id}"),
            serde_json::json!({
                "name": "API Renamed",
                "category": "test",
                "type": "live",
                "sort_order": 1
            }),
        ))
        .await
        .unwrap();
    assert_eq!(r.status(), StatusCode::OK);
    assert_eq!(body_json(r).await["name"], "API Renamed");

    // delete
    let r = app
        .clone()
        .oneshot(
            Request::builder()
                .method("DELETE")
                .uri(format!("/api/admin/channels/{id}"))
                .header("Authorization", AUTH)
                .body(Body::empty())
                .unwrap(),
        )
        .await
        .unwrap();
    assert_eq!(r.status(), StatusCode::NO_CONTENT);

    // get-after-delete → 404
    let r = app
        .oneshot(authed_get(&format!("/api/admin/channels/{id}")))
        .await
        .unwrap();
    assert_eq!(r.status(), StatusCode::NOT_FOUND);
}

#[tokio::test]
async fn channel_create_bad_type_is_422() {
    let r = app()
        .await
        .oneshot(authed_json(
            "POST",
            "/api/admin/channels",
            serde_json::json!({"name":"x","category":"y","type":"nonsense","sort_order":0}),
        ))
        .await
        .unwrap();
    assert_eq!(r.status(), StatusCode::UNPROCESSABLE_ENTITY);
}

#[tokio::test]
async fn channel_get_unknown_is_404() {
    let r = app()
        .await
        .oneshot(authed_get("/api/admin/channels/999999"))
        .await
        .unwrap();
    assert_eq!(r.status(), StatusCode::NOT_FOUND);
}
```

- [ ] **Step 2: Run to verify it fails**

Run: `cargo test --test api channel_crud_round_trip channel_create_bad_type_is_422 channel_get_unknown_is_404`
Expected: FAIL (routes 404 / method not allowed).

- [ ] **Step 3: Make `parse_loop_anchor` shareable** — in `src/routes/admin/channels.rs`, change its signature (line 73) from:
```rust
fn parse_loop_anchor(s: &str) -> Option<DateTime<Utc>> {
```
to:
```rust
pub(crate) fn parse_loop_anchor(s: &str) -> Option<DateTime<Utc>> {
```

- [ ] **Step 4: Add DTOs + handlers to `src/routes/api/channels.rs`**

Replace the file with:

```rust
use axum::{
    extract::{Path, State},
    http::StatusCode,
    response::Json,
};
use chrono::Utc;
use serde::Deserialize;

use super::{internal, ApiError};
use crate::routes::admin::channels::parse_loop_anchor;
use crate::{model::channel, AppState};

#[derive(Deserialize)]
pub struct CreateChannelRequest {
    pub name: String,
    pub category: String,
    pub logo_url: Option<String>,
    #[serde(rename = "type")]
    pub channel_type: String,
    pub sort_order: i64,
    pub loop_anchor: Option<String>,
}

#[derive(Deserialize)]
pub struct UpdateChannelRequest {
    pub name: String,
    pub category: String,
    pub logo_url: Option<String>,
    #[serde(rename = "type")]
    pub channel_type: String,
    pub sort_order: i64,
    pub loop_anchor: Option<String>,
}

fn parse_type(s: &str) -> Result<channel::ChannelType, ApiError> {
    s.parse::<channel::ChannelType>()
        .map_err(|_| ApiError::Validation(format!("invalid channel type: {s}")))
}

/// For a vod_loop channel, resolve the loop anchor: parse the given string,
/// or default to now (matching the form handler's behavior); live → None.
fn resolve_anchor(channel_type: channel::ChannelType, raw: Option<&str>) -> Option<chrono::DateTime<Utc>> {
    if channel_type == channel::ChannelType::VodLoop {
        raw.and_then(parse_loop_anchor).or_else(|| Some(Utc::now()))
    } else {
        None
    }
}

pub async fn list(State(state): State<AppState>) -> Result<Json<Vec<channel::Channel>>, ApiError> {
    Ok(Json(channel::list(&state.pool).await.map_err(internal)?))
}

pub async fn get_one(
    State(state): State<AppState>,
    Path(id): Path<i64>,
) -> Result<Json<channel::Channel>, ApiError> {
    let ch = channel::get(&state.pool, id)
        .await
        .map_err(internal)?
        .ok_or(ApiError::NotFound)?;
    Ok(Json(ch))
}

pub async fn create(
    State(state): State<AppState>,
    Json(req): Json<CreateChannelRequest>,
) -> Result<(StatusCode, Json<channel::Channel>), ApiError> {
    if req.name.trim().is_empty() || req.category.trim().is_empty() {
        return Err(ApiError::Validation("name and category are required".into()));
    }
    let channel_type = parse_type(&req.channel_type)?;
    let loop_anchor = resolve_anchor(channel_type, req.loop_anchor.as_deref());
    let ch = channel::create(
        &state.pool,
        channel::NewChannel {
            name: req.name.trim().to_string(),
            category: req.category.trim().to_string(),
            logo_url: req.logo_url,
            channel_type,
            sort_order: req.sort_order,
            loop_anchor,
        },
    )
    .await
    .map_err(internal)?;
    Ok((StatusCode::CREATED, Json(ch)))
}

pub async fn update(
    State(state): State<AppState>,
    Path(id): Path<i64>,
    Json(req): Json<UpdateChannelRequest>,
) -> Result<Json<channel::Channel>, ApiError> {
    if req.name.trim().is_empty() || req.category.trim().is_empty() {
        return Err(ApiError::Validation("name and category are required".into()));
    }
    let channel_type = parse_type(&req.channel_type)?;
    let loop_anchor = resolve_anchor(channel_type, req.loop_anchor.as_deref());
    let ch = channel::update(
        &state.pool,
        id,
        channel::UpdateChannel {
            name: req.name.trim().to_string(),
            category: req.category.trim().to_string(),
            logo_url: req.logo_url,
            channel_type,
            sort_order: req.sort_order,
            loop_anchor,
        },
    )
    .await
    .map_err(internal)?
    .ok_or(ApiError::NotFound)?;
    Ok(Json(ch))
}

pub async fn remove(
    State(state): State<AppState>,
    Path(id): Path<i64>,
) -> Result<StatusCode, ApiError> {
    let deleted = channel::delete(&state.pool, id).await.map_err(internal)?;
    if deleted {
        Ok(StatusCode::NO_CONTENT)
    } else {
        Err(ApiError::NotFound)
    }
}
```

- [ ] **Step 5: Wire the routes in `src/routes/api/mod.rs`**

Update imports and `api_router`:

```rust
use axum::{
    http::StatusCode,
    response::{IntoResponse, Json, Response},
    routing::{get, post},
    Router,
};
```

```rust
pub fn api_router() -> Router<AppState> {
    Router::new()
        .route("/channels", get(channels::list).post(channels::create))
        .route(
            "/channels/:id",
            get(channels::get_one)
                .patch(channels::update)
                .delete(channels::remove),
        )
}
```

(`post`/`patch`/`delete` here are method-builder calls on the `MethodRouter` returned by `get(...)`, so only `get` and `post` need importing as free functions.)

- [ ] **Step 6: Run tests to verify they pass**

Run: `cargo test --test api`
Expected: PASS (all channel tests + the two from Task 2).

- [ ] **Step 7: Format, lint, commit**

```bash
cargo fmt
cargo clippy -- -D warnings
git add src/routes/api/channels.rs src/routes/api/mod.rs src/routes/admin/channels.rs tests/api.rs
git commit -m "feat: channel CRUD endpoints on /api/admin"
```

---

## Task 4: Source endpoints (list/create/get/update/delete/toggle/test)

**Files:**
- Create: `src/routes/api/sources.rs`
- Modify: `src/routes/api/mod.rs` (declare `mod sources;`, add routes)
- Modify: `tests/api.rs` (add tests)

- [ ] **Step 1: Write the failing tests** — append to `tests/api.rs`:

```rust
#[tokio::test]
async fn source_crud_and_kind_autodetect() {
    let app = app().await;

    // create on channel 1, no kind → auto-detect from youtube URL
    let r = app
        .clone()
        .oneshot(authed_json(
            "POST",
            "/api/admin/channels/1/sources",
            serde_json::json!({"url": "https://www.youtube.com/watch?v=abc"}),
        ))
        .await
        .unwrap();
    assert_eq!(r.status(), StatusCode::CREATED);
    let created = body_json(r).await;
    let id = created["id"].as_i64().unwrap();
    assert_eq!(created["kind"], "youtube_live"); // SourceKind::detect → YoutubeLive
    assert_eq!(created["channel_id"], 1);

    // list for channel 1 contains it
    let r = app
        .clone()
        .oneshot(authed_get("/api/admin/channels/1/sources"))
        .await
        .unwrap();
    assert_eq!(r.status(), StatusCode::OK);
    let arr = body_json(r).await;
    assert!(arr.as_array().unwrap().iter().any(|s| s["id"] == id));

    // update url + priority
    let r = app
        .clone()
        .oneshot(authed_json(
            "PATCH",
            &format!("/api/admin/sources/{id}"),
            serde_json::json!({"url":"https://x.example.com/y.m3u8","priority":5}),
        ))
        .await
        .unwrap();
    assert_eq!(r.status(), StatusCode::OK);
    let updated = body_json(r).await;
    assert_eq!(updated["url"], "https://x.example.com/y.m3u8");
    assert_eq!(updated["priority"], 5);

    // toggle inactive
    let r = app
        .clone()
        .oneshot(authed_json(
            "POST",
            &format!("/api/admin/sources/{id}/toggle"),
            serde_json::json!({"active": false}),
        ))
        .await
        .unwrap();
    assert_eq!(r.status(), StatusCode::OK);
    assert_eq!(body_json(r).await["is_active"], false);

    // delete
    let r = app
        .clone()
        .oneshot(
            Request::builder()
                .method("DELETE")
                .uri(format!("/api/admin/sources/{id}"))
                .header("Authorization", AUTH)
                .body(Body::empty())
                .unwrap(),
        )
        .await
        .unwrap();
    assert_eq!(r.status(), StatusCode::NO_CONTENT);
}

#[tokio::test]
async fn source_create_empty_url_is_422() {
    let r = app()
        .await
        .oneshot(authed_json(
            "POST",
            "/api/admin/channels/1/sources",
            serde_json::json!({"url": "   "}),
        ))
        .await
        .unwrap();
    assert_eq!(r.status(), StatusCode::UNPROCESSABLE_ENTITY);
}

#[tokio::test]
async fn source_test_populates_last_checked() {
    // seed source 1 is an unreachable HLS URL; probe records a result either way.
    let r = app()
        .await
        .oneshot(
            Request::builder()
                .method("POST")
                .uri("/api/admin/sources/1/test")
                .header("Authorization", AUTH)
                .body(Body::empty())
                .unwrap(),
        )
        .await
        .unwrap();
    assert_eq!(r.status(), StatusCode::OK);
    let json = body_json(r).await;
    assert_eq!(json["id"], 1);
    assert!(!json["last_checked_at"].is_null());
}

#[tokio::test]
async fn source_update_unknown_is_404() {
    let r = app()
        .await
        .oneshot(authed_json(
            "PATCH",
            "/api/admin/sources/999999",
            serde_json::json!({"url":"https://x","priority":1}),
        ))
        .await
        .unwrap();
    assert_eq!(r.status(), StatusCode::NOT_FOUND);
}
```

- [ ] **Step 2: Run to verify it fails**

Run: `cargo test --test api source_crud_and_kind_autodetect source_create_empty_url_is_422 source_test_populates_last_checked source_update_unknown_is_404`
Expected: FAIL (routes not present).

- [ ] **Step 3: Create `src/routes/api/sources.rs`**

```rust
use axum::{
    extract::{Path, State},
    http::StatusCode,
    response::Json,
};
use serde::Deserialize;
use std::str::FromStr;

use super::{internal, ApiError};
use crate::{model::source, AppState};

#[derive(Deserialize)]
pub struct CreateSourceRequest {
    pub url: String,
    pub priority: Option<i64>,
    pub kind: Option<String>,
}

#[derive(Deserialize)]
pub struct UpdateSourceRequest {
    pub url: String,
    pub priority: i64,
}

#[derive(Deserialize)]
pub struct ToggleRequest {
    pub active: bool,
}

pub async fn list_for_channel(
    State(state): State<AppState>,
    Path(channel_id): Path<i64>,
) -> Result<Json<Vec<source::Source>>, ApiError> {
    let sources = source::list_for_channel(&state.pool, channel_id)
        .await
        .map_err(internal)?;
    Ok(Json(sources))
}

pub async fn get_one(
    State(state): State<AppState>,
    Path(id): Path<i64>,
) -> Result<Json<source::Source>, ApiError> {
    let src = source::get(&state.pool, id)
        .await
        .map_err(internal)?
        .ok_or(ApiError::NotFound)?;
    Ok(Json(src))
}

pub async fn create(
    State(state): State<AppState>,
    Path(channel_id): Path<i64>,
    Json(req): Json<CreateSourceRequest>,
) -> Result<(StatusCode, Json<source::Source>), ApiError> {
    let url = req.url.trim().to_string();
    if url.is_empty() {
        return Err(ApiError::Validation("url is required".into()));
    }
    let kind = match req.kind {
        Some(k) => source::SourceKind::from_str(&k)
            .map_err(|_| ApiError::Validation(format!("invalid source kind: {k}")))?,
        None => source::SourceKind::detect(&url),
    };
    let src = source::create(
        &state.pool,
        source::NewSource {
            channel_id,
            kind,
            url,
            priority: req.priority.unwrap_or(1),
        },
    )
    .await
    .map_err(internal)?;
    Ok((StatusCode::CREATED, Json(src)))
}

pub async fn update(
    State(state): State<AppState>,
    Path(id): Path<i64>,
    Json(req): Json<UpdateSourceRequest>,
) -> Result<Json<source::Source>, ApiError> {
    let url = req.url.trim().to_string();
    if url.is_empty() {
        return Err(ApiError::Validation("url is required".into()));
    }
    let src = source::update(
        &state.pool,
        id,
        source::UpdateSource { url, priority: req.priority },
    )
    .await
    .map_err(internal)?
    .ok_or(ApiError::NotFound)?;
    Ok(Json(src))
}

pub async fn remove(
    State(state): State<AppState>,
    Path(id): Path<i64>,
) -> Result<StatusCode, ApiError> {
    let deleted = source::delete(&state.pool, id).await.map_err(internal)?;
    if deleted {
        Ok(StatusCode::NO_CONTENT)
    } else {
        Err(ApiError::NotFound)
    }
}

pub async fn toggle(
    State(state): State<AppState>,
    Path(id): Path<i64>,
    Json(req): Json<ToggleRequest>,
) -> Result<Json<source::Source>, ApiError> {
    let changed = source::set_active(&state.pool, id, req.active)
        .await
        .map_err(internal)?;
    if !changed {
        return Err(ApiError::NotFound);
    }
    let src = source::get(&state.pool, id)
        .await
        .map_err(internal)?
        .ok_or(ApiError::NotFound)?;
    Ok(Json(src))
}

pub async fn test(
    State(state): State<AppState>,
    Path(id): Path<i64>,
) -> Result<Json<source::Source>, ApiError> {
    let src = source::get(&state.pool, id)
        .await
        .map_err(internal)?
        .ok_or(ApiError::NotFound)?;
    crate::health::probe_source(
        &state.pool,
        &state.http_client,
        &state.cors_cache,
        &state.live_cache,
        &src,
    )
    .await;
    let updated = source::get(&state.pool, id)
        .await
        .map_err(internal)?
        .ok_or(ApiError::NotFound)?;
    Ok(Json(updated))
}
```

- [ ] **Step 4: Wire routes in `src/routes/api/mod.rs`**

Add `mod sources;` near `mod channels;`. Extend `api_router`:

```rust
pub fn api_router() -> Router<AppState> {
    Router::new()
        .route("/channels", get(channels::list).post(channels::create))
        .route(
            "/channels/:id",
            get(channels::get_one)
                .patch(channels::update)
                .delete(channels::remove),
        )
        .route(
            "/channels/:id/sources",
            get(sources::list_for_channel).post(sources::create),
        )
        .route(
            "/sources/:id",
            get(sources::get_one)
                .patch(sources::update)
                .delete(sources::remove),
        )
        .route("/sources/:id/toggle", post(sources::toggle))
        .route("/sources/:id/test", post(sources::test))
}
```

- [ ] **Step 5: Run tests to verify they pass**

Run: `cargo test --test api`
Expected: PASS (all source + channel tests).

- [ ] **Step 6: Format, lint, commit**

```bash
cargo fmt
cargo clippy -- -D warnings
git add src/routes/api/sources.rs src/routes/api/mod.rs tests/api.rs
git commit -m "feat: source CRUD + toggle + test endpoints on /api/admin"
```

---

## Task 5: Playlist-item endpoints (list/create/get/update/delete/toggle/test)

**Files:**
- Create: `src/routes/api/playlist.rs`
- Modify: `src/routes/api/mod.rs` (declare `mod playlist;`, add routes, reuse `ToggleRequest`)
- Modify: `tests/api.rs` (add tests)

- [ ] **Step 1: Write the failing tests** — append to `tests/api.rs`:

```rust
#[tokio::test]
async fn playlist_crud_round_trip() {
    let app = app().await;

    // create on channel 4
    let r = app
        .clone()
        .oneshot(authed_json(
            "POST",
            "/api/admin/channels/4/playlist",
            serde_json::json!({
                "title": "API Ep",
                "url": "https://vod.example.com/api.mp4",
                "duration_secs": 600,
                "sort_order": 10
            }),
        ))
        .await
        .unwrap();
    assert_eq!(r.status(), StatusCode::CREATED);
    let created = body_json(r).await;
    let id = created["id"].as_i64().unwrap();
    assert_eq!(created["title"], "API Ep");
    assert_eq!(created["channel_id"], 4);

    // list for channel 4 contains it
    let r = app
        .clone()
        .oneshot(authed_get("/api/admin/channels/4/playlist"))
        .await
        .unwrap();
    assert_eq!(r.status(), StatusCode::OK);
    assert!(body_json(r)
        .await
        .as_array()
        .unwrap()
        .iter()
        .any(|i| i["id"] == id));

    // update
    let r = app
        .clone()
        .oneshot(authed_json(
            "PATCH",
            &format!("/api/admin/playlist/{id}"),
            serde_json::json!({
                "title": "API Ep 2",
                "url": "https://vod.example.com/api2.mp4",
                "duration_secs": 700,
                "sort_order": 11
            }),
        ))
        .await
        .unwrap();
    assert_eq!(r.status(), StatusCode::OK);
    assert_eq!(body_json(r).await["title"], "API Ep 2");

    // toggle
    let r = app
        .clone()
        .oneshot(authed_json(
            "POST",
            &format!("/api/admin/playlist/{id}/toggle"),
            serde_json::json!({"active": false}),
        ))
        .await
        .unwrap();
    assert_eq!(r.status(), StatusCode::OK);
    assert_eq!(body_json(r).await["is_active"], false);

    // delete
    let r = app
        .clone()
        .oneshot(
            Request::builder()
                .method("DELETE")
                .uri(format!("/api/admin/playlist/{id}"))
                .header("Authorization", AUTH)
                .body(Body::empty())
                .unwrap(),
        )
        .await
        .unwrap();
    assert_eq!(r.status(), StatusCode::NO_CONTENT);
}

#[tokio::test]
async fn playlist_get_unknown_is_404() {
    let r = app()
        .await
        .oneshot(authed_get("/api/admin/playlist/999999"))
        .await
        .unwrap();
    assert_eq!(r.status(), StatusCode::NOT_FOUND);
}
```

- [ ] **Step 2: Run to verify it fails**

Run: `cargo test --test api playlist_crud_round_trip playlist_get_unknown_is_404`
Expected: FAIL (routes not present).

- [ ] **Step 3: Make `ToggleRequest` reusable** — in `src/routes/api/sources.rs`, change `pub struct ToggleRequest` to be importable: it is already `pub`. In `src/routes/api/playlist.rs` we will import it via `use super::sources::ToggleRequest;`. (No change needed beyond it being `pub`.)

- [ ] **Step 4: Create `src/routes/api/playlist.rs`**

```rust
use axum::{
    extract::{Path, State},
    http::StatusCode,
    response::Json,
};
use serde::Deserialize;

use super::sources::ToggleRequest;
use super::{internal, ApiError};
use crate::{model::playlist_item, AppState};

#[derive(Deserialize)]
pub struct CreatePlaylistItemRequest {
    pub title: String,
    pub url: String,
    pub duration_secs: i64,
    pub sort_order: Option<i64>,
}

#[derive(Deserialize)]
pub struct UpdatePlaylistItemRequest {
    pub title: String,
    pub url: String,
    pub duration_secs: i64,
    pub sort_order: i64,
}

pub async fn list_for_channel(
    State(state): State<AppState>,
    Path(channel_id): Path<i64>,
) -> Result<Json<Vec<playlist_item::PlaylistItem>>, ApiError> {
    let items = playlist_item::list_for_channel(&state.pool, channel_id)
        .await
        .map_err(internal)?;
    Ok(Json(items))
}

pub async fn get_one(
    State(state): State<AppState>,
    Path(id): Path<i64>,
) -> Result<Json<playlist_item::PlaylistItem>, ApiError> {
    let item = playlist_item::get(&state.pool, id)
        .await
        .map_err(internal)?
        .ok_or(ApiError::NotFound)?;
    Ok(Json(item))
}

pub async fn create(
    State(state): State<AppState>,
    Path(channel_id): Path<i64>,
    Json(req): Json<CreatePlaylistItemRequest>,
) -> Result<(StatusCode, Json<playlist_item::PlaylistItem>), ApiError> {
    let title = req.title.trim().to_string();
    let url = req.url.trim().to_string();
    if title.is_empty() || url.is_empty() {
        return Err(ApiError::Validation("title and url are required".into()));
    }
    if req.duration_secs <= 0 {
        return Err(ApiError::Validation("duration_secs must be > 0".into()));
    }
    let item = playlist_item::create(
        &state.pool,
        playlist_item::NewPlaylistItem {
            channel_id,
            title,
            url,
            duration_secs: req.duration_secs,
            sort_order: req.sort_order.unwrap_or(0),
        },
    )
    .await
    .map_err(internal)?;
    Ok((StatusCode::CREATED, Json(item)))
}

pub async fn update(
    State(state): State<AppState>,
    Path(id): Path<i64>,
    Json(req): Json<UpdatePlaylistItemRequest>,
) -> Result<Json<playlist_item::PlaylistItem>, ApiError> {
    let title = req.title.trim().to_string();
    let url = req.url.trim().to_string();
    if title.is_empty() || url.is_empty() {
        return Err(ApiError::Validation("title and url are required".into()));
    }
    let item = playlist_item::update(
        &state.pool,
        id,
        playlist_item::UpdatePlaylistItem {
            title,
            url,
            duration_secs: req.duration_secs,
            sort_order: req.sort_order,
        },
    )
    .await
    .map_err(internal)?
    .ok_or(ApiError::NotFound)?;
    Ok(Json(item))
}

pub async fn remove(
    State(state): State<AppState>,
    Path(id): Path<i64>,
) -> Result<StatusCode, ApiError> {
    let deleted = playlist_item::delete(&state.pool, id)
        .await
        .map_err(internal)?;
    if deleted {
        Ok(StatusCode::NO_CONTENT)
    } else {
        Err(ApiError::NotFound)
    }
}

pub async fn toggle(
    State(state): State<AppState>,
    Path(id): Path<i64>,
    Json(req): Json<ToggleRequest>,
) -> Result<Json<playlist_item::PlaylistItem>, ApiError> {
    let changed = playlist_item::set_active(&state.pool, id, req.active)
        .await
        .map_err(internal)?;
    if !changed {
        return Err(ApiError::NotFound);
    }
    let item = playlist_item::get(&state.pool, id)
        .await
        .map_err(internal)?
        .ok_or(ApiError::NotFound)?;
    Ok(Json(item))
}

pub async fn test(
    State(state): State<AppState>,
    Path(id): Path<i64>,
) -> Result<Json<playlist_item::PlaylistItem>, ApiError> {
    let item = playlist_item::get(&state.pool, id)
        .await
        .map_err(internal)?
        .ok_or(ApiError::NotFound)?;
    crate::health::probe_playlist_item(&state.pool, &state.http_client, &state.cors_cache, &item)
        .await;
    let updated = playlist_item::get(&state.pool, id)
        .await
        .map_err(internal)?
        .ok_or(ApiError::NotFound)?;
    Ok(Json(updated))
}
```

- [ ] **Step 5: Wire routes in `src/routes/api/mod.rs`**

Add `mod playlist;`. Make `mod sources;` declared as `pub(crate) mod sources;` is NOT needed (same-module sibling access via `super::sources` works because `playlist` and `sources` are sibling submodules of `api`; `ToggleRequest` is `pub`). Extend `api_router`:

```rust
        .route(
            "/channels/:id/playlist",
            get(playlist::list_for_channel).post(playlist::create),
        )
        .route(
            "/playlist/:id",
            get(playlist::get_one)
                .patch(playlist::update)
                .delete(playlist::remove),
        )
        .route("/playlist/:id/toggle", post(playlist::toggle))
        .route("/playlist/:id/test", post(playlist::test))
```

- [ ] **Step 6: Run tests to verify they pass**

Run: `cargo test --test api`
Expected: PASS (all api tests).

- [ ] **Step 7: Format, lint, commit**

```bash
cargo fmt
cargo clippy -- -D warnings
git add src/routes/api/playlist.rs src/routes/api/mod.rs tests/api.rs
git commit -m "feat: playlist-item CRUD + toggle + test endpoints on /api/admin"
```

---

## Task 6: `mytvctl` CLI binary

**Files:**
- Modify: `Cargo.toml` (add `clap`)
- Create: `src/bin/mytvctl.rs`

- [ ] **Step 1: Add the `clap` dependency** — in `Cargo.toml` under `[dependencies]`:

```toml
clap = { version = "4", features = ["derive"] }
```

- [ ] **Step 2: Write the failing unit tests** — create `src/bin/mytvctl.rs` with ONLY the pure helpers and tests first (so the file compiles and tests fail meaningfully). Start with this content:

```rust
fn resolve_base_url(flag: Option<String>, env: Option<String>) -> String {
    flag.or(env)
        .unwrap_or_else(|| "http://localhost:3000".to_string())
}

fn exit_code_for_status(status: u16) -> i32 {
    if (200..300).contains(&status) {
        0
    } else {
        1
    }
}

fn main() {}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn base_url_prefers_flag_then_env_then_default() {
        assert_eq!(
            resolve_base_url(Some("http://flag".into()), Some("http://env".into())),
            "http://flag"
        );
        assert_eq!(resolve_base_url(None, Some("http://env".into())), "http://env");
        assert_eq!(resolve_base_url(None, None), "http://localhost:3000");
    }

    #[test]
    fn exit_code_maps_2xx_to_zero_else_one() {
        assert_eq!(exit_code_for_status(200), 0);
        assert_eq!(exit_code_for_status(201), 0);
        assert_eq!(exit_code_for_status(404), 1);
        assert_eq!(exit_code_for_status(500), 1);
    }
}
```

- [ ] **Step 3: Run to verify the tests pass for the helpers (red→green baseline)**

Run: `cargo test --bin mytvctl`
Expected: PASS (2 tests). This establishes the binary compiles and the pure helpers work. (We now add the request-mapping logic + its tests.)

- [ ] **Step 4: Write the failing test for request mapping** — add to the `tests` module in `src/bin/mytvctl.rs`:

```rust
    #[test]
    fn request_for_channel_create_builds_post() {
        let cmd = Command::Channel(ChannelCmd::Create {
            name: "N".into(),
            category: "C".into(),
            r#type: "live".into(),
            logo_url: None,
            sort_order: 3,
            loop_anchor: None,
        });
        let req = request_for(&cmd);
        assert_eq!(req.method, "POST");
        assert_eq!(req.path, "/api/admin/channels");
        let body = req.body.expect("has body");
        assert_eq!(body["name"], "N");
        assert_eq!(body["type"], "live");
        assert_eq!(body["sort_order"], 3);
    }

    #[test]
    fn request_for_channel_list_is_get_no_body() {
        let req = request_for(&Command::Channel(ChannelCmd::List));
        assert_eq!(req.method, "GET");
        assert_eq!(req.path, "/api/admin/channels");
        assert!(req.body.is_none());
    }

    #[test]
    fn request_for_source_toggle_builds_body() {
        let req = request_for(&Command::Source(SourceCmd::Toggle { id: 4, active: false }));
        assert_eq!(req.method, "POST");
        assert_eq!(req.path, "/api/admin/sources/4/toggle");
        assert_eq!(req.body.unwrap()["active"], false);
    }
```

- [ ] **Step 5: Run to verify it fails**

Run: `cargo test --bin mytvctl request_for`
Expected: FAIL — `Command`, `ChannelCmd`, `request_for`, `ApiRequest` not defined.

- [ ] **Step 6: Implement the full CLI** — replace the entire contents of `src/bin/mytvctl.rs` with:

```rust
use clap::{Parser, Subcommand};
use serde_json::{json, Value};

#[derive(Parser)]
#[command(name = "mytvctl", about = "MyTV admin CLI (talks to /api/admin)")]
struct Cli {
    /// Base URL (else $MYTV_BASE_URL, else http://localhost:3000)
    #[arg(long, global = true)]
    base_url: Option<String>,
    #[command(subcommand)]
    command: Command,
}

#[derive(Subcommand)]
enum Command {
    #[command(subcommand)]
    Channel(ChannelCmd),
    #[command(subcommand)]
    Source(SourceCmd),
    #[command(subcommand)]
    Playlist(PlaylistCmd),
}

#[derive(Subcommand)]
enum ChannelCmd {
    List,
    Get { id: i64 },
    Create {
        #[arg(long)] name: String,
        #[arg(long)] category: String,
        #[arg(long = "type")] r#type: String,
        #[arg(long)] logo_url: Option<String>,
        #[arg(long, default_value_t = 0)] sort_order: i64,
        #[arg(long)] loop_anchor: Option<String>,
    },
    Update {
        id: i64,
        #[arg(long)] name: String,
        #[arg(long)] category: String,
        #[arg(long = "type")] r#type: String,
        #[arg(long)] logo_url: Option<String>,
        #[arg(long, default_value_t = 0)] sort_order: i64,
        #[arg(long)] loop_anchor: Option<String>,
    },
    Delete { id: i64 },
}

#[derive(Subcommand)]
enum SourceCmd {
    List {
        #[arg(long)] channel: i64,
    },
    Get { id: i64 },
    Create {
        #[arg(long)] channel: i64,
        #[arg(long)] url: String,
        #[arg(long)] priority: Option<i64>,
        #[arg(long)] kind: Option<String>,
    },
    Update {
        id: i64,
        #[arg(long)] url: String,
        #[arg(long, default_value_t = 1)] priority: i64,
    },
    Delete { id: i64 },
    Toggle {
        id: i64,
        #[arg(long, action = clap::ArgAction::Set)] active: bool,
    },
    Test { id: i64 },
}

#[derive(Subcommand)]
enum PlaylistCmd {
    List {
        #[arg(long)] channel: i64,
    },
    Get { id: i64 },
    Create {
        #[arg(long)] channel: i64,
        #[arg(long)] title: String,
        #[arg(long)] url: String,
        #[arg(long)] duration_secs: i64,
        #[arg(long)] sort_order: Option<i64>,
    },
    Update {
        id: i64,
        #[arg(long)] title: String,
        #[arg(long)] url: String,
        #[arg(long)] duration_secs: i64,
        #[arg(long, default_value_t = 0)] sort_order: i64,
    },
    Delete { id: i64 },
    Toggle {
        id: i64,
        #[arg(long, action = clap::ArgAction::Set)] active: bool,
    },
    Test { id: i64 },
}

/// A resolved HTTP request to make against the API.
struct ApiRequest {
    method: &'static str,
    path: String,
    body: Option<Value>,
}

fn resolve_base_url(flag: Option<String>, env: Option<String>) -> String {
    flag.or(env)
        .unwrap_or_else(|| "http://localhost:3000".to_string())
}

fn exit_code_for_status(status: u16) -> i32 {
    if (200..300).contains(&status) {
        0
    } else {
        1
    }
}

/// Pure mapping from a parsed command to an HTTP request spec. No I/O.
fn request_for(cmd: &Command) -> ApiRequest {
    match cmd {
        Command::Channel(c) => match c {
            ChannelCmd::List => ApiRequest { method: "GET", path: "/api/admin/channels".into(), body: None },
            ChannelCmd::Get { id } => ApiRequest { method: "GET", path: format!("/api/admin/channels/{id}"), body: None },
            ChannelCmd::Create { name, category, r#type, logo_url, sort_order, loop_anchor } => ApiRequest {
                method: "POST",
                path: "/api/admin/channels".into(),
                body: Some(json!({
                    "name": name, "category": category, "type": r#type,
                    "logo_url": logo_url, "sort_order": sort_order, "loop_anchor": loop_anchor
                })),
            },
            ChannelCmd::Update { id, name, category, r#type, logo_url, sort_order, loop_anchor } => ApiRequest {
                method: "PATCH",
                path: format!("/api/admin/channels/{id}"),
                body: Some(json!({
                    "name": name, "category": category, "type": r#type,
                    "logo_url": logo_url, "sort_order": sort_order, "loop_anchor": loop_anchor
                })),
            },
            ChannelCmd::Delete { id } => ApiRequest { method: "DELETE", path: format!("/api/admin/channels/{id}"), body: None },
        },
        Command::Source(c) => match c {
            SourceCmd::List { channel } => ApiRequest { method: "GET", path: format!("/api/admin/channels/{channel}/sources"), body: None },
            SourceCmd::Get { id } => ApiRequest { method: "GET", path: format!("/api/admin/sources/{id}"), body: None },
            SourceCmd::Create { channel, url, priority, kind } => ApiRequest {
                method: "POST",
                path: format!("/api/admin/channels/{channel}/sources"),
                body: Some(json!({ "url": url, "priority": priority, "kind": kind })),
            },
            SourceCmd::Update { id, url, priority } => ApiRequest {
                method: "PATCH",
                path: format!("/api/admin/sources/{id}"),
                body: Some(json!({ "url": url, "priority": priority })),
            },
            SourceCmd::Delete { id } => ApiRequest { method: "DELETE", path: format!("/api/admin/sources/{id}"), body: None },
            SourceCmd::Toggle { id, active } => ApiRequest {
                method: "POST",
                path: format!("/api/admin/sources/{id}/toggle"),
                body: Some(json!({ "active": active })),
            },
            SourceCmd::Test { id } => ApiRequest { method: "POST", path: format!("/api/admin/sources/{id}/test"), body: None },
        },
        Command::Playlist(c) => match c {
            PlaylistCmd::List { channel } => ApiRequest { method: "GET", path: format!("/api/admin/channels/{channel}/playlist"), body: None },
            PlaylistCmd::Get { id } => ApiRequest { method: "GET", path: format!("/api/admin/playlist/{id}"), body: None },
            PlaylistCmd::Create { channel, title, url, duration_secs, sort_order } => ApiRequest {
                method: "POST",
                path: format!("/api/admin/channels/{channel}/playlist"),
                body: Some(json!({ "title": title, "url": url, "duration_secs": duration_secs, "sort_order": sort_order })),
            },
            PlaylistCmd::Update { id, title, url, duration_secs, sort_order } => ApiRequest {
                method: "PATCH",
                path: format!("/api/admin/playlist/{id}"),
                body: Some(json!({ "title": title, "url": url, "duration_secs": duration_secs, "sort_order": sort_order })),
            },
            PlaylistCmd::Delete { id } => ApiRequest { method: "DELETE", path: format!("/api/admin/playlist/{id}"), body: None },
            PlaylistCmd::Toggle { id, active } => ApiRequest {
                method: "POST",
                path: format!("/api/admin/playlist/{id}/toggle"),
                body: Some(json!({ "active": active })),
            },
            PlaylistCmd::Test { id } => ApiRequest { method: "POST", path: format!("/api/admin/playlist/{id}/test"), body: None },
        },
    }
}

#[tokio::main]
async fn main() {
    let cli = Cli::parse();

    let password = match std::env::var("MYTV_ADMIN_PASSWORD") {
        Ok(p) if !p.is_empty() => p,
        _ => {
            eprintln!("error: set MYTV_ADMIN_PASSWORD");
            std::process::exit(2);
        }
    };
    let base_url = resolve_base_url(cli.base_url.clone(), std::env::var("MYTV_BASE_URL").ok());

    let req = request_for(&cli.command);
    let client = reqwest::Client::new();
    let url = format!("{}{}", base_url.trim_end_matches('/'), req.path);
    let method = reqwest::Method::from_bytes(req.method.as_bytes()).unwrap();

    let mut builder = client
        .request(method, &url)
        .basic_auth("user", Some(&password));
    if let Some(body) = req.body {
        builder = builder.json(&body);
    }

    match builder.send().await {
        Ok(resp) => {
            let status = resp.status().as_u16();
            let text = resp.text().await.unwrap_or_default();
            if !text.is_empty() {
                println!("{text}");
            }
            std::process::exit(exit_code_for_status(status));
        }
        Err(e) => {
            eprintln!("request failed: {e}");
            std::process::exit(1);
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn base_url_prefers_flag_then_env_then_default() {
        assert_eq!(
            resolve_base_url(Some("http://flag".into()), Some("http://env".into())),
            "http://flag"
        );
        assert_eq!(resolve_base_url(None, Some("http://env".into())), "http://env");
        assert_eq!(resolve_base_url(None, None), "http://localhost:3000");
    }

    #[test]
    fn exit_code_maps_2xx_to_zero_else_one() {
        assert_eq!(exit_code_for_status(200), 0);
        assert_eq!(exit_code_for_status(201), 0);
        assert_eq!(exit_code_for_status(404), 1);
        assert_eq!(exit_code_for_status(500), 1);
    }

    #[test]
    fn request_for_channel_create_builds_post() {
        let cmd = Command::Channel(ChannelCmd::Create {
            name: "N".into(),
            category: "C".into(),
            r#type: "live".into(),
            logo_url: None,
            sort_order: 3,
            loop_anchor: None,
        });
        let req = request_for(&cmd);
        assert_eq!(req.method, "POST");
        assert_eq!(req.path, "/api/admin/channels");
        let body = req.body.expect("has body");
        assert_eq!(body["name"], "N");
        assert_eq!(body["type"], "live");
        assert_eq!(body["sort_order"], 3);
    }

    #[test]
    fn request_for_channel_list_is_get_no_body() {
        let req = request_for(&Command::Channel(ChannelCmd::List));
        assert_eq!(req.method, "GET");
        assert_eq!(req.path, "/api/admin/channels");
        assert!(req.body.is_none());
    }

    #[test]
    fn request_for_source_toggle_builds_body() {
        let req = request_for(&Command::Source(SourceCmd::Toggle { id: 4, active: false }));
        assert_eq!(req.method, "POST");
        assert_eq!(req.path, "/api/admin/sources/4/toggle");
        assert_eq!(req.body.unwrap()["active"], false);
    }
}
```

- [ ] **Step 7: Run tests to verify they pass**

Run: `cargo test --bin mytvctl`
Expected: PASS (5 tests).

- [ ] **Step 8: Smoke-test the binary builds and shows help**

Run: `cargo run --bin mytvctl -- --help`
Expected: clap prints usage with `channel`, `source`, `playlist` subcommands; exit 0.

Run: `MYTV_ADMIN_PASSWORD= cargo run --bin mytvctl -- channel list 2>&1 | head -1`
Expected: prints `error: set MYTV_ADMIN_PASSWORD` (empty password → exit 2).

- [ ] **Step 9: Format, lint, commit**

```bash
cargo fmt
cargo clippy -- -D warnings
git add Cargo.toml Cargo.lock src/bin/mytvctl.rs
git commit -m "feat: add mytvctl CLI client for the /api/admin API"
```

---

## Final verification

- [ ] Run `cargo fmt --check` → no diff.
- [ ] Run `cargo clippy --all-targets -- -D warnings` → exits 0.
- [ ] Run `cargo test` → all pass.
- [ ] Update `CLAUDE.md`: the test-count line and the project structure block (add `src/routes/api/` — JSON admin API; `src/bin/mytvctl.rs` — CLI client; `tests/api.rs`). Commit that doc change. Compute the new counts from the final `cargo test` output rather than guessing (Task 1 adds 4 unit tests + Task 6 adds 5 unit tests = +9 unit; `tests/api.rs` adds the integration tests written across Tasks 2-5).
```bash
git add CLAUDE.md
git commit -m "docs: document /api/admin, mytvctl, and updated test counts"
```
