use axum::{
    extract::{Path, State},
    http::StatusCode,
    response::Json,
};
use chrono::{DateTime, Utc};
use serde::Deserialize;

use super::{internal, ApiError};
use crate::routes::admin::channels::parse_loop_anchor;
use crate::{model::channel, AppState};

/// Channel create/update payload. PATCH is a full replacement (all fields
/// required), matching the form admin's update which rewrites every column.
#[derive(Deserialize)]
pub struct ChannelRequest {
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

fn normalize_logo(logo: Option<String>) -> Option<String> {
    logo.map(|s| s.trim().to_string()).filter(|s| !s.is_empty())
}

/// Resolve a channel's loop anchor, mirroring the form handler: for a vod_loop
/// channel use the parsed request anchor, else the existing one, else now;
/// a live channel has no anchor. `existing` is None on create.
fn resolve_anchor(
    channel_type: channel::ChannelType,
    raw: Option<&str>,
    existing: Option<DateTime<Utc>>,
) -> Option<DateTime<Utc>> {
    if channel_type == channel::ChannelType::VodLoop {
        raw.and_then(parse_loop_anchor)
            .or(existing)
            .or_else(|| Some(Utc::now()))
    } else {
        None
    }
}

fn validate_names(name: &str, category: &str) -> Result<(), ApiError> {
    if name.trim().is_empty() || category.trim().is_empty() {
        return Err(ApiError::Validation(
            "name and category are required".into(),
        ));
    }
    Ok(())
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
    Json(req): Json<ChannelRequest>,
) -> Result<(StatusCode, Json<channel::Channel>), ApiError> {
    validate_names(&req.name, &req.category)?;
    let channel_type = parse_type(&req.channel_type)?;
    let loop_anchor = resolve_anchor(channel_type, req.loop_anchor.as_deref(), None);
    let ch = channel::create(
        &state.pool,
        channel::NewChannel {
            name: req.name.trim().to_string(),
            category: req.category.trim().to_string(),
            logo_url: normalize_logo(req.logo_url),
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
    Json(req): Json<ChannelRequest>,
) -> Result<Json<channel::Channel>, ApiError> {
    validate_names(&req.name, &req.category)?;
    let channel_type = parse_type(&req.channel_type)?;
    let existing = channel::get(&state.pool, id)
        .await
        .map_err(internal)?
        .ok_or(ApiError::NotFound)?;
    let loop_anchor = resolve_anchor(
        channel_type,
        req.loop_anchor.as_deref(),
        existing.loop_anchor,
    );
    let ch = channel::update(
        &state.pool,
        id,
        channel::UpdateChannel {
            name: req.name.trim().to_string(),
            category: req.category.trim().to_string(),
            logo_url: normalize_logo(req.logo_url),
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
