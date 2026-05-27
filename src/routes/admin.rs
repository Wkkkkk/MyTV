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

use crate::{channel, AppState};

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
