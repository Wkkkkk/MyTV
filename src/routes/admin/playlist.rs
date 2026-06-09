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
    media,
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
) -> impl IntoResponse {
    let url = form.url.trim().to_string();
    let mut duration_secs: i64 = form.duration_secs.trim().parse().unwrap_or(0);
    if duration_secs <= 0 {
        match media::fetch_duration(&state.http_client, &url).await {
            Ok(d) => duration_secs = d,
            Err(e) => {
                tracing::warn!(url = %url, error = %e, "failed to auto-fetch duration");
                return Html(format!(
                    r#"<p style="color:#e94560;padding:16px">Could not determine duration — enter it manually. <a href="/admin/channels/{channel_id}">← Go back</a></p>"#
                ))
                .into_response();
            }
        }
    }

    let existing = match playlist_item::list_for_channel(&state.pool, channel_id).await {
        Ok(items) => items,
        Err(_) => return StatusCode::INTERNAL_SERVER_ERROR.into_response(),
    };
    let sort_order = existing
        .iter()
        .map(|i| i.sort_order)
        .max()
        .map(|m| m + 1)
        .unwrap_or(0);

    if playlist_item::create(
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
    .is_err()
    {
        return StatusCode::INTERNAL_SERVER_ERROR.into_response();
    }
    Redirect::to(&format!("/admin/channels/{channel_id}")).into_response()
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

pub async fn playlist_item_toggle(
    State(state): State<AppState>,
    Path(item_id): Path<i64>,
) -> Result<impl IntoResponse, StatusCode> {
    let item = playlist_item::get(&state.pool, item_id)
        .await
        .map_err(internal_error)?
        .ok_or(StatusCode::NOT_FOUND)?;

    playlist_item::set_active(&state.pool, item_id, !item.is_active)
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

    crate::health::probe_playlist_item(&state.pool, &state.http_client, &state.cors_cache, &item)
        .await;

    if media::resolver::needs_resolution(&item.url) {
        let host = media::hls::extract_manifest_host(&item.url);
        state.cors_cache.write().await.insert(host, true);
    }

    let updated = playlist_item::get(&state.pool, item_id)
        .await
        .map_err(internal_error)?
        .ok_or(StatusCode::NOT_FOUND)?;

    let cors = state.cors_cache.read().await.clone();
    let mut row: AdminPlaylistItemRow = updated.into();
    row.apply_budget(&cors);

    render(PlaylistItemRowTemplate { item: row })
}
