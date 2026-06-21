use axum::{
    extract::{Path, State},
    http::StatusCode,
    response::Json,
};
use serde::Deserialize;

use super::{internal, ApiError, ToggleRequest};
use crate::{model::playlist_item, AppState};

#[derive(Deserialize)]
pub struct CreatePlaylistItemRequest {
    pub title: String,
    pub url: String,
    pub duration_secs: i64,
    pub sort_order: Option<i64>,
}

#[derive(Deserialize)]
pub struct UpdatePlaylistItemRequest {
    pub title: String,
    pub url: String,
    pub duration_secs: i64,
    pub sort_order: i64,
}

pub async fn list_for_channel(
    State(state): State<AppState>,
    Path(channel_id): Path<i64>,
) -> Result<Json<Vec<playlist_item::PlaylistItem>>, ApiError> {
    let items = playlist_item::list_for_channel(&state.pool, channel_id)
        .await
        .map_err(internal)?;
    Ok(Json(items))
}

pub async fn get_one(
    State(state): State<AppState>,
    Path(id): Path<i64>,
) -> Result<Json<playlist_item::PlaylistItem>, ApiError> {
    let item = playlist_item::get(&state.pool, id)
        .await
        .map_err(internal)?
        .ok_or(ApiError::NotFound)?;
    Ok(Json(item))
}

pub async fn create(
    State(state): State<AppState>,
    Path(channel_id): Path<i64>,
    Json(req): Json<CreatePlaylistItemRequest>,
) -> Result<(StatusCode, Json<playlist_item::PlaylistItem>), ApiError> {
    // Validate first so dedup compares the trimmed url/title; sort_order is a
    // placeholder until we know it isn't a duplicate.
    let mut new = playlist_item::PlaylistInput {
        title: req.title,
        url: req.url,
        duration_secs: req.duration_secs,
        sort_order: crate::model::DEFAULT_SORT_ORDER,
    }
    .validate_new(channel_id)?;

    // Dedup: a re-upload of the same recording (same url or same title on this
    // channel) returns the existing item instead of appending a copy, so callers
    // like the publish-video plugin are safe to re-run.
    if let Some(existing) =
        playlist_item::find_duplicate(&state.pool, channel_id, &new.url, &new.title)
            .await
            .map_err(internal)?
    {
        return Ok((StatusCode::OK, Json(existing)));
    }

    // Append at the end unless the caller pinned an explicit position, so items
    // keep a sequential sort_order instead of collapsing onto 0.
    new.sort_order = match req.sort_order {
        Some(s) => s,
        None => playlist_item::next_sort_order(&state.pool, channel_id)
            .await
            .map_err(internal)?,
    };
    let item = playlist_item::create(&state.pool, new)
        .await
        .map_err(internal)?;
    Ok((StatusCode::CREATED, Json(item)))
}

pub async fn update(
    State(state): State<AppState>,
    Path(id): Path<i64>,
    Json(req): Json<UpdatePlaylistItemRequest>,
) -> Result<Json<playlist_item::PlaylistItem>, ApiError> {
    let upd = playlist_item::PlaylistInput {
        title: req.title,
        url: req.url,
        duration_secs: req.duration_secs,
        sort_order: req.sort_order,
    }
    .validate_update()?;
    let item = playlist_item::update(&state.pool, id, upd)
        .await
        .map_err(internal)?
        .ok_or(ApiError::NotFound)?;
    Ok(Json(item))
}

pub async fn remove(
    State(state): State<AppState>,
    Path(id): Path<i64>,
) -> Result<StatusCode, ApiError> {
    let deleted = playlist_item::delete(&state.pool, id)
        .await
        .map_err(internal)?;
    if deleted {
        Ok(StatusCode::NO_CONTENT)
    } else {
        Err(ApiError::NotFound)
    }
}

pub async fn toggle(
    State(state): State<AppState>,
    Path(id): Path<i64>,
    Json(req): Json<ToggleRequest>,
) -> Result<Json<playlist_item::PlaylistItem>, ApiError> {
    let changed = playlist_item::set_active(&state.pool, id, req.active)
        .await
        .map_err(internal)?;
    if !changed {
        return Err(ApiError::NotFound);
    }
    let item = playlist_item::get(&state.pool, id)
        .await
        .map_err(internal)?
        .ok_or(ApiError::NotFound)?;
    Ok(Json(item))
}

pub async fn test(
    State(state): State<AppState>,
    Path(id): Path<i64>,
) -> Result<Json<playlist_item::PlaylistItem>, ApiError> {
    let item = playlist_item::get(&state.pool, id)
        .await
        .map_err(internal)?
        .ok_or(ApiError::NotFound)?;
    crate::health::probe(
        &state.pool,
        &state.http_client,
        &state.cors_cache,
        &state.live_cache,
        crate::health::ProbeTarget::PlaylistItem(&item),
    )
    .await;
    let updated = playlist_item::get(&state.pool, id)
        .await
        .map_err(internal)?
        .ok_or(ApiError::NotFound)?;
    Ok(Json(updated))
}
