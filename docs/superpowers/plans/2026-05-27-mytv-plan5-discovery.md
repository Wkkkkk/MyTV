# MyTV Plan 5: Discovery Tools Implementation Plan

> **For agentic workers:** REQUIRED SUB-SKILL: Use superpowers:subagent-driven-development (recommended) or superpowers:executing-plans to implement this plan task-by-task. Steps use checkbox (`- [ ]`) syntax for tracking.

**Goal:** Add a `/admin/discover` page with M3U import, YouTube search, and manual URL entry so users can find streams and add them to channels without leaving the admin UI.

**Architecture:** New `src/routes/admin_discover.rs` module wired into the existing auth-protected admin router. Three HTMX-powered tab panels (all rendered server-side, toggled with JS). Results load inline via HTMX POST. A shared "add form" partial handles the new-vs-existing-channel choice and calls existing `channel`, `source`, `playlist_item` DB modules. External HTTP (YouTube API v3, iptv-org M3U) via `reqwest`. Duration auto-fetch for YouTube VOD via existing `resolver::fetch_duration_secs`.

**Tech Stack:** Rust, Axum 0.7, Askama 0.12, sqlx 0.7, reqwest 0.12, htmx 1.9 (added to admin base template), existing `resolver.rs`

---

## File Structure

```
Cargo.toml                                              modify: add reqwest 0.12
src/
  main.rs                                               modify: add http_client to AppState, wire 6 discover routes
  routes/
    mod.rs                                              modify: add pub mod admin_discover
    admin_discover.rs                                   NEW: all discover handlers + pure functions
templates/
  admin/
    base.html                                           modify: add htmx script + tab CSS
    discover.html                                       NEW: page shell, 3 tab panels, JS toggle
    partials/
      discover_add_form.html                            NEW: inline new/existing channel add form
      discover_m3u_results.html                         NEW: M3U search result rows
      discover_yt_results.html                          NEW: YouTube search result rows
      discover_manual_result.html                       NEW: resolved URL metadata + inline add form
```

---

## Routes (all under existing auth middleware)

```
GET  /admin/discover                   — discover page (3 tab panels rendered)
POST /admin/discover/add-form          — inline add form for a single result
POST /admin/discover/m3u/search        — fetch iptv-org M3U, filter, return rows partial
POST /admin/discover/youtube/search    — YouTube API search, return rows partial
POST /admin/discover/manual/resolve    — resolve URL metadata + inline add form
POST /admin/discover/add               — commit add → redirect to /admin/channels/:id
```

---

### Task 1: Dependencies, AppState, routing scaffold, admin htmx

**Files:**
- Modify: `Cargo.toml`
- Modify: `src/main.rs`
- Modify: `src/routes/mod.rs`
- Modify: `templates/admin/base.html`
- Create: `src/routes/admin_discover.rs`

- [ ] **Step 1: Add reqwest to Cargo.toml**

Open `Cargo.toml` and add after `base64 = "0.22"`:
```toml
reqwest = { version = "0.12", features = ["rustls-tls", "json"], default-features = false }
```

- [ ] **Step 2: Add http_client to AppState in src/main.rs**

Replace the `AppState` struct and the state construction in `main()`:

```rust
// Replace existing AppState struct:
#[derive(Clone)]
pub struct AppState {
    pub pool: SqlitePool,
    pub config: Arc<config::Config>,
    pub http_client: reqwest::Client,
}
```

In `main()`, after `let config = Arc::new(...)`:
```rust
let http_client = reqwest::Client::builder()
    .timeout(std::time::Duration::from_secs(10))
    .build()?;

let state = AppState {
    pool,
    config: config.clone(),
    http_client,
};
```

- [ ] **Step 3: Wire discover routes in src/main.rs**

In the `admin_router` definition, add these routes before `.route_layer(...)`:
```rust
.route("/discover", get(routes::admin_discover::discover_page))
.route("/discover/add-form", post(routes::admin_discover::discover_add_form))
.route("/discover/add", post(routes::admin_discover::discover_add))
.route("/discover/m3u/search", post(routes::admin_discover::discover_m3u_search))
.route("/discover/youtube/search", post(routes::admin_discover::discover_youtube_search))
.route("/discover/manual/resolve", post(routes::admin_discover::discover_manual_resolve))
```

The full admin_router block now looks like:
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
    .route("/discover", get(routes::admin_discover::discover_page))
    .route("/discover/add-form", post(routes::admin_discover::discover_add_form))
    .route("/discover/add", post(routes::admin_discover::discover_add))
    .route("/discover/m3u/search", post(routes::admin_discover::discover_m3u_search))
    .route("/discover/youtube/search", post(routes::admin_discover::discover_youtube_search))
    .route("/discover/manual/resolve", post(routes::admin_discover::discover_manual_resolve))
    .route_layer(middleware::from_fn_with_state(
        state.clone(),
        routes::admin::basic_auth,
    ));
```

- [ ] **Step 4: Add pub mod admin_discover to src/routes/mod.rs**

```rust
pub mod admin;
pub mod admin_discover;
pub mod guide;
pub mod health;
pub mod player;
```

- [ ] **Step 5: Create src/routes/admin_discover.rs with stub handlers**

```rust
use askama::Template;
use axum::{
    extract::{Form, State},
    http::StatusCode,
    response::{Html, IntoResponse, Redirect},
};
use chrono::Utc;
use serde::Deserialize;

use crate::{channel, playlist_item, resolver, source, AppState};

// ── pure data types ────────────────────────────────────────────────────────

pub struct M3uChannel {
    pub name: String,
    pub group: String,
    pub country: String,
    pub url: String,
}

pub struct DiscoverChannelOption {
    pub id: i64,
    pub name: String,
    pub type_str: String,
}

pub struct M3uResultRow {
    pub name: String,
    pub group: String,
    pub country: String,
    pub url: String,
    pub source_kind: String,
    pub form_id: usize,
}

pub struct YoutubeResultRow {
    pub title: String,
    pub channel_title: String,
    pub is_live: bool,
    pub duration_secs: i64,
    pub url: String,
    pub form_id: usize,
}

// ── template structs ───────────────────────────────────────────────────────

#[derive(Template)]
#[template(path = "admin/discover.html")]
struct DiscoverPageTemplate {
    youtube_api_key_set: bool,
}

#[derive(Template)]
#[template(path = "admin/partials/discover_add_form.html")]
struct DiscoverAddFormTemplate {
    form_id: String,
    url: String,
    title: String,
    is_live: bool,
    duration_secs: i64,
    source_kind: String,
    show_duration_input: bool,
    channels: Vec<DiscoverChannelOption>,
}

#[derive(Template)]
#[template(path = "admin/partials/discover_m3u_results.html")]
struct M3uResultsTemplate {
    rows: Vec<M3uResultRow>,
}

#[derive(Template)]
#[template(path = "admin/partials/discover_yt_results.html")]
struct YtResultsTemplate {
    rows: Vec<YoutubeResultRow>,
}

#[derive(Template)]
#[template(path = "admin/partials/discover_manual_result.html")]
struct ManualResultTemplate {
    form_id: String,
    url: String,
    title: String,
    is_live: bool,
    duration_secs: i64,
    source_kind: String,
    show_duration_input: bool,
    channels: Vec<DiscoverChannelOption>,
}

// ── form input types ───────────────────────────────────────────────────────

#[derive(Deserialize)]
pub struct M3uSearchForm {
    pub country: String,
    pub group: String,
}

#[derive(Deserialize)]
pub struct YoutubeSearchForm {
    pub keyword: String,
}

#[derive(Deserialize)]
pub struct ManualResolveForm {
    pub url: String,
}

#[derive(Deserialize)]
pub struct AddFormQuery {
    pub url: String,
    pub title: String,
    pub is_live: String,
    pub duration_secs: String,
    pub source_kind: String,
    pub form_id: String,
}

#[derive(Deserialize)]
pub struct AddForm {
    pub url: String,
    pub title: String,
    pub is_live: String,
    pub duration_secs: String,
    pub source_kind: String,
    pub channel_choice: String,
    pub new_name: String,
    pub new_category: String,
    pub new_channel_type: String,
}

// ── helpers ────────────────────────────────────────────────────────────────

fn render<T: askama::Template>(t: T) -> Result<Html<String>, StatusCode> {
    t.render().map(Html).map_err(|e| {
        tracing::error!("template render error: {e}");
        StatusCode::INTERNAL_SERVER_ERROR
    })
}

fn internal_error<E: std::fmt::Display>(e: E) -> StatusCode {
    tracing::error!("discover error: {e}");
    StatusCode::INTERNAL_SERVER_ERROR
}

// ── stub handlers (replaced in later tasks) ───────────────────────────────

pub async fn discover_page(State(state): State<AppState>) -> Result<Html<String>, StatusCode> {
    render(DiscoverPageTemplate {
        youtube_api_key_set: state.config.youtube_api_key.is_some(),
    })
}

pub async fn discover_add_form(
    State(state): State<AppState>,
    Form(form): Form<AddFormQuery>,
) -> Result<Html<String>, StatusCode> {
    let channels = channel::list(&state.pool).await.map_err(internal_error)?
        .into_iter()
        .map(|ch| DiscoverChannelOption { id: ch.id, name: ch.name, type_str: ch.r#type })
        .collect();
    let is_live = form.is_live == "true";
    let duration_secs: i64 = form.duration_secs.parse().unwrap_or(0);
    render(DiscoverAddFormTemplate {
        form_id: form.form_id,
        url: form.url,
        title: form.title,
        is_live,
        duration_secs,
        source_kind: form.source_kind,
        show_duration_input: !is_live && duration_secs == 0,
        channels,
    })
}

pub async fn discover_add(
    State(state): State<AppState>,
    Form(form): Form<AddForm>,
) -> Result<impl IntoResponse, StatusCode> {
    let duration_secs: i64 = form.duration_secs.parse().unwrap_or(0);
    let channel_id = do_discover_add(
        &state.pool,
        &form.url,
        &form.title,
        &form.source_kind,
        duration_secs,
        &form.channel_choice,
        &form.new_name,
        &form.new_category,
        &form.new_channel_type,
    ).await?;
    Ok(Redirect::to(&format!("/admin/channels/{}", channel_id)))
}

pub async fn discover_m3u_search(
    State(state): State<AppState>,
    Form(form): Form<M3uSearchForm>,
) -> Html<String> {
    let raw = match fetch_m3u(&state.http_client).await {
        Ok(r) => r,
        Err(e) => {
            tracing::error!("M3U fetch error: {e}");
            return Html("<p class=\"empty-state\" style=\"color:#f77\">Failed to fetch M3U list. Check server logs.</p>".to_string());
        }
    };
    let all = parse_m3u(&raw);
    let matches = filter_m3u(&all, &form.country, &form.group);
    let rows: Vec<M3uResultRow> = matches.iter().enumerate().map(|(i, ch)| M3uResultRow {
        name: ch.name.clone(),
        group: ch.group.clone(),
        country: ch.country.clone(),
        url: ch.url.clone(),
        source_kind: detect_source_kind(&ch.url).to_string(),
        form_id: i,
    }).collect();
    match (M3uResultsTemplate { rows }).render() {
        Ok(html) => Html(html),
        Err(e) => { tracing::error!("template error: {e}"); Html("<p class=\"empty-state\" style=\"color:#f77\">Render error.</p>".to_string()) }
    }
}

pub async fn discover_youtube_search(
    State(state): State<AppState>,
    Form(form): Form<YoutubeSearchForm>,
) -> Html<String> {
    let api_key = match &state.config.youtube_api_key {
        Some(k) => k.clone(),
        None => return Html("<p class=\"empty-state\" style=\"color:#f77\">YOUTUBE_API_KEY not configured.</p>".to_string()),
    };
    let rows = match fetch_youtube_results(&form.keyword, &api_key, &state.http_client).await {
        Ok(r) => r,
        Err(e) => {
            tracing::error!("YouTube API error: {e}");
            return Html(format!("<p class=\"empty-state\" style=\"color:#f77\">YouTube search failed: {}.</p>", e));
        }
    };
    match (YtResultsTemplate { rows }).render() {
        Ok(html) => Html(html),
        Err(e) => { tracing::error!("template error: {e}"); Html("<p class=\"empty-state\" style=\"color:#f77\">Render error.</p>".to_string()) }
    }
}

pub async fn discover_manual_resolve(
    State(state): State<AppState>,
    Form(form): Form<ManualResolveForm>,
) -> Result<Html<String>, StatusCode> {
    if !form.url.starts_with("http://") && !form.url.starts_with("https://") {
        return Err(StatusCode::UNPROCESSABLE_ENTITY);
    }
    let is_youtube = resolver::needs_resolution(&form.url);
    let duration_secs: i64 = if is_youtube {
        tokio::time::timeout(
            std::time::Duration::from_secs(5),
            resolver::fetch_duration_secs(&form.url),
        ).await.ok().and_then(|r| r.ok()).unwrap_or(0)
    } else {
        0
    };
    let is_live = duration_secs == 0;
    let channels = channel::list(&state.pool).await.map_err(internal_error)?
        .into_iter()
        .map(|ch| DiscoverChannelOption { id: ch.id, name: ch.name, type_str: ch.r#type })
        .collect();
    render(ManualResultTemplate {
        form_id: "manual".to_string(),
        url: form.url.clone(),
        title: form.url.clone(),
        is_live,
        duration_secs,
        source_kind: detect_source_kind(&form.url).to_string(),
        show_duration_input: !is_live && duration_secs == 0,
        channels,
    })
}

// ── pure functions (implemented in Task 2) ────────────────────────────────

pub fn parse_m3u(_input: &str) -> Vec<M3uChannel> { vec![] }
pub fn filter_m3u<'a>(_channels: &'a [M3uChannel], _country: &str, _group: &str) -> Vec<&'a M3uChannel> { vec![] }
pub fn detect_source_kind(_url: &str) -> &'static str { "iptv" }
pub fn parse_iso8601_duration(_s: &str) -> i64 { 0 }

// ── core add logic (implemented in Task 3) ────────────────────────────────

pub async fn do_discover_add(
    _pool: &sqlx::SqlitePool,
    _url: &str,
    _title: &str,
    _source_kind: &str,
    _duration_secs: i64,
    _channel_choice: &str,
    _new_name: &str,
    _new_category: &str,
    _new_channel_type: &str,
) -> Result<i64, StatusCode> {
    Err(StatusCode::NOT_IMPLEMENTED)
}

// ── YouTube fetch (implemented in Task 5) ─────────────────────────────────

async fn fetch_youtube_results(
    _keyword: &str,
    _api_key: &str,
    _client: &reqwest::Client,
) -> anyhow::Result<Vec<YoutubeResultRow>> {
    Ok(vec![])
}

async fn fetch_m3u(_client: &reqwest::Client) -> anyhow::Result<String> {
    Ok(String::new())
}

// ── tests ──────────────────────────────────────────────────────────────────

#[cfg(test)]
mod tests {
    use super::*;
}
```

- [ ] **Step 6: Add htmx script and tab CSS to templates/admin/base.html**

In the `<head>` section, after the `<title>` tag, add:
```html
  <script src="https://unpkg.com/htmx.org@1.9.10" defer></script>
```

In the `<style>` block, after the `.empty-state` rule, add:
```css
    .tabs{display:flex;gap:6px;flex-wrap:wrap;margin-bottom:16px}
    .tab{padding:4px 14px;border-radius:3px;cursor:pointer;border:1px solid #333;
         background:#1a1a1a;color:#999;font-size:0.82rem}
    .tab.active,.tab:hover{background:#e94560;color:#fff;border-color:#e94560}
    .discover-nav{margin-bottom:16px}
```

Also add a "Discover" nav link in the header nav:
```html
    <nav>
      <a href="/guide">← Guide</a>
      <a href="/admin/channels">Channels</a>
      <a href="/admin/discover">Discover</a>
    </nav>
```

- [ ] **Step 7: Create stub templates (so Askama compiles)**

Create `templates/admin/discover.html`:
```html
{% extends "admin/base.html" %}
{% block content %}
<p>Discover stub</p>
{% endblock %}
```

Create `templates/admin/partials/discover_add_form.html`:
```html
<p>Add form stub</p>
```

Create `templates/admin/partials/discover_m3u_results.html`:
```html
<p>M3U results stub</p>
```

Create `templates/admin/partials/discover_yt_results.html`:
```html
<p>YT results stub</p>
```

Create `templates/admin/partials/discover_manual_result.html`:
```html
<p>Manual result stub</p>
```

- [ ] **Step 8: Verify build succeeds**

```bash
cargo build 2>&1
```

Expected: compiles with 0 errors. There may be warnings about unused imports or dead code — those are fine at this stage.

- [ ] **Step 9: Commit**

```bash
git add Cargo.toml Cargo.lock src/main.rs src/routes/mod.rs src/routes/admin_discover.rs templates/admin/base.html templates/admin/discover.html templates/admin/partials/
git commit -m "feat: scaffold discover routes, add reqwest + htmx to admin"
```

---

### Task 2: Pure functions — M3U parser, source kind detection, ISO 8601 duration

**Files:**
- Modify: `src/routes/admin_discover.rs`

- [ ] **Step 1: Write failing tests for pure functions**

Replace the `#[cfg(test)]` block in `src/routes/admin_discover.rs` with:

```rust
#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn test_parse_m3u_single_channel() {
        let input = "#EXTM3U\n#EXTINF:-1 group-title=\"News\" country=\"US\",CNN\nhttps://example.com/cnn.m3u8\n";
        let result = parse_m3u(input);
        assert_eq!(result.len(), 1);
        assert_eq!(result[0].name, "CNN");
        assert_eq!(result[0].group, "News");
        assert_eq!(result[0].country, "US");
        assert_eq!(result[0].url, "https://example.com/cnn.m3u8");
    }

    #[test]
    fn test_parse_m3u_missing_optional_attrs() {
        let input = "#EXTINF:-1,MyChannel\nhttps://example.com/stream.m3u8\n";
        let result = parse_m3u(input);
        assert_eq!(result.len(), 1);
        assert_eq!(result[0].name, "MyChannel");
        assert_eq!(result[0].group, "");
        assert_eq!(result[0].country, "");
    }

    #[test]
    fn test_parse_m3u_skips_entry_without_url() {
        // First EXTINF is immediately followed by another EXTINF (no URL line)
        let input = "#EXTINF:-1,CNN\n#EXTINF:-1,ESPN\nhttps://espn.com/stream.m3u8\n";
        let result = parse_m3u(input);
        assert_eq!(result.len(), 1);
        assert_eq!(result[0].name, "ESPN");
    }

    #[test]
    fn test_parse_m3u_multiple_channels() {
        let input = concat!(
            "#EXTM3U\n",
            "#EXTINF:-1 group-title=\"News\" country=\"US\",CNN\nhttps://cnn.com/stream.m3u8\n",
            "#EXTINF:-1 group-title=\"Sports\" country=\"US\",ESPN\nhttps://espn.com/stream.m3u8\n",
        );
        let result = parse_m3u(input);
        assert_eq!(result.len(), 2);
    }

    #[test]
    fn test_filter_m3u_by_country_case_insensitive() {
        let channels = vec![
            M3uChannel { name: "CNN".into(), group: "News".into(), country: "US".into(), url: "https://a.com".into() },
            M3uChannel { name: "BBC".into(), group: "News".into(), country: "UK".into(), url: "https://b.com".into() },
        ];
        let result = filter_m3u(&channels, "us", "");
        assert_eq!(result.len(), 1);
        assert_eq!(result[0].name, "CNN");
    }

    #[test]
    fn test_filter_m3u_by_group_case_insensitive() {
        let channels = vec![
            M3uChannel { name: "CNN".into(), group: "News".into(), country: "US".into(), url: "https://a.com".into() },
            M3uChannel { name: "ESPN".into(), group: "Sports".into(), country: "US".into(), url: "https://b.com".into() },
        ];
        let result = filter_m3u(&channels, "", "sports");
        assert_eq!(result.len(), 1);
        assert_eq!(result[0].name, "ESPN");
    }

    #[test]
    fn test_filter_m3u_no_filter_capped_at_50() {
        let channels: Vec<M3uChannel> = (0..60).map(|i| M3uChannel {
            name: format!("Ch{}", i), group: "Test".into(),
            country: "US".into(), url: format!("https://example.com/{}", i),
        }).collect();
        let result = filter_m3u(&channels, "", "");
        assert_eq!(result.len(), 50);
    }

    #[test]
    fn test_detect_source_kind() {
        assert_eq!(detect_source_kind("https://www.youtube.com/watch?v=abc"), "youtube_live");
        assert_eq!(detect_source_kind("https://youtu.be/abc"), "youtube_live");
        assert_eq!(detect_source_kind("https://example.com/stream.m3u8"), "hls");
        assert_eq!(detect_source_kind("https://iptv.example.com/channel/1"), "iptv");
    }

    #[test]
    fn test_parse_iso8601_duration() {
        assert_eq!(parse_iso8601_duration("PT4M13S"), 253);
        assert_eq!(parse_iso8601_duration("PT1H30M"), 5400);
        assert_eq!(parse_iso8601_duration("PT2H"), 7200);
        assert_eq!(parse_iso8601_duration("PT0S"), 0);
        assert_eq!(parse_iso8601_duration("PT45S"), 45);
    }
}
```

- [ ] **Step 2: Run tests to confirm they fail**

```bash
cargo test routes::admin_discover::tests -- --test-threads=1 2>&1 | tail -20
```

Expected: several FAILED (stubs return empty/0).

- [ ] **Step 3: Implement parse_m3u**

Replace the stub `pub fn parse_m3u(_input: &str) -> Vec<M3uChannel> { vec![] }` with:

```rust
pub fn parse_m3u(input: &str) -> Vec<M3uChannel> {
    fn extract_attr(line: &str, attr: &str) -> String {
        let key = format!("{}=\"", attr);
        line.find(&key)
            .map(|start| {
                let after = &line[start + key.len()..];
                after.find('"').map(|end| after[..end].to_string()).unwrap_or_default()
            })
            .unwrap_or_default()
    }

    let mut channels = Vec::new();
    let mut lines = input.lines().peekable();
    while let Some(line) = lines.next() {
        if !line.starts_with("#EXTINF:") {
            continue;
        }
        let name = line.rsplit(',').next().unwrap_or("").trim().to_string();
        let group = extract_attr(line, "group-title");
        let country = extract_attr(line, "country");
        let url = loop {
            match lines.peek() {
                Some(next) if next.trim().is_empty() => { lines.next(); }
                Some(next) if next.starts_with('#') => break String::new(),
                Some(next) => { let u = next.trim().to_string(); lines.next(); break u; }
                None => break String::new(),
            }
        };
        if !url.is_empty() {
            channels.push(M3uChannel { name, group, country, url });
        }
    }
    channels
}
```

- [ ] **Step 4: Implement filter_m3u**

Replace the stub `pub fn filter_m3u<'a>(...) -> Vec<...> { vec![] }` with:

```rust
pub fn filter_m3u<'a>(
    channels: &'a [M3uChannel],
    country: &str,
    group: &str,
) -> Vec<&'a M3uChannel> {
    let country_lower = country.trim().to_lowercase();
    let group_lower = group.trim().to_lowercase();
    channels
        .iter()
        .filter(|ch| {
            let country_ok = country_lower.is_empty()
                || ch.country.to_lowercase().contains(&country_lower);
            let group_ok = group_lower.is_empty()
                || ch.group.to_lowercase().contains(&group_lower);
            country_ok && group_ok
        })
        .take(50)
        .collect()
}
```

- [ ] **Step 5: Implement detect_source_kind**

Replace the stub with:

```rust
pub fn detect_source_kind(url: &str) -> &'static str {
    if url.contains("youtube.com") || url.contains("youtu.be") {
        "youtube_live"
    } else if url.contains(".m3u8") {
        "hls"
    } else {
        "iptv"
    }
}
```

- [ ] **Step 6: Implement parse_iso8601_duration**

Replace the stub with:

```rust
pub fn parse_iso8601_duration(s: &str) -> i64 {
    let s = s.strip_prefix("PT").unwrap_or(s);
    let mut total = 0i64;
    let mut current = String::new();
    for ch in s.chars() {
        match ch {
            '0'..='9' => current.push(ch),
            'H' => { total += current.parse::<i64>().unwrap_or(0) * 3600; current.clear(); }
            'M' => { total += current.parse::<i64>().unwrap_or(0) * 60; current.clear(); }
            'S' => { total += current.parse::<i64>().unwrap_or(0); current.clear(); }
            _ => current.clear(),
        }
    }
    total
}
```

- [ ] **Step 7: Run tests to confirm they pass**

```bash
cargo test routes::admin_discover::tests -- --test-threads=1 2>&1 | tail -15
```

Expected: `9 passed; 0 failed`

- [ ] **Step 8: Commit**

```bash
git add src/routes/admin_discover.rs
git commit -m "feat: implement M3U parser, source kind detection, ISO 8601 duration parsing"
```

---

### Task 3: do_discover_add helper + add form handler (core business logic)

**Files:**
- Modify: `src/routes/admin_discover.rs`
- Create: `templates/admin/partials/discover_add_form.html`

- [ ] **Step 1: Write failing integration tests for do_discover_add**

Add these tests inside the `mod tests` block in `src/routes/admin_discover.rs`, after the pure function tests:

```rust
    use crate::{channel, db, playlist_item, source};
    use axum::http::StatusCode;
    use chrono::Utc;

    async fn test_pool() -> sqlx::SqlitePool {
        db::connect("sqlite::memory:").await.unwrap()
    }

    #[tokio::test]
    async fn test_add_new_live_channel_creates_source() {
        let pool = test_pool().await;
        let ch_id = do_discover_add(
            &pool, "https://example.com/s.m3u8", "CNN", "hls", 0,
            "new", "CNN", "news", "live",
        ).await.unwrap();
        let ch = channel::get(&pool, ch_id).await.unwrap().unwrap();
        assert_eq!(ch.r#type, "live");
        let sources = source::list_for_channel(&pool, ch_id).await.unwrap();
        assert_eq!(sources.len(), 1);
        assert_eq!(sources[0].kind, "hls");
        assert_eq!(sources[0].url, "https://example.com/s.m3u8");
    }

    #[tokio::test]
    async fn test_add_new_vod_channel_creates_playlist_item() {
        let pool = test_pool().await;
        let ch_id = do_discover_add(
            &pool, "https://example.com/ep1.mp4", "Ep 1", "hls", 3600,
            "new", "My Show", "entertainment", "vod_loop",
        ).await.unwrap();
        let ch = channel::get(&pool, ch_id).await.unwrap().unwrap();
        assert_eq!(ch.r#type, "vod_loop");
        assert!(ch.loop_anchor.is_some());
        let items = playlist_item::list_for_channel(&pool, ch_id).await.unwrap();
        assert_eq!(items.len(), 1);
        assert_eq!(items[0].duration_secs, 3600);
        assert_eq!(items[0].title, "Ep 1");
    }

    #[tokio::test]
    async fn test_add_source_to_existing_live_channel() {
        let pool = test_pool().await;
        let existing = channel::create(&pool, channel::NewChannel {
            name: "Existing".into(), category: "news".into(), logo_url: None,
            channel_type: "live".into(), sort_order: 0, loop_anchor: None,
        }).await.unwrap();
        let ch_id = do_discover_add(
            &pool, "https://example.com/s.m3u8", "Existing", "iptv", 0,
            &existing.id.to_string(), "", "", "",
        ).await.unwrap();
        assert_eq!(ch_id, existing.id);
        let sources = source::list_for_channel(&pool, ch_id).await.unwrap();
        assert_eq!(sources.len(), 1);
        assert_eq!(sources[0].kind, "iptv");
    }

    #[tokio::test]
    async fn test_add_playlist_item_to_existing_vod_channel() {
        let pool = test_pool().await;
        let existing = channel::create(&pool, channel::NewChannel {
            name: "VOD".into(), category: "movies".into(), logo_url: None,
            channel_type: "vod_loop".into(), sort_order: 0, loop_anchor: Some(Utc::now()),
        }).await.unwrap();
        let ch_id = do_discover_add(
            &pool, "https://example.com/movie.mp4", "Movie", "hls", 5400,
            &existing.id.to_string(), "", "", "",
        ).await.unwrap();
        let items = playlist_item::list_for_channel(&pool, ch_id).await.unwrap();
        assert_eq!(items.len(), 1);
        assert_eq!(items[0].duration_secs, 5400);
    }

    #[tokio::test]
    async fn test_add_returns_422_when_new_name_empty() {
        let pool = test_pool().await;
        let result = do_discover_add(
            &pool, "https://example.com/s.m3u8", "Test", "hls", 0,
            "new", "", "news", "live",
        ).await;
        assert_eq!(result, Err(StatusCode::UNPROCESSABLE_ENTITY));
    }

    #[tokio::test]
    async fn test_add_returns_422_when_vod_duration_zero() {
        let pool = test_pool().await;
        let result = do_discover_add(
            &pool, "https://example.com/v.mp4", "Test", "hls", 0,
            "new", "Show", "movies", "vod_loop",
        ).await;
        assert_eq!(result, Err(StatusCode::UNPROCESSABLE_ENTITY));
    }
```

- [ ] **Step 2: Run tests to confirm they fail**

```bash
cargo test routes::admin_discover::tests::test_add -- --test-threads=1 2>&1 | tail -15
```

Expected: all `test_add_*` tests FAIL (stub returns `NOT_IMPLEMENTED`).

- [ ] **Step 3: Implement do_discover_add**

Replace the stub `do_discover_add` function with:

```rust
pub async fn do_discover_add(
    pool: &sqlx::SqlitePool,
    url: &str,
    title: &str,
    source_kind: &str,
    duration_secs: i64,
    channel_choice: &str,
    new_name: &str,
    new_category: &str,
    new_channel_type: &str,
) -> Result<i64, StatusCode> {
    if url.is_empty() || (!url.starts_with("http://") && !url.starts_with("https://")) {
        return Err(StatusCode::UNPROCESSABLE_ENTITY);
    }
    if !["hls", "youtube_live", "iptv"].contains(&source_kind) {
        return Err(StatusCode::UNPROCESSABLE_ENTITY);
    }

    let channel_id = if channel_choice == "new" {
        if new_name.trim().is_empty() {
            return Err(StatusCode::UNPROCESSABLE_ENTITY);
        }
        if new_category.trim().is_empty() {
            return Err(StatusCode::UNPROCESSABLE_ENTITY);
        }
        if !["live", "vod_loop"].contains(&new_channel_type) {
            return Err(StatusCode::UNPROCESSABLE_ENTITY);
        }
        let loop_anchor = if new_channel_type == "vod_loop" { Some(Utc::now()) } else { None };
        let ch = channel::create(pool, channel::NewChannel {
            name: new_name.trim().to_string(),
            category: new_category.trim().to_string(),
            logo_url: None,
            channel_type: new_channel_type.to_string(),
            sort_order: 0,
            loop_anchor,
        }).await.map_err(internal_error)?;
        ch.id
    } else {
        channel_choice.parse::<i64>().map_err(|_| StatusCode::UNPROCESSABLE_ENTITY)?
    };

    let ch = channel::get(pool, channel_id)
        .await
        .map_err(internal_error)?
        .ok_or(StatusCode::NOT_FOUND)?;

    if ch.channel_type() == channel::ChannelType::VodLoop {
        if duration_secs <= 0 {
            return Err(StatusCode::UNPROCESSABLE_ENTITY);
        }
        let items = playlist_item::list_for_channel(pool, channel_id)
            .await.map_err(internal_error)?;
        playlist_item::create(pool, playlist_item::NewPlaylistItem {
            channel_id,
            title: title.to_string(),
            url: url.to_string(),
            duration_secs,
            sort_order: items.len() as i64,
        }).await.map_err(internal_error)?;
    } else {
        source::create(pool, source::NewSource {
            channel_id,
            kind: source_kind.to_string(),
            url: url.to_string(),
            priority: 0,
        }).await.map_err(internal_error)?;
    }

    Ok(channel_id)
}
```

- [ ] **Step 4: Run tests to confirm they pass**

```bash
cargo test routes::admin_discover::tests::test_add -- --test-threads=1 2>&1 | tail -15
```

Expected: `6 passed; 0 failed`

- [ ] **Step 5: Write discover_add_form.html template**

Replace `templates/admin/partials/discover_add_form.html` with:

```html
<form action="/admin/discover/add" method="post"
      style="border-top:1px solid #222;padding-top:12px;margin-top:4px">
  <input type="hidden" name="url" value="{{ url }}">
  <input type="hidden" name="is_live" value="{% if is_live %}true{% else %}false{% endif %}">
  <input type="hidden" name="source_kind" value="{{ source_kind }}">

  <div class="form-row" style="margin-bottom:10px">
    <label>Title</label>
    <input type="text" name="title" value="{{ title }}">
  </div>

  {% if show_duration_input %}
  <div class="form-row" style="margin-bottom:10px">
    <label>Duration (seconds)</label>
    <input type="number" name="duration_secs" min="1" required placeholder="e.g. 3600" style="width:140px">
  </div>
  {% else %}
  <input type="hidden" name="duration_secs" value="{{ duration_secs }}">
  {% endif %}

  <div class="form-row" style="margin-bottom:10px">
    <label>Add to</label>
    <select name="channel_choice" style="width:auto" onchange="discoverToggle(this.value,'{{ form_id }}')">
      <option value="new">— New Channel —</option>
      {% for ch in channels %}
      <option value="{{ ch.id }}">{{ ch.name }} ({{ ch.type_str }})</option>
      {% endfor %}
    </select>
  </div>

  <div id="new-ch-{{ form_id }}" style="padding-left:12px;border-left:2px solid #222;margin-bottom:10px">
    <div class="form-row" style="margin-bottom:8px">
      <label>Channel Name</label>
      <input type="text" name="new_name" value="{{ title }}">
    </div>
    <div class="form-row" style="margin-bottom:8px">
      <label>Category</label>
      <input type="text" name="new_category" placeholder="news, sports, movies…">
    </div>
    <div class="form-row" style="margin-bottom:0">
      <label>Type</label>
      <select name="new_channel_type" style="width:auto">
        {% if is_live %}
        <option value="live">live</option>
        <option value="vod_loop">vod_loop</option>
        {% else %}
        <option value="vod_loop">vod_loop</option>
        <option value="live">live</option>
        {% endif %}
      </select>
    </div>
  </div>

  <div id="exist-ch-{{ form_id }}" style="display:none;margin-bottom:10px">
    <input type="hidden" name="new_name" value="">
    <input type="hidden" name="new_category" value="">
    <input type="hidden" name="new_channel_type" value="live">
  </div>

  <button class="btn btn-primary btn-sm" type="submit">Add to Channel →</button>
</form>
```

- [ ] **Step 6: Run full test suite**

```bash
cargo test -- --test-threads=1 2>&1 | tail -5
```

Expected: all tests pass (count increases from 65 to 74).

- [ ] **Step 7: Commit**

```bash
git add src/routes/admin_discover.rs templates/admin/partials/discover_add_form.html
git commit -m "feat: implement do_discover_add with 6 integration tests, add form template"
```

---

### Task 4: Discover page shell + M3U tab

**Files:**
- Modify: `src/routes/admin_discover.rs` (replace fetch_m3u stub)
- Replace: `templates/admin/discover.html`
- Replace: `templates/admin/partials/discover_m3u_results.html`

- [ ] **Step 1: Implement fetch_m3u**

Replace the stub `async fn fetch_m3u(...)` with:

```rust
async fn fetch_m3u(client: &reqwest::Client) -> anyhow::Result<String> {
    let text = client
        .get("https://iptv-org.github.io/iptv/index.m3u")
        .send()
        .await?
        .error_for_status()?
        .text()
        .await?;
    Ok(text)
}
```

- [ ] **Step 2: Write discover.html**

Replace `templates/admin/discover.html` with:

```html
{% extends "admin/base.html" %}
{% block content %}
<div class="page-header">
  <h2>Discover Channels</h2>
</div>

<div class="tabs">
  <a class="tab active" id="tab-btn-m3u" href="javascript:void(0)" onclick="showDiscover('m3u')">M3U Import</a>
  <a class="tab" id="tab-btn-youtube" href="javascript:void(0)" onclick="showDiscover('youtube')">YouTube</a>
  <a class="tab" id="tab-btn-manual" href="javascript:void(0)" onclick="showDiscover('manual')">Manual URL</a>
</div>

<!-- M3U Tab -->
<div id="tab-panel-m3u">
  <form hx-post="/admin/discover/m3u/search"
        hx-target="#m3u-results"
        hx-swap="innerHTML"
        style="display:flex;gap:10px;flex-wrap:wrap;align-items:flex-end;margin-bottom:16px">
    <div class="form-row" style="margin:0;min-width:140px">
      <label>Country</label>
      <input type="text" name="country" placeholder="e.g. US">
    </div>
    <div class="form-row" style="margin:0;min-width:160px">
      <label>Category / Group</label>
      <input type="text" name="group" placeholder="e.g. News">
    </div>
    <button class="btn btn-primary btn-sm" type="submit" style="margin-bottom:1px">Search</button>
  </form>
  <p class="empty-state" style="font-size:0.8rem">Enter a country or category filter, then click Search. Returns up to 50 channels from the iptv-org public index.</p>
  <div id="m3u-results"></div>
</div>

<!-- YouTube Tab -->
<div id="tab-panel-youtube" style="display:none">
  {% if youtube_api_key_set %}
  <form hx-post="/admin/discover/youtube/search"
        hx-target="#yt-results"
        hx-swap="innerHTML"
        style="display:flex;gap:10px;align-items:flex-end;margin-bottom:16px">
    <div class="form-row" style="margin:0;flex:1;min-width:200px">
      <label>Keyword</label>
      <input type="text" name="keyword" placeholder="search YouTube…">
    </div>
    <button class="btn btn-primary btn-sm" type="submit" style="margin-bottom:1px">Search</button>
  </form>
  <div id="yt-results"></div>
  {% else %}
  <p class="empty-state" style="padding:20px 0">
    Set the <code style="background:#1a1a1a;padding:2px 6px;border-radius:3px">YOUTUBE_API_KEY</code>
    environment variable to enable YouTube search.
  </p>
  {% endif %}
</div>

<!-- Manual Tab -->
<div id="tab-panel-manual" style="display:none">
  <form hx-post="/admin/discover/manual/resolve"
        hx-target="#manual-result"
        hx-swap="innerHTML"
        style="display:flex;gap:10px;align-items:flex-end;margin-bottom:16px">
    <div class="form-row" style="margin:0;flex:1;min-width:260px">
      <label>Stream URL (HLS, IPTV, or YouTube)</label>
      <input type="text" name="url" placeholder="https://…" required>
    </div>
    <button class="btn btn-primary btn-sm" type="submit" style="margin-bottom:1px">Resolve</button>
  </form>
  <div id="manual-result"></div>
</div>

<script>
function showDiscover(tab) {
  ['m3u', 'youtube', 'manual'].forEach(function(t) {
    document.getElementById('tab-panel-' + t).style.display = t === tab ? '' : 'none';
    document.getElementById('tab-btn-' + t).classList.toggle('active', t === tab);
  });
}
function discoverToggle(value, formId) {
  var newDiv = document.getElementById('new-ch-' + formId);
  var existDiv = document.getElementById('exist-ch-' + formId);
  if (value === 'new') {
    if (newDiv) newDiv.style.display = '';
    if (existDiv) existDiv.style.display = 'none';
  } else {
    if (newDiv) newDiv.style.display = 'none';
    if (existDiv) existDiv.style.display = '';
  }
}
</script>
{% endblock %}
```

- [ ] **Step 3: Write discover_m3u_results.html**

Replace `templates/admin/partials/discover_m3u_results.html` with:

```html
{% if rows.is_empty() %}
<p class="empty-state">No channels matched — try different filters.</p>
{% else %}
<p style="font-size:0.78rem;color:#555;margin-bottom:8px">{{ rows.len() }} results (max 50)</p>
<table>
  <thead>
    <tr><th>Name</th><th>Group</th><th>Country</th><th style="max-width:260px">URL</th><th></th></tr>
  </thead>
  <tbody>
    {% for row in rows %}
    <tr>
      <td>{{ row.name }}</td>
      <td style="color:#777">{{ row.group }}</td>
      <td style="color:#777">{{ row.country }}</td>
      <td style="font-size:0.75rem;word-break:break-all;max-width:260px;color:#555">{{ row.url }}</td>
      <td style="white-space:nowrap">
        <form hx-post="/admin/discover/add-form"
              hx-target="#add-form-{{ row.form_id }}"
              hx-swap="innerHTML"
              style="display:inline">
          <input type="hidden" name="url" value="{{ row.url }}">
          <input type="hidden" name="title" value="{{ row.name }}">
          <input type="hidden" name="is_live" value="true">
          <input type="hidden" name="duration_secs" value="0">
          <input type="hidden" name="source_kind" value="{{ row.source_kind }}">
          <input type="hidden" name="form_id" value="{{ row.form_id }}">
          <button type="submit" class="btn btn-primary btn-sm">Add</button>
        </form>
      </td>
    </tr>
    <tr>
      <td colspan="5" style="padding:0">
        <div id="add-form-{{ row.form_id }}" style="padding:10px 14px;background:#080810"></div>
      </td>
    </tr>
    {% endfor %}
  </tbody>
</table>
{% endif %}
```

- [ ] **Step 4: Verify build**

```bash
cargo build 2>&1 | grep -E "error|warning: unused" | head -20
```

Expected: 0 errors.

- [ ] **Step 5: Commit**

```bash
git add src/routes/admin_discover.rs templates/admin/discover.html templates/admin/partials/discover_m3u_results.html
git commit -m "feat: discover page shell + M3U import tab"
```

---

### Task 5: YouTube search tab

**Files:**
- Modify: `src/routes/admin_discover.rs` (replace fetch_youtube_results stub)
- Replace: `templates/admin/partials/discover_yt_results.html`

- [ ] **Step 1: Implement fetch_youtube_results**

Replace the stub `async fn fetch_youtube_results(...)` with:

```rust
async fn fetch_youtube_results(
    keyword: &str,
    api_key: &str,
    client: &reqwest::Client,
) -> anyhow::Result<Vec<YoutubeResultRow>> {
    // Search request
    let search_resp: serde_json::Value = client
        .get("https://www.googleapis.com/youtube/v3/search")
        .query(&[
            ("part", "snippet"),
            ("type", "video"),
            ("maxResults", "12"),
            ("q", keyword),
            ("key", api_key),
        ])
        .send()
        .await?
        .json()
        .await?;

    if let Some(err) = search_resp.get("error") {
        let msg = err["message"].as_str().unwrap_or("YouTube API error").to_string();
        anyhow::bail!("{}", msg);
    }

    let items = match search_resp["items"].as_array() {
        Some(v) => v,
        None => return Ok(vec![]),
    };

    let video_ids: Vec<&str> = items
        .iter()
        .filter_map(|item| item["id"]["videoId"].as_str())
        .collect();

    if video_ids.is_empty() {
        return Ok(vec![]);
    }

    // Batch fetch durations
    let ids_joined = video_ids.join(",");
    let details_resp: serde_json::Value = client
        .get("https://www.googleapis.com/youtube/v3/videos")
        .query(&[("part", "contentDetails"), ("id", ids_joined.as_str()), ("key", api_key)])
        .send()
        .await?
        .json()
        .await?;

    let mut duration_map = std::collections::HashMap::<String, i64>::new();
    if let Some(detail_items) = details_resp["items"].as_array() {
        for item in detail_items {
            let id = item["id"].as_str().unwrap_or("").to_string();
            let dur_str = item["contentDetails"]["duration"].as_str().unwrap_or("PT0S");
            duration_map.insert(id, parse_iso8601_duration(dur_str));
        }
    }

    let rows = items
        .iter()
        .enumerate()
        .filter_map(|(i, item)| {
            let video_id = item["id"]["videoId"].as_str()?;
            let snippet = &item["snippet"];
            let title = snippet["title"].as_str().unwrap_or("Unknown").to_string();
            let channel_title = snippet["channelTitle"].as_str().unwrap_or("").to_string();
            let is_live = snippet["liveBroadcastContent"].as_str() == Some("live");
            let duration_secs = *duration_map.get(video_id).unwrap_or(&0);
            let url = format!("https://www.youtube.com/watch?v={}", video_id);
            Some(YoutubeResultRow { title, channel_title, is_live, duration_secs, url, form_id: i })
        })
        .collect();

    Ok(rows)
}
```

- [ ] **Step 2: Write discover_yt_results.html**

Replace `templates/admin/partials/discover_yt_results.html` with:

```html
{% if rows.is_empty() %}
<p class="empty-state">No results found.</p>
{% else %}
<table>
  <thead>
    <tr><th>Title</th><th>Channel</th><th>Type</th><th>Duration</th><th></th></tr>
  </thead>
  <tbody>
    {% for row in rows %}
    <tr>
      <td>{{ row.title }}</td>
      <td style="color:#777;font-size:0.8rem">{{ row.channel_title }}</td>
      <td>
        {% if row.is_live %}
        <span class="badge badge-live">LIVE</span>
        {% else %}
        <span class="badge badge-vod">VOD</span>
        {% endif %}
      </td>
      <td style="white-space:nowrap;color:#777;font-size:0.8rem">
        {% if row.duration_secs > 0 %}{{ row.duration_secs }}s{% else %}—{% endif %}
      </td>
      <td style="white-space:nowrap">
        <form hx-post="/admin/discover/add-form"
              hx-target="#yt-add-form-{{ row.form_id }}"
              hx-swap="innerHTML"
              style="display:inline">
          <input type="hidden" name="url" value="{{ row.url }}">
          <input type="hidden" name="title" value="{{ row.title }}">
          <input type="hidden" name="is_live" value="{% if row.is_live %}true{% else %}false{% endif %}">
          <input type="hidden" name="duration_secs" value="{{ row.duration_secs }}">
          <input type="hidden" name="source_kind" value="youtube_live">
          <input type="hidden" name="form_id" value="yt{{ row.form_id }}">
          <button type="submit" class="btn btn-primary btn-sm">Add</button>
        </form>
      </td>
    </tr>
    <tr>
      <td colspan="5" style="padding:0">
        <div id="yt-add-form-{{ row.form_id }}" style="padding:10px 14px;background:#080810"></div>
      </td>
    </tr>
    {% endfor %}
  </tbody>
</table>
{% endif %}
```

- [ ] **Step 3: Verify build**

```bash
cargo build 2>&1 | grep error | head -10
```

Expected: 0 errors.

- [ ] **Step 4: Commit**

```bash
git add src/routes/admin_discover.rs templates/admin/partials/discover_yt_results.html
git commit -m "feat: YouTube search tab with API v3 integration"
```

---

### Task 6: Manual entry tab + final verification

**Files:**
- Replace: `templates/admin/partials/discover_manual_result.html`
- Run: full test suite

- [ ] **Step 1: Write discover_manual_result.html**

The manual result template uses `{% include %}` to embed the add form. All fields required by `discover_add_form.html` are present in `ManualResultTemplate`.

Replace `templates/admin/partials/discover_manual_result.html` with:

```html
<div style="border:1px solid #1e2030;border-radius:4px;padding:12px;margin-bottom:4px;background:#0a0a18">
  <div style="font-size:0.78rem;color:#555;margin-bottom:6px;word-break:break-all">{{ url }}</div>
  <div style="display:flex;gap:12px;align-items:center;font-size:0.8rem">
    {% if is_live %}
    <span class="badge badge-live">Live stream</span>
    {% else %}
    <span class="badge badge-vod">VOD</span>
    {% endif %}
    {% if duration_secs > 0 %}
    <span style="color:#666">{{ duration_secs }}s</span>
    {% endif %}
    <span style="color:#444">kind: {{ source_kind }}</span>
  </div>
</div>
{% include "admin/partials/discover_add_form.html" %}
```

- [ ] **Step 2: Run full test suite**

```bash
cargo test -- --test-threads=1 2>&1 | tail -8
```

Expected: all tests pass. Count should be 74 (65 original + 9 new from Tasks 2 and 3).

- [ ] **Step 3: Verify cargo build is clean**

```bash
cargo build 2>&1 | grep "^error" | wc -l
```

Expected: `0`

- [ ] **Step 4: Commit**

```bash
git add templates/admin/partials/discover_manual_result.html
git commit -m "feat: manual URL entry tab, complete discover page"
```

---

## Self-Review

**Spec coverage:**

| Spec requirement | Task |
|---|---|
| `reqwest` added to Cargo.toml | Task 1 |
| `youtube_api_key: Option<String>` in config | Already exists (Plan 4) |
| `GET /admin/discover` page with 3 tabs | Task 4 |
| `POST /admin/discover/m3u/search` | Task 4 |
| `POST /admin/discover/youtube/search` | Task 5 |
| `POST /admin/discover/manual/resolve` | Task 6 |
| `POST /admin/discover/add-form` | Task 3 |
| `POST /admin/discover/add` | Task 3 |
| M3U parser: name/group/country/url | Task 2 |
| M3U filter: case-insensitive, max 50 | Task 2 |
| `detect_source_kind` | Task 2 |
| `parse_iso8601_duration` | Task 2 |
| YouTube "not configured" message | Task 4 (template) |
| 10s timeout on external HTTP | Task 1 (reqwest client) |
| 5s timeout on yt-dlp in manual resolve | Task 1 (handler) |
| `do_discover_add`: 4 paths | Task 3 |
| Validation: 422 on empty name/zero duration | Task 3 |
| Redirect to `/admin/channels/:id` on success | Task 3 |
| htmx in admin base | Task 1 |
| Discover link in admin nav | Task 1 |

**Placeholder scan:** No TBDs. All code is complete.

**Type consistency:** `YoutubeResultRow.form_id` is `usize` — template uses `{{ row.form_id }}` ✓. `M3uResultRow.form_id` is `usize` ✓. `DiscoverAddFormTemplate.form_id` is `String`, matches `AddFormQuery.form_id: String` ✓. `do_discover_add` signature matches all 6 test call sites ✓.
