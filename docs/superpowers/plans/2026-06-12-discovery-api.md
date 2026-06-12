# Discovery API + CLI Implementation Plan

> **For agentic workers:** REQUIRED SUB-SKILL: Use superpowers:subagent-driven-development (recommended) or superpowers:executing-plans to implement this plan task-by-task. Steps use checkbox (`- [ ]`) syntax for tracking.

**Goal:** Expose the discover subsystem (M3U/YouTube search, URL resolve, add) as JSON under `/api/admin/discover`, plus a `mytvctl discover` CLI noun.

**Architecture:** First extract the search/resolve orchestration currently inlined in the HTML discover handlers into reusable `pub(crate)` functions (behavior-preserving), so the new JSON handlers and the web UI share one implementation. Then add `src/routes/api/discover.rs` (clean candidate DTOs, reusing Spec 2's `ApiError`) wired into the existing `api_router` behind `basic_auth`, with `discover/add` a thin wrapper over the existing `do_discover_add`. Finally add a `discover` noun to `mytvctl`.

**Tech Stack:** Rust, Axum 0.7, SQLx (SQLite), serde/serde_json, reqwest, clap, `tower::ServiceExt::oneshot` integration tests.

**Spec:** `docs/superpowers/specs/2026-06-12-discovery-api-design.md`

---

## File Structure

- `src/routes/admin/discover/m3u.rs` — Task 1: add `pub(crate) async fn search(...)` (lift from the handler).
- `src/routes/admin/discover/youtube.rs` — Task 1: widen 4 fns from `pub(super)` to `pub(crate)`.
- `src/routes/admin/discover/mod.rs` — Task 1: add `ResolvedMeta` + `resolve_manual`/`resolve_channel`; re-export for the API; thin out the 3 HTML handlers to call the shared fns.
- `src/routes/api/discover.rs` — Tasks 2-4: JSON handlers + candidate DTOs.
- `src/routes/api/mod.rs` — Tasks 2-4: `mod discover;`, route wiring, a `503` `ApiError` variant.
- `src/bin/mytvctl.rs` — Task 5: `discover` subcommand + `request_for` arms + unit tests.
- `tests/api.rs` — Tasks 2-4: integration tests.

Reference facts (verified against current code):
- `discover::do_discover_add(DiscoverAddParams)` and `DiscoverAddParams` are already `pub use`d from `discover/mod.rs` (reachable as `crate::routes::admin::discover::{do_discover_add, DiscoverAddParams}`).
- `DiscoverAddParams` fields: `pool, client, url, title, source_kind, duration_secs, channel_choice ("new"|"<id>"), new_name, new_category, new_channel_type`. Returns `Result<i64 /*channel_id*/, StatusCode>`.
- `m3u::{country_to_code, fetch_m3u, url_is_reachable, M3uResultRow}`; `crate::media::m3u::{parse_m3u, filter_m3u}`.
- `youtube::{fetch_youtube_results, fetch_youtube_channels, normalize_channel_url, channel_title_from_url, YoutubeResultRow}` (the first four are currently `pub(super)`).
- `resolver::{needs_resolution(&str)->bool, fetch_duration_secs(&str)->Result<i64>, fetch_title(&str)->Result<String>}`.
- `source::SourceKind::detect(&str)`, `.as_str()`.
- `tests/api.rs` `app()` builds state with `youtube_api_key: None` — so the no-key path is deterministically testable.
- Spec 2's `ApiError { NotFound, Validation(String), Internal }` with `IntoResponse` lives in `src/routes/api/mod.rs`; `internal()` maps errors to a logged 500.

---

## Task 1: Refactor — extract shared search/resolve functions (behavior-preserving)

**Files:** `src/routes/admin/discover/m3u.rs`, `youtube.rs`, `mod.rs`. No new tests (existing `tests/http.rs` discover tests must stay green).

- [ ] **Step 1: Widen YouTube helper visibility** — in `src/routes/admin/discover/youtube.rs`, change these four signatures from `pub(super)` to `pub(crate)`: `fetch_youtube_results`, `fetch_youtube_channels`, `normalize_channel_url`, `channel_title_from_url`. (Leave everything else unchanged.)

- [ ] **Step 2: Add `m3u::search`** — in `src/routes/admin/discover/m3u.rs`, add at the end:

```rust
/// Fetch + parse + filter the iptv-org M3U, then keep only reachable entries
/// (capped at `limit`). Shared by the HTML handler and the JSON API.
pub(crate) async fn search(
    client: &reqwest::Client,
    country: &str,
    group: &str,
    limit: usize,
) -> anyhow::Result<Vec<M3uResultRow>> {
    let country_code = if country.trim().is_empty() {
        None
    } else {
        country_to_code(country)
    };
    let raw = fetch_m3u(client, country_code.as_deref()).await?;
    let all = crate::media::m3u::parse_m3u(&raw);
    let matches: Vec<_> = crate::media::m3u::filter_m3u(&all, "", group)
        .into_iter()
        .take(limit)
        .collect();

    let handles: Vec<_> = matches
        .iter()
        .map(|ch| {
            let client = client.clone();
            let url = ch.url.clone();
            tokio::spawn(async move { url_is_reachable(&client, &url).await })
        })
        .collect();
    let mut reachable = Vec::with_capacity(handles.len());
    for h in handles {
        reachable.push(h.await.unwrap_or(false));
    }

    let rows = matches
        .iter()
        .zip(reachable)
        .filter(|(_, ok)| *ok)
        .enumerate()
        .map(|(i, (ch, _))| M3uResultRow {
            name: ch.name.clone(),
            group: ch.group.clone(),
            country: ch.country.clone(),
            url: ch.url.clone(),
            source_kind: crate::model::source::SourceKind::detect(&ch.url)
                .as_str()
                .to_string(),
            form_id: i,
        })
        .collect();
    Ok(rows)
}
```

(Capping `matches` with `.take(limit)` BEFORE the reachability probes bounds the HEAD requests. The HTML handler will pass `usize::MAX`, preserving its current unbounded behavior.)

- [ ] **Step 3: Add `ResolvedMeta` + `resolve_manual` + `resolve_channel`** — in `src/routes/admin/discover/mod.rs`, after the `html_escape` helper (around line 136), add:

```rust
/// Resolved metadata for a single URL — shared shape behind the manual/channel
/// resolve HTML handlers and the JSON API.
pub(crate) struct ResolvedMeta {
    pub url: String,
    pub title: String,
    pub duration_secs: i64,
    pub is_live: bool,
    pub source_kind: String,
}

/// Resolve an arbitrary stream URL. For YouTube URLs, fetch duration+title via
/// yt-dlp (5s timeouts); otherwise title=url, duration=0. `is_live` ≙ duration 0.
pub(crate) async fn resolve_manual(url: &str) -> Result<ResolvedMeta, StatusCode> {
    if !url.starts_with("http://") && !url.starts_with("https://") {
        return Err(StatusCode::UNPROCESSABLE_ENTITY);
    }
    let (duration_secs, title) = if resolver::needs_resolution(url) {
        let (dur_result, title_result) = tokio::join!(
            tokio::time::timeout(
                std::time::Duration::from_secs(5),
                resolver::fetch_duration_secs(url),
            ),
            tokio::time::timeout(
                std::time::Duration::from_secs(5),
                resolver::fetch_title(url),
            ),
        );
        let duration = dur_result.ok().and_then(|r| r.ok()).unwrap_or(0);
        let title = title_result
            .ok()
            .and_then(|r| r.ok())
            .unwrap_or_else(|| url.to_string());
        (duration, title)
    } else {
        (0, url.to_string())
    };
    let is_live = duration_secs == 0;
    Ok(ResolvedMeta {
        url: url.to_string(),
        title,
        duration_secs,
        is_live,
        source_kind: source::SourceKind::detect(url).as_str().to_string(),
    })
}

/// Resolve a YouTube channel URL to a normalized live-source candidate.
pub(crate) fn resolve_channel(url: &str) -> Result<ResolvedMeta, StatusCode> {
    let normalized =
        youtube::normalize_channel_url(url).ok_or(StatusCode::UNPROCESSABLE_ENTITY)?;
    let title = youtube::channel_title_from_url(&normalized);
    Ok(ResolvedMeta {
        url: normalized,
        title,
        duration_secs: 0,
        is_live: true,
        source_kind: "youtube_live".to_string(),
    })
}
```

- [ ] **Step 4: Re-export the shared items for the API** — in `src/routes/admin/discover/mod.rs`, near the existing `pub use add::{...}` (line 5), add:

```rust
pub(crate) use m3u::{search as m3u_search, M3uResultRow};
pub(crate) use youtube::{fetch_youtube_channels, fetch_youtube_results, YoutubeResultRow};
```

(`ResolvedMeta`, `resolve_manual`, `resolve_channel` are already `pub(crate)` in this module, reachable as `crate::routes::admin::discover::{ResolvedMeta, resolve_manual, resolve_channel}`.)

- [ ] **Step 5: Thin out the three HTML handlers to call the shared fns** — in `src/routes/admin/discover/mod.rs`:

Replace `discover_m3u_search` (currently ~lines 202-258) with:
```rust
pub async fn discover_m3u_search(
    State(state): State<AppState>,
    Form(form): Form<M3uSearchForm>,
) -> Html<String> {
    let rows = match m3u::search(&state.http_client, &form.country, &form.group, usize::MAX).await {
        Ok(r) => r,
        Err(e) => {
            tracing::error!("M3U fetch error: {e}");
            return Html("<p class=\"empty-state\" style=\"color:#f77\">Failed to fetch M3U list. Check server logs.</p>".to_string());
        }
    };
    match (M3uResultsTemplate { rows }).render() {
        Ok(html) => Html(html),
        Err(e) => {
            tracing::error!("template error: {e}");
            Html("<p class=\"empty-state\" style=\"color:#f77\">Render error.</p>".to_string())
        }
    }
}
```

Replace the body of `discover_channel_resolve` (currently ~lines 297-325) so it uses `resolve_channel`:
```rust
pub async fn discover_channel_resolve(
    State(state): State<AppState>,
    Form(form): Form<ChannelUrlForm>,
) -> Result<Html<String>, StatusCode> {
    let meta = resolve_channel(&form.url)?;
    let channels = channel::list(&state.pool)
        .await
        .map_err(internal_error)?
        .into_iter()
        .map(|ch| DiscoverChannelOption {
            id: ch.id,
            name: ch.name,
            type_str: ch.r#type,
        })
        .collect();
    render(ManualResultTemplate {
        form_id: "channel".to_string(),
        url: meta.url,
        title: meta.title,
        group: String::new(),
        is_live: meta.is_live,
        duration_secs: meta.duration_secs,
        source_kind: meta.source_kind,
        show_duration_input: false,
        channels,
    })
}
```

Replace the body of `discover_manual_resolve` (currently ~lines 327-377) so it uses `resolve_manual`:
```rust
pub async fn discover_manual_resolve(
    State(state): State<AppState>,
    Form(form): Form<ManualResolveForm>,
) -> Result<Html<String>, StatusCode> {
    let meta = resolve_manual(&form.url).await?;
    let channels = channel::list(&state.pool)
        .await
        .map_err(internal_error)?
        .into_iter()
        .map(|ch| DiscoverChannelOption {
            id: ch.id,
            name: ch.name,
            type_str: ch.r#type,
        })
        .collect();
    render(ManualResultTemplate {
        form_id: "manual".to_string(),
        url: meta.url.clone(),
        title: meta.title,
        group: String::new(),
        is_live: meta.is_live,
        duration_secs: meta.duration_secs,
        source_kind: meta.source_kind,
        show_duration_input: !meta.is_live && meta.duration_secs == 0,
        channels,
    })
}
```

After these edits, some imports in `mod.rs` may become unused (e.g. `media_m3u`, `resolver`, `source` if no longer referenced directly in mod.rs — note `resolve_manual`/`resolve_channel` now use `resolver`/`source`/`youtube`, so those stay). Remove only genuinely-unused imports so clippy passes.

- [ ] **Step 6: Verify no behavior change**

Run: `cargo test --test http`
Expected: PASS — all existing discover/admin tests unchanged.
Run: `cargo build` then `cargo clippy --all-targets -- -D warnings`
Expected: clippy exits 0 (fix any now-unused import it flags).

- [ ] **Step 7: Format, commit**

```bash
cargo fmt
cargo clippy --all-targets -- -D warnings
git add src/routes/admin/discover/m3u.rs src/routes/admin/discover/youtube.rs src/routes/admin/discover/mod.rs
git commit -m "refactor: extract shared m3u::search + resolve_manual/resolve_channel for reuse"
```

---

## Task 2: API scaffolding + resolve & channel endpoints

**Files:** create `src/routes/api/discover.rs`; modify `src/routes/api/mod.rs`; modify `tests/api.rs`.

- [ ] **Step 1: Write the failing tests** — append to `tests/api.rs`:

```rust
#[tokio::test]
async fn discover_resolve_non_youtube_url_is_deterministic() {
    let r = app().await.oneshot(authed_json("POST", "/api/admin/discover/resolve",
        serde_json::json!({"url": "https://cdn.example.com/live.m3u8"}))).await.unwrap();
    assert_eq!(r.status(), StatusCode::OK);
    let j = body_json(r).await;
    assert_eq!(j["url"], "https://cdn.example.com/live.m3u8");
    assert_eq!(j["title"], "https://cdn.example.com/live.m3u8");
    assert_eq!(j["duration_secs"], 0);
    assert_eq!(j["is_live"], true);
    assert_eq!(j["source_kind"], "hls");
}

#[tokio::test]
async fn discover_resolve_bad_url_is_422() {
    let r = app().await.oneshot(authed_json("POST", "/api/admin/discover/resolve",
        serde_json::json!({"url": "ftp://nope"}))).await.unwrap();
    assert_eq!(r.status(), StatusCode::UNPROCESSABLE_ENTITY);
}

#[tokio::test]
async fn discover_channel_valid_youtube_handle() {
    let r = app().await.oneshot(authed_json("POST", "/api/admin/discover/channel",
        serde_json::json!({"url": "https://www.youtube.com/@NASA"}))).await.unwrap();
    assert_eq!(r.status(), StatusCode::OK);
    let j = body_json(r).await;
    assert_eq!(j["source_kind"], "youtube_live");
    assert_eq!(j["is_live"], true);
}

#[tokio::test]
async fn discover_channel_non_youtube_is_422() {
    let r = app().await.oneshot(authed_json("POST", "/api/admin/discover/channel",
        serde_json::json!({"url": "https://example.com/notyt"}))).await.unwrap();
    assert_eq!(r.status(), StatusCode::UNPROCESSABLE_ENTITY);
}

#[tokio::test]
async fn discover_resolve_requires_auth() {
    let r = app().await.oneshot(Request::builder().method("POST")
        .uri("/api/admin/discover/resolve").header("content-type","application/json")
        .body(Body::from(r#"{"url":"https://x/y.m3u8"}"#)).unwrap()).await.unwrap();
    assert_eq!(r.status(), StatusCode::UNAUTHORIZED);
}
```

NOTE: `discover_channel_valid_youtube_handle` and `discover_channel_non_youtube_is_422` depend on `youtube::normalize_channel_url` accepting `@handle`/`/channel/`/`/c/` forms and rejecting non-YouTube hosts. Before relying on the exact assertions, open `src/routes/admin/discover/youtube.rs` and read `normalize_channel_url` to confirm `https://www.youtube.com/@NASA` normalizes (Some) and `https://example.com/notyt` returns None. If the accepted forms differ, adjust the test URLs to ones the function actually accepts/rejects (keep one Some case and one None case).

- [ ] **Step 2: Run to verify FAIL**

Run: `cargo test --test api discover_resolve_non_youtube_url_is_deterministic discover_resolve_bad_url_is_422 discover_channel_valid_youtube_handle discover_channel_non_youtube_is_422 discover_resolve_requires_auth`
Expected: FAIL (routes absent).

- [ ] **Step 3: Create `src/routes/api/discover.rs`**

```rust
use axum::{extract::State, response::Json};
use serde::{Deserialize, Serialize};

use super::{ApiError};
use crate::routes::admin::discover::{resolve_channel, resolve_manual, ResolvedMeta};
use crate::AppState;

#[derive(Serialize)]
pub struct ResolvedCandidate {
    pub url: String,
    pub title: String,
    pub duration_secs: i64,
    pub is_live: bool,
    pub source_kind: String,
}

impl From<ResolvedMeta> for ResolvedCandidate {
    fn from(m: ResolvedMeta) -> Self {
        ResolvedCandidate {
            url: m.url,
            title: m.title,
            duration_secs: m.duration_secs,
            is_live: m.is_live,
            source_kind: m.source_kind,
        }
    }
}

#[derive(Deserialize)]
pub struct ResolveRequest {
    pub url: String,
}

pub async fn resolve(
    State(_state): State<AppState>,
    Json(req): Json<ResolveRequest>,
) -> Result<Json<ResolvedCandidate>, ApiError> {
    let meta = resolve_manual(&req.url)
        .await
        .map_err(|_| ApiError::Validation("invalid or unresolvable URL".into()))?;
    Ok(Json(meta.into()))
}

pub async fn channel(
    State(_state): State<AppState>,
    Json(req): Json<ResolveRequest>,
) -> Result<Json<ResolvedCandidate>, ApiError> {
    let meta = resolve_channel(&req.url)
        .map_err(|_| ApiError::Validation("not a recognized YouTube channel URL".into()))?;
    Ok(Json(meta.into()))
}
```

(`resolve`/`channel` don't use `state` yet — the `_state` extractor keeps the handler signature uniform and lets `axum` route it; clippy is fine with `State(_state)`. If clippy flags it, drop the extractor entirely: `pub async fn resolve(Json(req): Json<ResolveRequest>) -> ...`.)

- [ ] **Step 4: Wire routes + module in `src/routes/api/mod.rs`**

Add `mod discover;` near the other `mod` lines. Extend `api_router()` (keep all existing routes) with:
```rust
        .route("/discover/resolve", post(discover::resolve))
        .route("/discover/channel", post(discover::channel))
```
(`post` is already imported from Spec 2's Task 4.)

- [ ] **Step 5: Run to verify PASS**

Run: `cargo test --test api`
Expected: PASS (the 5 new tests + all Spec 2 api tests).

- [ ] **Step 6: Format, lint, commit**

```bash
cargo fmt
cargo clippy --all-targets -- -D warnings
git add src/routes/api/discover.rs src/routes/api/mod.rs tests/api.rs
git commit -m "feat: /api/admin/discover resolve + channel endpoints"
```

---

## Task 3: Search endpoints (M3U + YouTube) + 503 ApiError variant

**Files:** modify `src/routes/api/mod.rs` (add `Unavailable` variant + 2 routes), `src/routes/api/discover.rs` (handlers + DTOs), `tests/api.rs`.

- [ ] **Step 1: Write the failing tests** — append to `tests/api.rs`:

```rust
#[tokio::test]
async fn discover_youtube_without_api_key_is_503() {
    // test app() builds config with youtube_api_key: None
    let r = app().await.oneshot(authed_get("/api/admin/discover/youtube?keyword=news")).await.unwrap();
    assert_eq!(r.status(), StatusCode::SERVICE_UNAVAILABLE);
    let j = body_json(r).await;
    assert_eq!(j["error"], "YOUTUBE_API_KEY not configured");
}

#[tokio::test]
async fn discover_youtube_requires_auth() {
    let r = app().await.oneshot(get("/api/admin/discover/youtube?keyword=news")).await.unwrap();
    assert_eq!(r.status(), StatusCode::UNAUTHORIZED);
}
```

(`get` is the no-auth request helper already in `tests/api.rs`; `authed_get` is the authed one. Live M3U/YouTube search is covered by `#[ignore]` tests in Step 6.)

- [ ] **Step 2: Run to verify FAIL**

Run: `cargo test --test api discover_youtube_without_api_key_is_503 discover_youtube_requires_auth`
Expected: FAIL (route absent → 404/401-mismatch).

- [ ] **Step 3: Add the `Unavailable` variant to `ApiError`** — in `src/routes/api/mod.rs`, add a variant and arm:

In the enum:
```rust
pub enum ApiError {
    NotFound,
    Validation(String),
    Unavailable(String),
    Internal,
}
```
In `into_response`, add the arm:
```rust
            ApiError::Unavailable(m) => (StatusCode::SERVICE_UNAVAILABLE, m),
```

- [ ] **Step 4: Add the search handlers + DTOs to `src/routes/api/discover.rs`**

Add imports at the top (extend the existing `use` lines):
```rust
use axum::extract::Query;
use crate::routes::admin::discover::{
    fetch_youtube_channels, fetch_youtube_results, m3u_search, M3uResultRow, YoutubeResultRow,
};
use super::internal;
```

Add DTOs + handlers:
```rust
#[derive(Serialize)]
pub struct M3uCandidate {
    pub name: String,
    pub group: String,
    pub country: String,
    pub url: String,
    pub source_kind: String,
}

impl From<M3uResultRow> for M3uCandidate {
    fn from(r: M3uResultRow) -> Self {
        M3uCandidate { name: r.name, group: r.group, country: r.country, url: r.url, source_kind: r.source_kind }
    }
}

#[derive(Serialize)]
pub struct YoutubeCandidate {
    pub title: String,
    pub channel_title: String,
    pub is_live: bool,
    pub is_upcoming: bool,
    pub duration_secs: i64,
    pub scheduled_start: String,
    pub thumbnail_url: String,
    pub url: String,
    pub source_kind: String,
}

impl From<YoutubeResultRow> for YoutubeCandidate {
    fn from(r: YoutubeResultRow) -> Self {
        YoutubeCandidate {
            title: r.title,
            channel_title: r.channel_title,
            is_live: r.is_live,
            is_upcoming: r.is_upcoming,
            duration_secs: r.duration_secs,
            scheduled_start: r.scheduled_start,
            thumbnail_url: r.thumbnail_url,
            url: r.url,
            source_kind: r.source_kind,
        }
    }
}

#[derive(Deserialize)]
pub struct M3uQuery {
    pub country: Option<String>,
    pub group: Option<String>,
    pub limit: Option<usize>,
}

#[derive(Deserialize)]
pub struct YoutubeQuery {
    pub keyword: String,
    #[serde(rename = "type")]
    pub search_type: Option<String>,
}

const M3U_LIMIT_DEFAULT: usize = 50;
const M3U_LIMIT_MAX: usize = 200;

pub async fn m3u(
    State(state): State<AppState>,
    Query(q): Query<M3uQuery>,
) -> Result<Json<Vec<M3uCandidate>>, ApiError> {
    let limit = q.limit.unwrap_or(M3U_LIMIT_DEFAULT).min(M3U_LIMIT_MAX);
    let rows = m3u_search(
        &state.http_client,
        q.country.as_deref().unwrap_or(""),
        q.group.as_deref().unwrap_or(""),
        limit,
    )
    .await
    .map_err(internal)?;
    Ok(Json(rows.into_iter().map(M3uCandidate::from).collect()))
}

pub async fn youtube(
    State(state): State<AppState>,
    Query(q): Query<YoutubeQuery>,
) -> Result<Json<Vec<YoutubeCandidate>>, ApiError> {
    let api_key = state
        .config
        .youtube_api_key
        .clone()
        .ok_or_else(|| ApiError::Unavailable("YOUTUBE_API_KEY not configured".into()))?;
    let search_type = q.search_type.as_deref().unwrap_or("video");
    let rows = if search_type == "channel" {
        fetch_youtube_channels(&q.keyword, &api_key, &state.http_client).await
    } else {
        fetch_youtube_results(&q.keyword, &api_key, &state.http_client).await
    }
    .map_err(internal)?;
    Ok(Json(rows.into_iter().map(YoutubeCandidate::from).collect()))
}
```

- [ ] **Step 5: Wire the routes in `src/routes/api/mod.rs`**

Extend `api_router()` with:
```rust
        .route("/discover/m3u", get(discover::m3u))
        .route("/discover/youtube", get(discover::youtube))
```
(`get` already imported.)

- [ ] **Step 6: Add the network-gated live tests** — append to `tests/api.rs`:

```rust
#[tokio::test]
#[ignore = "requires network access — run manually"]
async fn discover_m3u_live_search() {
    let r = app().await.oneshot(authed_get("/api/admin/discover/m3u?country=us&group=&limit=5")).await.unwrap();
    assert_eq!(r.status(), StatusCode::OK);
    let j = body_json(r).await;
    assert!(j.as_array().unwrap().len() <= 5);
}
```

- [ ] **Step 7: Run to verify PASS**

Run: `cargo test --test api`
Expected: PASS (new 503/auth tests + Spec 2 + Task 2 tests; the live test is ignored).

- [ ] **Step 8: Format, lint, commit**

```bash
cargo fmt
cargo clippy --all-targets -- -D warnings
git add src/routes/api/discover.rs src/routes/api/mod.rs tests/api.rs
git commit -m "feat: /api/admin/discover m3u + youtube search endpoints (503 when no API key)"
```

---

## Task 4: `discover/add` endpoint

**Files:** modify `src/routes/api/discover.rs`, `src/routes/api/mod.rs`, `tests/api.rs`.

- [ ] **Step 1: Write the failing tests** — append to `tests/api.rs`:

```rust
#[tokio::test]
async fn discover_add_to_existing_channel() {
    let app = app().await;
    let r = app.clone().oneshot(authed_json("POST", "/api/admin/discover/add",
        serde_json::json!({
            "url": "https://cdn.example.com/added.m3u8",
            "title": "Added",
            "source_kind": "hls",
            "channel": {"existing_id": 1}
        }))).await.unwrap();
    assert_eq!(r.status(), StatusCode::CREATED);
    let j = body_json(r).await;
    assert_eq!(j["channel_id"], 1);
    assert_eq!(j["channel"]["id"], 1);

    // the source is now attached to channel 1
    let r = app.oneshot(authed_get("/api/admin/channels/1/sources")).await.unwrap();
    let sources = body_json(r).await;
    assert!(sources.as_array().unwrap().iter().any(|s| s["url"] == "https://cdn.example.com/added.m3u8"));
}

#[tokio::test]
async fn discover_add_creates_new_channel() {
    let app = app().await;
    let r = app.clone().oneshot(authed_json("POST", "/api/admin/discover/add",
        serde_json::json!({
            "url": "https://cdn.example.com/newchan.m3u8",
            "title": "NC",
            "source_kind": "hls",
            "channel": {"new": {"name": "New Discovered", "category": "test", "type": "live"}}
        }))).await.unwrap();
    assert_eq!(r.status(), StatusCode::CREATED);
    let j = body_json(r).await;
    assert_eq!(j["channel"]["name"], "New Discovered");
    let new_id = j["channel_id"].as_i64().unwrap();
    assert!(new_id > 5); // seed has channels 1-5
}

#[tokio::test]
async fn discover_add_unknown_existing_channel_is_404() {
    let r = app().await.oneshot(authed_json("POST", "/api/admin/discover/add",
        serde_json::json!({
            "url": "https://cdn.example.com/x.m3u8", "title": "X", "source_kind": "hls",
            "channel": {"existing_id": 999999}
        }))).await.unwrap();
    assert_eq!(r.status(), StatusCode::NOT_FOUND);
}
```

- [ ] **Step 2: Run to verify FAIL**

Run: `cargo test --test api discover_add_to_existing_channel discover_add_creates_new_channel discover_add_unknown_existing_channel_is_404`
Expected: FAIL (route absent).

- [ ] **Step 3: Add the add handler + DTOs to `src/routes/api/discover.rs`**

Extend the imports:
```rust
use axum::http::StatusCode;
use crate::routes::admin::discover::{do_discover_add, DiscoverAddParams};
use crate::model::channel;
```

Add DTOs + handler:
```rust
#[derive(Deserialize)]
pub struct NewChannelSpec {
    pub name: String,
    pub category: String,
    #[serde(rename = "type")]
    pub channel_type: String,
}

#[derive(Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum ChannelTarget {
    ExistingId(i64),
    New(NewChannelSpec),
}

#[derive(Deserialize)]
pub struct AddRequest {
    pub url: String,
    pub title: String,
    pub source_kind: String,
    #[serde(default)]
    pub duration_secs: i64,
    pub channel: ChannelTarget,
}

#[derive(Serialize)]
pub struct AddResponse {
    pub channel_id: i64,
    pub channel: channel::Channel,
}

pub async fn add(
    State(state): State<AppState>,
    Json(req): Json<AddRequest>,
) -> Result<(StatusCode, Json<AddResponse>), ApiError> {
    // Build the do_discover_add params from the tagged channel target.
    let (channel_choice, new_name, new_category, new_channel_type) = match &req.channel {
        ChannelTarget::ExistingId(id) => (id.to_string(), String::new(), String::new(), "live".to_string()),
        ChannelTarget::New(spec) => (
            "new".to_string(),
            spec.name.clone(),
            spec.category.clone(),
            spec.channel_type.clone(),
        ),
    };

    let channel_id = do_discover_add(DiscoverAddParams {
        pool: &state.pool,
        client: &state.http_client,
        url: &req.url,
        title: &req.title,
        source_kind: &req.source_kind,
        duration_secs: req.duration_secs,
        channel_choice: &channel_choice,
        new_name: &new_name,
        new_category: &new_category,
        new_channel_type: &new_channel_type,
    })
    .await
    .map_err(map_add_status)?;

    let channel = channel::get(&state.pool, channel_id)
        .await
        .map_err(internal)?
        .ok_or(ApiError::NotFound)?;
    Ok((StatusCode::CREATED, Json(AddResponse { channel_id, channel })))
}

/// `do_discover_add` returns a bare StatusCode; map it to ApiError so the JSON
/// error body stays consistent.
fn map_add_status(status: StatusCode) -> ApiError {
    match status {
        StatusCode::NOT_FOUND => ApiError::NotFound,
        StatusCode::UNPROCESSABLE_ENTITY => {
            ApiError::Validation("invalid discover-add request".into())
        }
        _ => ApiError::Internal,
    }
}
```

- [ ] **Step 4: Wire the route in `src/routes/api/mod.rs`**

Extend `api_router()` with:
```rust
        .route("/discover/add", post(discover::add))
```

- [ ] **Step 5: Run to verify PASS**

Run: `cargo test --test api`
Expected: PASS (the 3 new add tests + all earlier api tests).

- [ ] **Step 6: Format, lint, commit**

```bash
cargo fmt
cargo clippy --all-targets -- -D warnings
git add src/routes/api/discover.rs src/routes/api/mod.rs tests/api.rs
git commit -m "feat: /api/admin/discover/add wrapping do_discover_add"
```

---

## Task 5: `mytvctl discover` CLI noun

**Files:** modify `src/bin/mytvctl.rs`.

- [ ] **Step 1: Write the failing unit tests** — add to the `tests` module in `src/bin/mytvctl.rs`:

```rust
    #[test]
    fn request_for_discover_resolve_posts_url() {
        let req = request_for(&Command::Discover(DiscoverCmd::Resolve { url: "https://x/y.m3u8".into() }));
        assert_eq!(req.method, "POST");
        assert_eq!(req.path, "/api/admin/discover/resolve");
        assert_eq!(req.body.unwrap()["url"], "https://x/y.m3u8");
    }

    #[test]
    fn request_for_discover_m3u_builds_get_query() {
        let req = request_for(&Command::Discover(DiscoverCmd::M3u {
            country: "us".into(), group: "News".into(), limit: Some(10),
        }));
        assert_eq!(req.method, "GET");
        assert_eq!(req.path, "/api/admin/discover/m3u?country=us&group=News&limit=10");
        assert!(req.body.is_none());
    }

    #[test]
    fn request_for_discover_add_existing_channel() {
        let req = request_for(&Command::Discover(DiscoverCmd::Add {
            url: "https://x/y.m3u8".into(), title: "T".into(), source_kind: "hls".into(),
            duration_secs: None, channel: Some(1), new_name: None, new_category: None, new_type: None,
        }));
        assert_eq!(req.method, "POST");
        assert_eq!(req.path, "/api/admin/discover/add");
        let body = req.body.unwrap();
        assert_eq!(body["channel"]["existing_id"], 1);
        assert_eq!(body["source_kind"], "hls");
    }

    #[test]
    fn request_for_discover_add_new_channel() {
        let req = request_for(&Command::Discover(DiscoverCmd::Add {
            url: "https://x/y.m3u8".into(), title: "T".into(), source_kind: "hls".into(),
            duration_secs: None, channel: None,
            new_name: Some("NC".into()), new_category: Some("test".into()), new_type: Some("live".into()),
        }));
        let body = req.body.unwrap();
        assert_eq!(body["channel"]["new"]["name"], "NC");
        assert_eq!(body["channel"]["new"]["type"], "live");
    }
```

- [ ] **Step 2: Run to verify FAIL**

Run: `cargo test --bin mytvctl request_for_discover`
Expected: FAIL to compile (DiscoverCmd / Command::Discover undefined).

- [ ] **Step 3: Add the `Discover` command tree** — in `src/bin/mytvctl.rs`, add a variant to the `Command` enum:
```rust
    #[command(subcommand)]
    Discover(DiscoverCmd),
```
And the subcommand enum (place near the other `*Cmd` enums):
```rust
#[derive(Subcommand)]
enum DiscoverCmd {
    M3u {
        #[arg(long, default_value = "")] country: String,
        #[arg(long, default_value = "")] group: String,
        #[arg(long)] limit: Option<usize>,
    },
    Youtube {
        #[arg(long)] keyword: String,
        #[arg(long = "type")] r#type: Option<String>,
    },
    Resolve {
        #[arg(long)] url: String,
    },
    Channel {
        #[arg(long)] url: String,
    },
    Add {
        #[arg(long)] url: String,
        #[arg(long)] title: String,
        #[arg(long)] source_kind: String,
        #[arg(long)] duration_secs: Option<i64>,
        #[arg(long, conflicts_with_all = ["new_name", "new_category", "new_type"])] channel: Option<i64>,
        #[arg(long, requires_all = ["new_category", "new_type"])] new_name: Option<String>,
        #[arg(long)] new_category: Option<String>,
        #[arg(long)] new_type: Option<String>,
    },
}
```

- [ ] **Step 4: Add the `request_for` arm** — inside `request_for`'s `match cmd`, add:
```rust
        Command::Discover(c) => match c {
            DiscoverCmd::M3u { country, group, limit } => {
                let mut qs = format!("country={country}&group={group}");
                if let Some(l) = limit {
                    qs.push_str(&format!("&limit={l}"));
                }
                ApiRequest { method: "GET", path: format!("/api/admin/discover/m3u?{qs}"), body: None }
            }
            DiscoverCmd::Youtube { keyword, r#type } => {
                let mut qs = format!("keyword={keyword}");
                if let Some(t) = r#type {
                    qs.push_str(&format!("&type={t}"));
                }
                ApiRequest { method: "GET", path: format!("/api/admin/discover/youtube?{qs}"), body: None }
            }
            DiscoverCmd::Resolve { url } => ApiRequest {
                method: "POST", path: "/api/admin/discover/resolve".into(),
                body: Some(json!({ "url": url })),
            },
            DiscoverCmd::Channel { url } => ApiRequest {
                method: "POST", path: "/api/admin/discover/channel".into(),
                body: Some(json!({ "url": url })),
            },
            DiscoverCmd::Add { url, title, source_kind, duration_secs, channel, new_name, new_category, new_type } => {
                let channel_val = if let Some(id) = channel {
                    json!({ "existing_id": id })
                } else {
                    json!({ "new": {
                        "name": new_name, "category": new_category, "type": new_type
                    }})
                };
                let mut body = json!({
                    "url": url, "title": title, "source_kind": source_kind, "channel": channel_val
                });
                if let Some(d) = duration_secs {
                    body["duration_secs"] = json!(d);
                }
                ApiRequest { method: "POST", path: "/api/admin/discover/add".into(), body: Some(body) }
            }
        },
```

NOTE: the `m3u`/`youtube` query strings interpolate values raw (no percent-encoding). For the common cases (country codes, simple group/keyword) this is fine; a group/keyword with spaces or `&` would need encoding. Keep it simple for now — if a value needs encoding the user can quote/encode it, and the server reads the decoded query param. (Do not add a urlencoding dependency for this.)

- [ ] **Step 5: Run to verify PASS**

Run: `cargo test --bin mytvctl`
Expected: PASS (the 5 existing CLI tests + 4 new discover tests).

- [ ] **Step 6: Smoke-test**

Run: `cargo run --bin mytvctl -- discover --help`
Expected: clap prints the m3u/youtube/resolve/channel/add subcommands; exit 0.

- [ ] **Step 7: Format, lint, commit**

```bash
cargo fmt
cargo clippy --all-targets -- -D warnings
git add src/bin/mytvctl.rs
git commit -m "feat: add mytvctl discover subcommands (m3u/youtube/resolve/channel/add)"
```

---

## Final verification

- [ ] Run `cargo fmt --check` → no diff.
- [ ] Run `cargo clippy --all-targets -- -D warnings` → exits 0.
- [ ] Run `cargo test` → all pass (ignored: the prior 7 network tests + the new `discover_m3u_live_search`).
- [ ] Update `CLAUDE.md`: refresh the test-count line from the actual `cargo test` output, and extend the `routes/api/` structure note and the JSON-API architecture note to mention `/api/admin/discover/**` (m3u/youtube search, resolve, channel, add) and the `mytvctl discover` noun. Commit:
```bash
git add CLAUDE.md
git commit -m "docs: document /api/admin/discover endpoints + mytvctl discover"
```
