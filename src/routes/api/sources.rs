use axum::{
    extract::{Path, State},
    http::StatusCode,
    response::Json,
};
use serde::Deserialize;

use super::{internal, ApiError, ToggleRequest};
use crate::{model::source, AppState};

#[derive(Deserialize)]
pub struct CreateSourceRequest {
    pub url: String,
    pub priority: Option<i64>,
    pub kind: Option<String>,
}

#[derive(Deserialize)]
pub struct UpdateSourceRequest {
    pub url: String,
    pub priority: i64,
}

pub async fn list_for_channel(
    State(state): State<AppState>,
    Path(channel_id): Path<i64>,
) -> Result<Json<Vec<source::Source>>, ApiError> {
    let sources = source::list_for_channel(&state.pool, channel_id)
        .await
        .map_err(internal)?;
    Ok(Json(sources))
}

pub async fn get_one(
    State(state): State<AppState>,
    Path(id): Path<i64>,
) -> Result<Json<source::Source>, ApiError> {
    let src = source::get(&state.pool, id)
        .await
        .map_err(internal)?
        .ok_or(ApiError::NotFound)?;
    Ok(Json(src))
}

pub async fn create(
    State(state): State<AppState>,
    Path(channel_id): Path<i64>,
    Json(req): Json<CreateSourceRequest>,
) -> Result<(StatusCode, Json<source::Source>), ApiError> {
    let new = source::SourceInput {
        kind: req.kind,
        url: req.url,
        priority: req.priority.unwrap_or(1),
    }
    .validate_new(channel_id)?;
    let src = source::create(&state.pool, new).await.map_err(internal)?;
    Ok((StatusCode::CREATED, Json(src)))
}

pub async fn update(
    State(state): State<AppState>,
    Path(id): Path<i64>,
    Json(req): Json<UpdateSourceRequest>,
) -> Result<Json<source::Source>, ApiError> {
    let upd = source::SourceInput {
        kind: None,
        url: req.url,
        priority: req.priority,
    }
    .validate_update()?;
    let src = source::update(&state.pool, id, upd)
        .await
        .map_err(internal)?
        .ok_or(ApiError::NotFound)?;
    Ok(Json(src))
}

pub async fn remove(
    State(state): State<AppState>,
    Path(id): Path<i64>,
) -> Result<StatusCode, ApiError> {
    let deleted = source::delete(&state.pool, id).await.map_err(internal)?;
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
) -> Result<Json<source::Source>, ApiError> {
    let changed = source::set_active(&state.pool, id, req.active)
        .await
        .map_err(internal)?;
    if !changed {
        return Err(ApiError::NotFound);
    }
    let src = source::get(&state.pool, id)
        .await
        .map_err(internal)?
        .ok_or(ApiError::NotFound)?;
    Ok(Json(src))
}

pub async fn test(
    State(state): State<AppState>,
    Path(id): Path<i64>,
) -> Result<Json<source::Source>, ApiError> {
    let src = source::get(&state.pool, id)
        .await
        .map_err(internal)?
        .ok_or(ApiError::NotFound)?;
    crate::health::probe(
        &state.pool,
        &state.http_client,
        &state.cors_cache,
        &state.live_cache,
        crate::health::ProbeTarget::Source(&src),
    )
    .await;
    let updated = source::get(&state.pool, id)
        .await
        .map_err(internal)?
        .ok_or(ApiError::NotFound)?;
    Ok(Json(updated))
}
