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

// ── handlers ──────────────────────────────────────────────────────────────

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

// ── pure functions (stubs — implemented in Task 2) ────────────────────────

pub fn parse_m3u(_input: &str) -> Vec<M3uChannel> { vec![] }

pub fn filter_m3u<'a>(_channels: &'a [M3uChannel], _country: &str, _group: &str) -> Vec<&'a M3uChannel> { vec![] }

pub fn detect_source_kind(_url: &str) -> &'static str { "iptv" }

pub fn parse_iso8601_duration(_s: &str) -> i64 { 0 }

// ── core add logic (stub — implemented in Task 3) ─────────────────────────

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

// ── YouTube fetch (stub — implemented in Task 5) ──────────────────────────

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
