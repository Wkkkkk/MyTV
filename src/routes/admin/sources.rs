use askama::Template;
use axum::{
    extract::{Path, State},
    http::StatusCode,
    response::{Html, IntoResponse, Redirect},
};
use serde::Deserialize;

use crate::routes::admin::AdminSourceRow;
use crate::routes::{internal_error, render};
use crate::{model::source, AppState};

// ── form input types ───────────────────────────────────────────────────────

#[derive(Deserialize)]
pub struct SourceForm {
    pub kind: String,
    pub url: String,
    pub priority: String,
}

#[derive(Template)]
#[template(path = "admin/partials/source_row.html")]
struct SourceRowTemplate {
    src: AdminSourceRow,
}

// ── handlers ───────────────────────────────────────────────────────────────

pub async fn source_create(
    State(state): State<AppState>,
    Path(channel_id): Path<i64>,
    axum::extract::Form(form): axum::extract::Form<SourceForm>,
) -> Result<impl IntoResponse, StatusCode> {
    let kind = form
        .kind
        .parse::<source::SourceKind>()
        .map_err(|_| StatusCode::UNPROCESSABLE_ENTITY)?;
    if form.url.trim().is_empty() {
        return Err(StatusCode::UNPROCESSABLE_ENTITY);
    }
    let priority: i64 = form.priority.trim().parse().unwrap_or(1);
    source::create(
        &state.pool,
        source::NewSource {
            channel_id,
            kind,
            url: form.url.trim().to_string(),
            priority,
        },
    )
    .await
    .map_err(internal_error)?;
    Ok(Redirect::to(&format!("/admin/channels/{channel_id}")))
}

pub async fn source_delete(
    State(state): State<AppState>,
    Path(source_id): Path<i64>,
) -> Result<impl IntoResponse, StatusCode> {
    let src = source::get(&state.pool, source_id)
        .await
        .map_err(internal_error)?
        .ok_or(StatusCode::NOT_FOUND)?;

    source::delete(&state.pool, source_id)
        .await
        .map_err(internal_error)?;
    Ok(Redirect::to(&format!("/admin/channels/{}", src.channel_id)))
}

pub async fn source_toggle(
    State(state): State<AppState>,
    Path(source_id): Path<i64>,
) -> Result<impl IntoResponse, StatusCode> {
    let src = source::get(&state.pool, source_id)
        .await
        .map_err(internal_error)?
        .ok_or(StatusCode::NOT_FOUND)?;

    source::set_active(&state.pool, source_id, !src.is_active)
        .await
        .map_err(internal_error)?;
    Ok(Redirect::to(&format!("/admin/channels/{}", src.channel_id)))
}

pub async fn source_test(
    State(state): State<AppState>,
    Path(source_id): Path<i64>,
) -> Result<Html<String>, StatusCode> {
    let src = source::get(&state.pool, source_id)
        .await
        .map_err(internal_error)?
        .ok_or(StatusCode::NOT_FOUND)?;

    crate::health::probe_source(&state.pool, &state.http_client, &state.cors_cache, &src).await;

    let updated = source::get(&state.pool, source_id)
        .await
        .map_err(internal_error)?
        .ok_or(StatusCode::NOT_FOUND)?;

    let cors = state.cors_cache.read().await.clone();
    let mut row: AdminSourceRow = updated.into();
    row.apply_budget(&cors);

    render(SourceRowTemplate { src: row })
}
