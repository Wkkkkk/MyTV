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

pub fn detect_source_kind(url: &str) -> &'static str {
    if url.contains("youtube.com") || url.contains("youtu.be") {
        "youtube_live"
    } else if url.contains(".m3u8") {
        "hls"
    } else {
        "iptv"
    }
}

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
    use crate::{channel, db, playlist_item, source};
    use axum::http::StatusCode;
    use chrono::Utc;

    async fn test_pool() -> sqlx::SqlitePool {
        db::connect("sqlite::memory:").await.unwrap()
    }

    // ── pure function tests ────────────────────────────────────────────────

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
        // First EXTINF immediately followed by another EXTINF (no URL line)
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
