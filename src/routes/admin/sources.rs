use axum::{
    extract::{Path, State},
    http::StatusCode,
    response::{Html, IntoResponse, Redirect},
};
use serde::Deserialize;

use crate::media::resolver;
use crate::routes::internal_error;
use crate::{model::source, AppState};

// ── form input types ───────────────────────────────────────────────────────

#[derive(Deserialize)]
pub struct SourceForm {
    pub kind: String,
    pub url: String,
    pub priority: String,
}

// ── handlers ───────────────────────────────────────────────────────────────

pub async fn source_create(
    State(state): State<AppState>,
    Path(channel_id): Path<i64>,
    axum::extract::Form(form): axum::extract::Form<SourceForm>,
) -> Result<impl IntoResponse, StatusCode> {
    if !["hls", "youtube_live", "iptv"].contains(&form.kind.as_str()) {
        return Err(StatusCode::UNPROCESSABLE_ENTITY);
    }
    if form.url.trim().is_empty() {
        return Err(StatusCode::UNPROCESSABLE_ENTITY);
    }
    let priority: i64 = form.priority.trim().parse().unwrap_or(1);
    source::create(
        &state.pool,
        source::NewSource {
            channel_id,
            kind: form.kind.clone(),
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

    let ok_html = r#"<span class="badge badge-on">OK</span>"#;
    let fail_html = r#"<span style="color:#e94560;font-size:0.78rem">Failed</span>"#;

    if resolver::needs_resolution(&src.url) {
        return Ok(Html(match resolver::resolve_url(&src.url).await {
            Ok(_) => ok_html.to_string(),
            Err(_) => fail_html.to_string(),
        }));
    }

    Ok(Html(
        match state
            .http_client
            .head(&src.url)
            .timeout(std::time::Duration::from_secs(10))
            .send()
            .await
        {
            Ok(resp) if resp.status().is_success() || resp.status().is_redirection() => {
                ok_html.to_string()
            }
            Ok(resp) => format!(
                r#"<span style="color:#e94560;font-size:0.78rem">Failed: HTTP {}</span>"#,
                resp.status().as_u16()
            ),
            Err(_) => fail_html.to_string(),
        },
    ))
}
