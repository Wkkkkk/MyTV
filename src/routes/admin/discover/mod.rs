mod add;
mod m3u;
mod youtube;

pub use add::{do_discover_add, DiscoverAddParams};

use askama::Template;
use axum::{
    extract::{Form, State},
    http::StatusCode,
    response::{Html, IntoResponse, Redirect},
};
use serde::Deserialize;

use crate::routes::{internal_error, render};
use crate::{
    media::m3u as media_m3u,
    media::resolver,
    model::{channel, source},
    AppState,
};

use m3u::M3uResultRow;
use youtube::YoutubeResultRow;

// ── pure data types ────────────────────────────────────────────────────────

pub struct DiscoverChannelOption {
    pub id: i64,
    pub name: String,
    pub type_str: String,
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
    pub duration_secs: String,
    pub source_kind: String,
    pub channel_choice: String,
    pub new_name: Option<String>,
    pub new_category: Option<String>,
    pub new_channel_type: Option<String>,
}

// ── helpers ───────────────────────────────────────────────────────────────

fn html_escape(s: &str) -> String {
    s.replace('&', "&amp;")
        .replace('<', "&lt;")
        .replace('>', "&gt;")
        .replace('"', "&quot;")
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
    match do_discover_add(DiscoverAddParams {
        pool: &state.pool,
        client: &state.http_client,
        url: &form.url,
        title: &form.title,
        source_kind: &form.source_kind,
        duration_secs,
        channel_choice: &form.channel_choice,
        new_name: form.new_name.as_deref().unwrap_or(""),
        new_category: form.new_category.as_deref().unwrap_or(""),
        new_channel_type: form.new_channel_type.as_deref().unwrap_or("live"),
    })
    .await
    {
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
        m3u::country_to_code(&form.country)
    };
    let raw = match m3u::fetch_m3u(&state.http_client, country_code.as_deref()).await {
        Ok(r) => r,
        Err(e) => {
            tracing::error!("M3U fetch error: {e}");
            return Html("<p class=\"empty-state\" style=\"color:#f77\">Failed to fetch M3U list. Check server logs.</p>".to_string());
        }
    };
    let all = media_m3u::parse_m3u(&raw);
    let matches = media_m3u::filter_m3u(&all, "", &form.group);

    let handles: Vec<_> = matches
        .iter()
        .map(|ch| {
            let client = state.http_client.clone();
            let url = ch.url.clone();
            tokio::spawn(async move { m3u::url_is_reachable(&client, &url).await })
        })
        .collect();
    let reachable: Vec<bool> = {
        let mut r = Vec::with_capacity(handles.len());
        for h in handles {
            r.push(h.await.unwrap_or(false));
        }
        r
    };

    let rows: Vec<M3uResultRow> = matches
        .iter()
        .zip(reachable)
        .filter(|(_, ok)| *ok)
        .enumerate()
        .map(|(i, (ch, _))| M3uResultRow {
            name: ch.name.clone(),
            group: ch.group.clone(),
            country: ch.country.clone(),
            url: ch.url.clone(),
            source_kind: source::SourceKind::detect(&ch.url).as_str().to_string(),
            form_id: i,
        })
        .collect();
    match (M3uResultsTemplate { rows }).render() {
        Ok(html) => Html(html),
        Err(e) => {
            tracing::error!("template error: {e}");
            Html("<p class=\"empty-state\" style=\"color:#f77\">Render error.</p>".to_string())
        }
    }
}

pub async fn discover_youtube_search(
    State(state): State<AppState>,
    Form(form): Form<YoutubeSearchForm>,
) -> Html<String> {
    let api_key =
        match &state.config.youtube_api_key {
            Some(k) => k.clone(),
            None => return Html(
                "<p class=\"empty-state\" style=\"color:#f77\">YOUTUBE_API_KEY not configured.</p>"
                    .to_string(),
            ),
        };
    let rows =
        match youtube::fetch_youtube_results(&form.keyword, &api_key, &state.http_client).await {
            Ok(r) => r,
            Err(e) => {
                tracing::error!("YouTube API error: {e}");
                return Html(format!(
                    "<p class=\"empty-state\" style=\"color:#f77\">YouTube search failed: {}.</p>",
                    html_escape(&e.to_string())
                ));
            }
        };
    match (YtResultsTemplate { rows }).render() {
        Ok(html) => Html(html),
        Err(e) => {
            tracing::error!("template error: {e}");
            Html("<p class=\"empty-state\" style=\"color:#f77\">Render error.</p>".to_string())
        }
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
    let (duration_secs, title) = if is_youtube {
        let (dur_result, title_result) = tokio::join!(
            tokio::time::timeout(
                std::time::Duration::from_secs(5),
                resolver::fetch_duration_secs(&form.url),
            ),
            tokio::time::timeout(
                std::time::Duration::from_secs(5),
                resolver::fetch_title(&form.url),
            ),
        );
        let duration = dur_result.ok().and_then(|r| r.ok()).unwrap_or(0);
        let title = title_result
            .ok()
            .and_then(|r| r.ok())
            .unwrap_or_else(|| form.url.clone());
        (duration, title)
    } else {
        (0, form.url.clone())
    };
    let is_live = duration_secs == 0;
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
        url: form.url.clone(),
        title,
        group: String::new(),
        is_live,
        duration_secs,
        source_kind: source::SourceKind::detect(&form.url).as_str().to_string(),
        show_duration_input: !is_live && duration_secs == 0,
        channels,
    })
}
