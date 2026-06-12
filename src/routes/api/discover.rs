use axum::extract::{Query, State};
use axum::http::StatusCode;
use axum::response::Json;
use serde::{Deserialize, Serialize};

use super::{internal, ApiError};
use crate::model::channel;
use crate::routes::admin::discover::{
    do_discover_add, fetch_youtube_channels, fetch_youtube_results, m3u_search, resolve_channel,
    resolve_manual, DiscoverAddParams, M3uResultRow, ResolvedMeta, YoutubeResultRow,
};
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

pub async fn resolve(Json(req): Json<ResolveRequest>) -> Result<Json<ResolvedCandidate>, ApiError> {
    let meta = resolve_manual(&req.url)
        .await
        .map_err(|_| ApiError::Validation("invalid or unresolvable URL".into()))?;
    Ok(Json(meta.into()))
}

pub async fn channel(Json(req): Json<ResolveRequest>) -> Result<Json<ResolvedCandidate>, ApiError> {
    let meta = resolve_channel(&req.url)
        .map_err(|_| ApiError::Validation("not a recognized YouTube channel URL".into()))?;
    Ok(Json(meta.into()))
}

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
        M3uCandidate {
            name: r.name,
            group: r.group,
            country: r.country,
            url: r.url,
            source_kind: r.source_kind,
        }
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
    let (channel_choice, new_name, new_category, new_channel_type) = match &req.channel {
        ChannelTarget::ExistingId(id) => (
            id.to_string(),
            String::new(),
            String::new(),
            "live".to_string(),
        ),
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
    Ok((
        StatusCode::CREATED,
        Json(AddResponse {
            channel_id,
            channel,
        }),
    ))
}

/// `do_discover_add` returns a bare StatusCode; map it to ApiError so the JSON
/// error body stays consistent.
fn map_add_status(status: StatusCode) -> ApiError {
    match status {
        StatusCode::NOT_FOUND => ApiError::NotFound,
        StatusCode::UNPROCESSABLE_ENTITY => ApiError::Validation(
            "invalid discover-add request: check url (must be http/https), source_kind, \
             and (for a new channel) a non-empty name and a valid type"
                .into(),
        ),
        _ => ApiError::Internal,
    }
}
