use askama::Template;
use axum::{
    extract::{Path, State},
    http::StatusCode,
    response::{Html, IntoResponse, Redirect},
};
use serde::Deserialize;

use crate::routes::admin::AdminPlaylistItemRow;
use crate::routes::{internal_error, render};
use crate::{
    media::{hls, resolver},
    model::{playlist_item, playlist_item::NewPlaylistItem},
    AppState,
};

#[derive(Template)]
#[template(path = "admin/partials/playlist_item_row.html")]
struct PlaylistItemRowTemplate {
    item: AdminPlaylistItemRow,
}

// ── form input types ───────────────────────────────────────────────────────

#[derive(Deserialize)]
pub struct PlaylistItemForm {
    pub title: String,
    pub url: String,
    pub duration_secs: String,
}

// ── handlers ───────────────────────────────────────────────────────────────

pub async fn playlist_item_create(
    State(state): State<AppState>,
    Path(channel_id): Path<i64>,
    axum::extract::Form(form): axum::extract::Form<PlaylistItemForm>,
) -> Result<impl IntoResponse, StatusCode> {
    let url = form.url.trim().to_string();
    let mut duration_secs: i64 = form.duration_secs.trim().parse().unwrap_or(0);
    if duration_secs <= 0 {
        if resolver::needs_resolution(&url) {
            duration_secs = resolver::fetch_duration_secs(&url).await.map_err(|e| {
                tracing::warn!(url = %url, error = %e, "failed to auto-fetch duration");
                StatusCode::UNPROCESSABLE_ENTITY
            })?;
        } else {
            duration_secs = hls::fetch_hls_duration(&state.http_client, &url)
                .await
                .map_err(|e| {
                    tracing::warn!(url = %url, error = %e, "failed to fetch HLS duration");
                    StatusCode::UNPROCESSABLE_ENTITY
                })?;
        }
    }

    let existing = playlist_item::list_for_channel(&state.pool, channel_id)
        .await
        .map_err(internal_error)?;
    let sort_order = existing.len() as i64;

    playlist_item::create(
        &state.pool,
        NewPlaylistItem {
            channel_id,
            title: form.title.trim().to_string(),
            url,
            duration_secs,
            sort_order,
        },
    )
    .await
    .map_err(internal_error)?;
    Ok(Redirect::to(&format!("/admin/channels/{channel_id}")))
}

pub async fn playlist_item_delete(
    State(state): State<AppState>,
    Path(item_id): Path<i64>,
) -> Result<impl IntoResponse, StatusCode> {
    let item = playlist_item::get(&state.pool, item_id)
        .await
        .map_err(internal_error)?
        .ok_or(StatusCode::NOT_FOUND)?;

    playlist_item::delete(&state.pool, item_id)
        .await
        .map_err(internal_error)?;
    Ok(Redirect::to(&format!(
        "/admin/channels/{}",
        item.channel_id
    )))
}

pub async fn playlist_item_test(
    State(state): State<AppState>,
    Path(item_id): Path<i64>,
) -> Result<Html<String>, StatusCode> {
    let item = playlist_item::get(&state.pool, item_id)
        .await
        .map_err(internal_error)?
        .ok_or(StatusCode::NOT_FOUND)?;

    crate::health::probe_and_cache_cors(&state.http_client, &state.cors_cache, &item.url).await;

    let cors = state.cors_cache.read().await.clone();
    let mut row: AdminPlaylistItemRow = item.into();
    row.apply_budget(&cors);

    render(PlaylistItemRowTemplate { item: row })
}
