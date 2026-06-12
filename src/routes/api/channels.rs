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

fn resolve_anchor(
    channel_type: channel::ChannelType,
    raw: Option<&str>,
) -> Option<chrono::DateTime<Utc>> {
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
        return Err(ApiError::Validation(
            "name and category are required".into(),
        ));
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
        return Err(ApiError::Validation(
            "name and category are required".into(),
        ));
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
