use axum::{
    extract::{Path, State},
    http::StatusCode,
    response::Json,
};
use serde::Deserialize;
use std::str::FromStr;

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
    let url = req.url.trim().to_string();
    if url.is_empty() {
        return Err(ApiError::Validation("url is required".into()));
    }
    let kind = match req.kind {
        Some(k) => {
            source::SourceKind::from_str(&k).map_err(|e| ApiError::Validation(e.to_string()))?
        }
        None => source::SourceKind::detect(&url),
    };
    let src = source::create(
        &state.pool,
        source::NewSource {
            channel_id,
            kind,
            url,
            priority: req.priority.unwrap_or(1),
        },
    )
    .await
    .map_err(internal)?;
    Ok((StatusCode::CREATED, Json(src)))
}

pub async fn update(
    State(state): State<AppState>,
    Path(id): Path<i64>,
    Json(req): Json<UpdateSourceRequest>,
) -> Result<Json<source::Source>, ApiError> {
    let url = req.url.trim().to_string();
    if url.is_empty() {
        return Err(ApiError::Validation("url is required".into()));
    }
    let src = source::update(
        &state.pool,
        id,
        source::UpdateSource {
            url,
            priority: req.priority,
        },
    )
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
    crate::health::probe_source(
        &state.pool,
        &state.http_client,
        &state.cors_cache,
        &state.live_cache,
        &src,
    )
    .await;
    let updated = source::get(&state.pool, id)
        .await
        .map_err(internal)?
        .ok_or(ApiError::NotFound)?;
    Ok(Json(updated))
}
