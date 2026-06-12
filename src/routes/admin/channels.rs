use askama::Template;
use axum::{
    extract::{Form, Path, State},
    http::StatusCode,
    response::{Html, IntoResponse, Redirect},
};
use chrono::Utc;
use serde::Deserialize;

use super::{AdminChannelRow, AdminPlaylistItemRow, AdminSourceRow};
use crate::routes::{internal_error, render};
use crate::{
    epg,
    model::{channel, playlist_item, source},
    AppState,
};

// ── local display types ────────────────────────────────────────────────────

struct AdminScheduleRow {
    is_current: bool,
    title: String,
    start_time: String,
    duration_secs: i64,
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
    vod_schedule: Vec<AdminScheduleRow>,
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

fn parse_sort_order(s: &str) -> Result<i64, StatusCode> {
    let trimmed = s.trim();
    if trimmed.is_empty() {
        Ok(0)
    } else {
        trimmed
            .parse()
            .map_err(|_| StatusCode::UNPROCESSABLE_ENTITY)
    }
}

// ── handlers ───────────────────────────────────────────────────────────────

pub async fn admin_index() -> impl IntoResponse {
    Redirect::to("/admin/channels")
}

pub async fn channel_list(State(state): State<AppState>) -> Result<Html<String>, StatusCode> {
    let all = channel::list(&state.pool).await.map_err(internal_error)?;
    let channels = all.into_iter().map(Into::into).collect();
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
    let sort_order = parse_sort_order(&form.sort_order)?;
    let new = channel::ChannelInput {
        name: form.name,
        category: form.category,
        channel_type: form.channel_type,
        sort_order,
        logo_url: Some(form.logo_url),
        loop_anchor: Some(form.loop_anchor),
    }
    .validate_new()
    .map_err(|_| StatusCode::UNPROCESSABLE_ENTITY)?;

    channel::create(&state.pool, new)
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
    let existing = channel::get(&state.pool, id)
        .await
        .map_err(internal_error)?
        .ok_or(StatusCode::NOT_FOUND)?;

    let sort_order = parse_sort_order(&form.sort_order)?;
    let upd = channel::ChannelInput {
        name: form.name,
        category: form.category,
        channel_type: form.channel_type,
        sort_order,
        logo_url: Some(form.logo_url),
        loop_anchor: Some(form.loop_anchor),
    }
    .validate_update(existing.loop_anchor)
    .map_err(|_| StatusCode::UNPROCESSABLE_ENTITY)?;

    channel::update(&state.pool, id, upd)
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

    let vod_schedule: Vec<AdminScheduleRow> = if ch.r#type == "vod_loop" {
        if let Some(anchor) = ch.loop_anchor {
            let total_dur: i64 = items.iter().map(|i| i.duration_secs).sum();
            if total_dur > 0 {
                let now = Utc::now();
                let window_end = now + chrono::Duration::seconds(2 * total_dur);
                epg::vod_schedule(ch.id, &items, anchor.timestamp(), now, window_end)
                    .into_iter()
                    .take(8)
                    .enumerate()
                    .map(|(i, e)| AdminScheduleRow {
                        is_current: i == 0,
                        title: e.title,
                        start_time: e.start_time.format("%H:%M UTC").to_string(),
                        duration_secs: (e.end_time - e.start_time).num_seconds(),
                    })
                    .collect()
            } else {
                vec![]
            }
        } else {
            vec![]
        }
    } else {
        vec![]
    };

    let sources: Vec<AdminSourceRow> = super::build_rows(srcs, &state.cors_cache).await;
    let playlist_items: Vec<AdminPlaylistItemRow> =
        super::build_rows(items, &state.cors_cache).await;

    render(ChannelDetailTemplate {
        channel_id: ch.id,
        channel_name: ch.name,
        channel_type: ch.r#type,
        sources,
        playlist_items,
        vod_schedule,
    })
}
