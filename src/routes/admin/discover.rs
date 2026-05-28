use askama::Template;
use axum::{
    extract::{Form, State},
    http::StatusCode,
    response::{Html, IntoResponse, Redirect},
};
use chrono::Utc;
use serde::Deserialize;

use crate::{
    media::{hls, m3u, resolver},
    model::{channel, playlist_item, source},
    AppState,
};
use crate::routes::{internal_error, render};

// ── pure data types ────────────────────────────────────────────────────────

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
    group: String,
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
    group: String,
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
    pub group: Option<String>,
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
    pub new_name: Option<String>,
    pub new_category: Option<String>,
    pub new_channel_type: Option<String>,
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
        group: form.group.unwrap_or_default(),
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
) -> impl IntoResponse {
    let duration_secs: i64 = form.duration_secs.parse().unwrap_or(0);
    match do_discover_add(
        &state.pool,
        &state.http_client,
        &form.url,
        &form.title,
        &form.source_kind,
        duration_secs,
        &form.channel_choice,
        form.new_name.as_deref().unwrap_or(""),
        form.new_category.as_deref().unwrap_or(""),
        form.new_channel_type.as_deref().unwrap_or("live"),
    ).await {
        Ok(channel_id) => Redirect::to(&format!("/admin/channels/{}", channel_id)).into_response(),
        Err(status) => Html(format!(
            r#"<p style="color:#e94560;padding:16px">Error {}: could not add item — <a href="/admin/discover">go back</a></p>"#,
            status.as_u16()
        )).into_response(),
    }
}

pub async fn discover_m3u_search(
    State(state): State<AppState>,
    Form(form): Form<M3uSearchForm>,
) -> Html<String> {
    let country_code = if form.country.trim().is_empty() {
        None
    } else {
        country_to_code(&form.country)
    };
    let raw = match fetch_m3u(&state.http_client, country_code.as_deref()).await {
        Ok(r) => r,
        Err(e) => {
            tracing::error!("M3U fetch error: {e}");
            return Html("<p class=\"empty-state\" style=\"color:#f77\">Failed to fetch M3U list. Check server logs.</p>".to_string());
        }
    };
    let all = m3u::parse_m3u(&raw);
    let matches = m3u::filter_m3u(&all, "", &form.group);

    let handles: Vec<_> = matches.iter().map(|ch| {
        let client = state.http_client.clone();
        let url = ch.url.clone();
        tokio::spawn(async move { url_is_reachable(&client, &url).await })
    }).collect();
    let reachable: Vec<bool> = {
        let mut r = Vec::with_capacity(handles.len());
        for h in handles { r.push(h.await.unwrap_or(false)); }
        r
    };

    let rows: Vec<M3uResultRow> = matches.iter().zip(reachable)
        .filter(|(_, ok)| *ok)
        .enumerate()
        .map(|(i, (ch, _))| M3uResultRow {
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
        group: String::new(),
        is_live,
        duration_secs,
        source_kind: detect_source_kind(&form.url).to_string(),
        show_duration_input: !is_live && duration_secs == 0,
        channels,
    })
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

// ── core add logic ────────────────────────────────────────────────────────

pub async fn do_discover_add(
    pool: &sqlx::SqlitePool,
    client: &reqwest::Client,
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
        let mut duration_secs = duration_secs;
        if duration_secs <= 0 {
            if resolver::needs_resolution(url) {
                duration_secs = resolver::fetch_duration_secs(url).await.map_err(|e| {
                    tracing::warn!(url = %url, error = %e, "failed to auto-fetch duration in discover_add");
                    StatusCode::UNPROCESSABLE_ENTITY
                })?;
            } else {
                duration_secs = hls::fetch_hls_duration(client, url).await.map_err(|e| {
                    tracing::warn!(url = %url, error = %e, "failed to fetch HLS duration in discover_add");
                    StatusCode::UNPROCESSABLE_ENTITY
                })?;
            }
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

// ── YouTube fetch ─────────────────────────────────────────────────────────

async fn fetch_youtube_results(
    keyword: &str,
    api_key: &str,
    client: &reqwest::Client,
) -> anyhow::Result<Vec<YoutubeResultRow>> {
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

async fn url_is_reachable(client: &reqwest::Client, url: &str) -> bool {
    match client
        .head(url)
        .timeout(std::time::Duration::from_secs(5))
        .send()
        .await
    {
        Ok(r) => r.status().is_success() || r.status().is_redirection(),
        Err(_) => false,
    }
}

fn country_to_code(input: &str) -> Option<String> {
    let s = input.trim().to_lowercase();
    if s.len() == 2 && s.chars().all(|c| c.is_ascii_alphabetic()) {
        return Some(s);
    }
    let map: &[(&str, &str)] = &[
        ("afghanistan", "af"), ("albania", "al"), ("algeria", "dz"),
        ("argentina", "ar"), ("australia", "au"), ("austria", "at"),
        ("bangladesh", "bd"), ("belgium", "be"), ("brazil", "br"),
        ("bulgaria", "bg"), ("canada", "ca"), ("chile", "cl"),
        ("china", "cn"), ("colombia", "co"), ("croatia", "hr"),
        ("czech republic", "cz"), ("denmark", "dk"), ("egypt", "eg"),
        ("finland", "fi"), ("france", "fr"), ("germany", "de"),
        ("ghana", "gh"), ("greece", "gr"), ("hong kong", "hk"),
        ("hungary", "hu"), ("india", "in"), ("indonesia", "id"),
        ("iran", "ir"), ("iraq", "iq"), ("ireland", "ie"),
        ("israel", "il"), ("italy", "it"), ("japan", "jp"),
        ("jordan", "jo"), ("kenya", "ke"), ("kuwait", "kw"),
        ("lebanon", "lb"), ("malaysia", "my"), ("mexico", "mx"),
        ("morocco", "ma"), ("netherlands", "nl"), ("new zealand", "nz"),
        ("nigeria", "ng"), ("norway", "no"), ("pakistan", "pk"),
        ("philippines", "ph"), ("poland", "pl"), ("portugal", "pt"),
        ("qatar", "qa"), ("romania", "ro"), ("russia", "ru"),
        ("saudi arabia", "sa"), ("serbia", "rs"), ("singapore", "sg"),
        ("south africa", "za"), ("south korea", "kr"), ("korea", "kr"),
        ("spain", "es"), ("sweden", "se"), ("switzerland", "ch"),
        ("taiwan", "tw"), ("thailand", "th"), ("tunisia", "tn"),
        ("turkey", "tr"), ("ukraine", "ua"), ("united arab emirates", "ae"),
        ("uae", "ae"), ("united kingdom", "gb"), ("uk", "gb"),
        ("united states", "us"), ("usa", "us"), ("vietnam", "vn"),
    ];
    map.iter().find(|(name, _)| s.contains(name)).map(|(_, code)| code.to_string())
}

async fn fetch_m3u(client: &reqwest::Client, country_code: Option<&str>) -> anyhow::Result<String> {
    let url = match country_code {
        Some(code) => format!("https://iptv-org.github.io/iptv/countries/{}.m3u", code),
        None => "https://iptv-org.github.io/iptv/index.m3u".to_string(),
    };
    let text = client
        .get(&url)
        .send()
        .await?
        .error_for_status()?
        .text()
        .await?;
    Ok(text)
}

// ── tests ──────────────────────────────────────────────────────────────────

#[cfg(test)]
mod tests {
    use super::*;
    use crate::{db, model::{channel, playlist_item, source}};
    use axum::http::StatusCode;
    use chrono::Utc;

    async fn test_pool() -> sqlx::SqlitePool {
        db::connect("sqlite::memory:").await.unwrap()
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

    #[tokio::test]
    async fn test_add_new_live_channel_creates_source() {
        let pool = test_pool().await;
        let client = reqwest::Client::new();
        let ch_id = do_discover_add(
            &pool, &client, "https://example.com/s.m3u8", "CNN", "hls", 0,
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
        let client = reqwest::Client::new();
        let ch_id = do_discover_add(
            &pool, &client, "https://example.com/ep1.mp4", "Ep 1", "hls", 3600,
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
        let client = reqwest::Client::new();
        let ch_id = do_discover_add(
            &pool, &client, "https://example.com/s.m3u8", "Existing", "iptv", 0,
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
        let client = reqwest::Client::new();
        let ch_id = do_discover_add(
            &pool, &client, "https://example.com/movie.mp4", "Movie", "hls", 5400,
            &existing.id.to_string(), "", "", "",
        ).await.unwrap();
        let items = playlist_item::list_for_channel(&pool, ch_id).await.unwrap();
        assert_eq!(items.len(), 1);
        assert_eq!(items[0].duration_secs, 5400);
    }

    #[tokio::test]
    async fn test_add_returns_422_when_new_name_empty() {
        let pool = test_pool().await;
        let client = reqwest::Client::new();
        let result = do_discover_add(
            &pool, &client, "https://example.com/s.m3u8", "Test", "hls", 0,
            "new", "", "news", "live",
        ).await;
        assert_eq!(result, Err(StatusCode::UNPROCESSABLE_ENTITY));
    }

    #[tokio::test]
    async fn test_add_returns_422_when_vod_duration_zero() {
        let pool = test_pool().await;
        let client = reqwest::Client::new();
        let result = do_discover_add(
            &pool, &client, "https://example.com/v.mp4", "Test", "hls", 0,
            "new", "Show", "movies", "vod_loop",
        ).await;
        assert_eq!(result, Err(StatusCode::UNPROCESSABLE_ENTITY));
    }
}
