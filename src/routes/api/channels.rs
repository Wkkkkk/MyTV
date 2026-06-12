use axum::{
    extract::{Path, State},
    http::StatusCode,
    response::Json,
};
use serde::Deserialize;

use super::{internal, ApiError};
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
    let new = channel::ChannelInput {
        name: req.name,
        category: req.category,
        channel_type: req.channel_type,
        sort_order: req.sort_order,
        logo_url: req.logo_url,
        loop_anchor: req.loop_anchor,
    }
    .validate_new()?;
    let ch = channel::create(&state.pool, new).await.map_err(internal)?;
    Ok((StatusCode::CREATED, Json(ch)))
}

pub async fn update(
    State(state): State<AppState>,
    Path(id): Path<i64>,
    Json(req): Json<ChannelRequest>,
) -> Result<Json<channel::Channel>, ApiError> {
    let existing = channel::get(&state.pool, id)
        .await
        .map_err(internal)?
        .ok_or(ApiError::NotFound)?;
    let upd = channel::ChannelInput {
        name: req.name,
        category: req.category,
        channel_type: req.channel_type,
        sort_order: req.sort_order,
        logo_url: req.logo_url,
        loop_anchor: req.loop_anchor,
    }
    .validate_update(existing.loop_anchor)?;
    let ch = channel::update(&state.pool, id, upd)
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
