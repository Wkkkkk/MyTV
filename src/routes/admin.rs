use askama::Template;
use axum::{
    extract::{Form, Path, Request, State},
    http::{header, StatusCode},
    middleware::Next,
    response::{Html, IntoResponse, Redirect, Response},
};
use base64::{engine::general_purpose, Engine as _};
use chrono::{DateTime, NaiveDateTime, Utc};
use serde::Deserialize;

use crate::{channel, playlist_item, source, AppState};

// ── auth ───────────────────────────────────────────────────────────────────

pub fn check_basic_auth(header_value: &str, expected_password: &str) -> bool {
    header_value
        .strip_prefix("Basic ")
        .and_then(|b64| general_purpose::STANDARD.decode(b64).ok())
        .and_then(|bytes| String::from_utf8(bytes).ok())
        .map(|credentials| {
            let actual = credentials.splitn(2, ':').nth(1).unwrap_or("");
            actual.len() == expected_password.len()
                && actual
                    .bytes()
                    .zip(expected_password.bytes())
                    .fold(0u8, |acc, (a, b)| acc | (a ^ b))
                    == 0
        })
        .unwrap_or(false)
}

pub async fn basic_auth(
    State(state): State<AppState>,
    request: Request,
    next: Next,
) -> Response {
    let authorized = request
        .headers()
        .get(header::AUTHORIZATION)
        .and_then(|v| v.to_str().ok())
        .map(|v| check_basic_auth(v, &state.config.admin_password))
        .unwrap_or(false);

    if authorized {
        next.run(request).await
    } else {
        (
            StatusCode::UNAUTHORIZED,
            [(header::WWW_AUTHENTICATE, "Basic realm=\"MyTV Admin\"")],
            "Unauthorized",
        )
            .into_response()
    }
}

// ── display types ──────────────────────────────────────────────────────────

pub struct AdminChannelRow {
    pub id: i64,
    pub name: String,
    pub category: String,
    pub type_str: String,
    pub sort_order: i64,
}

pub struct AdminSourceRow {
    pub id: i64,
    pub kind: String,
    pub url: String,
    pub priority: i64,
    pub is_active: bool,
}

pub struct AdminPlaylistItemRow {
    pub id: i64,
    pub title: String,
    pub url: String,
    pub duration_secs: i64,
    pub sort_order: i64,
}

// ── template types ─────────────────────────────────────────────────────────

#[derive(Template)]
#[template(path = "admin/channels.html")]
struct ChannelListTemplate {
    channels: Vec<AdminChannelRow>,
}

#[derive(Template)]
#[template(path = "admin/channel_form.html")]
struct ChannelFormTemplate {
    is_edit: bool,
    channel_id: i64,
    name: String,
    category: String,
    channel_type: String,
    sort_order: i64,
    logo_url: String,
    loop_anchor: String,
}

#[derive(Template)]
#[template(path = "admin/channel_detail.html")]
struct ChannelDetailTemplate {
    channel_id: i64,
    channel_name: String,
    channel_type: String,
    sources: Vec<AdminSourceRow>,
    playlist_items: Vec<AdminPlaylistItemRow>,
}

// ── form input types ───────────────────────────────────────────────────────

#[derive(Deserialize)]
pub struct ChannelForm {
    pub name: String,
    pub category: String,
    pub channel_type: String,
    pub sort_order: String,
    pub logo_url: String,
    pub loop_anchor: String,
}

#[derive(Deserialize)]
pub struct SourceForm {
    pub kind: String,
    pub url: String,
    pub priority: String,
}

#[derive(Deserialize)]
pub struct PlaylistItemForm {
    pub title: String,
    pub url: String,
    pub duration_secs: String,
}

// ── helpers ────────────────────────────────────────────────────────────────

fn parse_loop_anchor(s: &str) -> Option<DateTime<Utc>> {
    let trimmed = s.trim();
    if trimmed.is_empty() {
        return None;
    }
    NaiveDateTime::parse_from_str(trimmed, "%Y-%m-%dT%H:%M")
        .ok()
        .map(|dt| DateTime::from_naive_utc_and_offset(dt, Utc))
}

fn render<T: askama::Template>(t: T) -> Result<Html<String>, StatusCode> {
    t.render()
        .map(Html)
        .map_err(|e| {
            tracing::error!("template render error: {e}");
            StatusCode::INTERNAL_SERVER_ERROR
        })
}

fn internal_error<E: std::fmt::Display>(e: E) -> StatusCode {
    tracing::error!("admin error: {e}");
    StatusCode::INTERNAL_SERVER_ERROR
}

// ── handlers ───────────────────────────────────────────────────────────────

pub async fn admin_index() -> impl IntoResponse {
    Redirect::to("/admin/channels")
}

pub async fn channel_list(
    State(state): State<AppState>,
) -> Result<Html<String>, StatusCode> {
    let all = channel::list(&state.pool).await.map_err(internal_error)?;
    let channels = all
        .into_iter()
        .map(|ch| AdminChannelRow {
            id: ch.id,
            name: ch.name,
            category: ch.category,
            type_str: ch.r#type,
            sort_order: ch.sort_order,
        })
        .collect();
    render(ChannelListTemplate { channels })
}

pub async fn channel_new_form() -> Result<Html<String>, StatusCode> {
    render(ChannelFormTemplate {
        is_edit: false,
        channel_id: 0,
        name: String::new(),
        category: String::new(),
        channel_type: "live".to_string(),
        sort_order: 0,
        logo_url: String::new(),
        loop_anchor: String::new(),
    })
}

pub async fn channel_create(
    State(state): State<AppState>,
    Form(form): Form<ChannelForm>,
) -> Result<impl IntoResponse, StatusCode> {
    if !["live", "vod_loop"].contains(&form.channel_type.as_str()) {
        return Err(StatusCode::UNPROCESSABLE_ENTITY);
    }
    if form.name.trim().is_empty() || form.category.trim().is_empty() {
        return Err(StatusCode::UNPROCESSABLE_ENTITY);
    }
    let sort_order: i64 = form.sort_order.trim().parse().unwrap_or(0);
    let logo_url = if form.logo_url.trim().is_empty() {
        None
    } else {
        Some(form.logo_url.trim().to_string())
    };
    let loop_anchor = if form.channel_type.as_str() == "vod_loop" {
        parse_loop_anchor(&form.loop_anchor).or_else(|| Some(Utc::now()))
    } else {
        None
    };

    channel::create(
        &state.pool,
        channel::NewChannel {
            name: form.name.trim().to_string(),
            category: form.category.trim().to_string(),
            logo_url,
            channel_type: form.channel_type.clone(),
            sort_order,
            loop_anchor,
        },
    )
    .await
    .map_err(internal_error)?;

    Ok(Redirect::to("/admin/channels"))
}

pub async fn channel_edit_form(
    State(state): State<AppState>,
    Path(id): Path<i64>,
) -> Result<Html<String>, StatusCode> {
    let ch = channel::get(&state.pool, id)
        .await
        .map_err(internal_error)?
        .ok_or(StatusCode::NOT_FOUND)?;

    render(ChannelFormTemplate {
        is_edit: true,
        channel_id: ch.id,
        name: ch.name,
        category: ch.category,
        channel_type: ch.r#type,
        sort_order: ch.sort_order,
        logo_url: ch.logo_url.unwrap_or_default(),
        loop_anchor: ch
            .loop_anchor
            .map(|dt| dt.format("%Y-%m-%dT%H:%M").to_string())
            .unwrap_or_default(),
    })
}

pub async fn channel_update(
    State(state): State<AppState>,
    Path(id): Path<i64>,
    Form(form): Form<ChannelForm>,
) -> Result<impl IntoResponse, StatusCode> {
    if !["live", "vod_loop"].contains(&form.channel_type.as_str()) {
        return Err(StatusCode::UNPROCESSABLE_ENTITY);
    }
    if form.name.trim().is_empty() || form.category.trim().is_empty() {
        return Err(StatusCode::UNPROCESSABLE_ENTITY);
    }
    let existing = channel::get(&state.pool, id)
        .await
        .map_err(internal_error)?
        .ok_or(StatusCode::NOT_FOUND)?;

    let sort_order: i64 = form.sort_order.trim().parse().unwrap_or(0);
    let logo_url = if form.logo_url.trim().is_empty() {
        None
    } else {
        Some(form.logo_url.trim().to_string())
    };
    let loop_anchor = if form.channel_type.as_str() == "vod_loop" {
        parse_loop_anchor(&form.loop_anchor).or(existing.loop_anchor)
    } else {
        None
    };

    channel::update(
        &state.pool,
        id,
        channel::UpdateChannel {
            name: form.name.trim().to_string(),
            category: form.category.trim().to_string(),
            logo_url,
            channel_type: form.channel_type.clone(),
            sort_order,
            loop_anchor,
        },
    )
    .await
    .map_err(internal_error)?;

    Ok(Redirect::to("/admin/channels"))
}

pub async fn channel_delete(
    State(state): State<AppState>,
    Path(id): Path<i64>,
) -> Result<impl IntoResponse, StatusCode> {
    let found = channel::delete(&state.pool, id)
        .await
        .map_err(internal_error)?;
    if !found {
        return Err(StatusCode::NOT_FOUND);
    }
    Ok(Redirect::to("/admin/channels"))
}

pub async fn channel_detail(
    State(state): State<AppState>,
    Path(id): Path<i64>,
) -> Result<Html<String>, StatusCode> {
    let ch = channel::get(&state.pool, id)
        .await
        .map_err(internal_error)?
        .ok_or(StatusCode::NOT_FOUND)?;

    let srcs = source::list_for_channel(&state.pool, id)
        .await
        .map_err(internal_error)?;

    let items = playlist_item::list_for_channel(&state.pool, id)
        .await
        .map_err(internal_error)?;

    render(ChannelDetailTemplate {
        channel_id: ch.id,
        channel_name: ch.name,
        channel_type: ch.r#type,
        sources: srcs
            .into_iter()
            .map(|s| AdminSourceRow {
                id: s.id,
                kind: s.kind,
                url: s.url,
                priority: s.priority,
                is_active: s.is_active,
            })
            .collect(),
        playlist_items: items
            .into_iter()
            .map(|i| AdminPlaylistItemRow {
                id: i.id,
                title: i.title,
                url: i.url,
                duration_secs: i.duration_secs,
                sort_order: i.sort_order,
            })
            .collect(),
    })
}

pub async fn source_create(
    State(state): State<AppState>,
    Path(channel_id): Path<i64>,
    Form(form): Form<SourceForm>,
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
    let src = sqlx::query_as::<_, source::Source>(
        "SELECT * FROM sources WHERE id = ?",
    )
    .bind(source_id)
    .fetch_optional(&state.pool)
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
    let src = sqlx::query_as::<_, source::Source>(
        "SELECT * FROM sources WHERE id = ?",
    )
    .bind(source_id)
    .fetch_optional(&state.pool)
    .await
    .map_err(internal_error)?
    .ok_or(StatusCode::NOT_FOUND)?;

    source::set_active(&state.pool, source_id, !src.is_active)
        .await
        .map_err(internal_error)?;
    Ok(Redirect::to(&format!("/admin/channels/{}", src.channel_id)))
}

pub async fn playlist_item_create(
    State(state): State<AppState>,
    Path(channel_id): Path<i64>,
    Form(form): Form<PlaylistItemForm>,
) -> Result<impl IntoResponse, StatusCode> {
    let duration_secs: i64 = form.duration_secs.trim().parse().unwrap_or(0);
    if duration_secs <= 0 {
        return Err(StatusCode::UNPROCESSABLE_ENTITY);
    }

    let existing = playlist_item::list_for_channel(&state.pool, channel_id)
        .await
        .map_err(internal_error)?;
    let sort_order = existing.len() as i64;

    playlist_item::create(
        &state.pool,
        playlist_item::NewPlaylistItem {
            channel_id,
            title: form.title.trim().to_string(),
            url: form.url.trim().to_string(),
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
    let item = sqlx::query_as::<_, playlist_item::PlaylistItem>(
        "SELECT * FROM playlist_items WHERE id = ?",
    )
    .bind(item_id)
    .fetch_optional(&state.pool)
    .await
    .map_err(internal_error)?
    .ok_or(StatusCode::NOT_FOUND)?;

    playlist_item::delete(&state.pool, item_id)
        .await
        .map_err(internal_error)?;
    Ok(Redirect::to(&format!("/admin/channels/{}", item.channel_id)))
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn test_check_basic_auth_valid_credentials() {
        assert!(check_basic_auth("Basic dXNlcjpzZWNyZXQ=", "secret"));
    }

    #[test]
    fn test_check_basic_auth_wrong_password() {
        assert!(!check_basic_auth("Basic dXNlcjp3cm9uZw==", "secret"));
    }

    #[test]
    fn test_check_basic_auth_malformed_no_basic_prefix() {
        assert!(!check_basic_auth("Bearer sometoken", "secret"));
    }

    #[test]
    fn test_check_basic_auth_empty_header() {
        assert!(!check_basic_auth("", "secret"));
    }

    #[test]
    fn test_check_basic_auth_no_colon_in_credentials() {
        assert!(!check_basic_auth("Basic cGFzc3dvcmRvbmx5", "passwordonly"));
    }

    #[test]
    fn test_check_basic_auth_password_containing_colon() {
        // base64("user:pass:word") = "dXNlcjpwYXNzOndvcmQ="
        assert!(check_basic_auth("Basic dXNlcjpwYXNzOndvcmQ=", "pass:word"));
    }
}
