mod badges;
mod data;
mod layout;

use askama::Template;
use axum::{
    extract::{Path, Query, State},
    http::StatusCode,
    response::Html,
};
use serde::Deserialize;

use crate::{model::channel, AppState};

use data::{build_guide_data, ChannelRow, GuideData};
use layout::TimeLabel;

// ── template structs ───────────────────────────────────────────────────────

macro_rules! define_guide_template {
    ($name:ident, $path:literal) => {
        #[derive(Template)]
        #[template(path = $path)]
        struct $name {
            categories: Vec<String>,
            active_category: String,
            offset_hours: i64,
            offset_prev: i64,
            offset_next: i64,
            window_label: String,
            labels: Vec<TimeLabel>,
            now_pct: Option<f64>,
            rows: Vec<ChannelRow>,
            channels_json: String,
        }

        impl From<GuideData> for $name {
            fn from(d: GuideData) -> Self {
                Self {
                    categories: d.categories,
                    active_category: d.active_category,
                    offset_hours: d.offset_hours,
                    offset_prev: d.offset_prev,
                    offset_next: d.offset_next,
                    window_label: d.window_label,
                    labels: d.labels,
                    now_pct: d.now_pct,
                    rows: d.rows,
                    channels_json: d.channels_json,
                }
            }
        }
    };
}

define_guide_template!(EpgContentTemplate, "partials/epg_content.html");

#[derive(Template)]
#[template(path = "guide.html")]
struct GuidePageTemplate {
    categories: Vec<String>,
    active_category: String,
    offset_hours: i64,
    offset_prev: i64,
    offset_next: i64,
    window_label: String,
    labels: Vec<TimeLabel>,
    now_pct: Option<f64>,
    rows: Vec<ChannelRow>,
    channels_json: String,
    auto_tune_channel_id: Option<i64>,
}

impl From<GuideData> for GuidePageTemplate {
    fn from(d: GuideData) -> Self {
        Self {
            categories: d.categories,
            active_category: d.active_category,
            offset_hours: d.offset_hours,
            offset_prev: d.offset_prev,
            offset_next: d.offset_next,
            window_label: d.window_label,
            labels: d.labels,
            now_pct: d.now_pct,
            rows: d.rows,
            channels_json: d.channels_json,
            auto_tune_channel_id: None,
        }
    }
}

// ── query params ───────────────────────────────────────────────────────────

#[derive(Debug, Deserialize)]
pub struct GuideQuery {
    pub category: Option<String>,
    pub offset: Option<i64>,
}

fn parse_query(params: GuideQuery) -> (String, i64) {
    let category = params.category.unwrap_or_else(|| "all".to_string());
    let offset_hours = params.offset.unwrap_or(-2).clamp(-48, 48);
    (category, offset_hours)
}

async fn load_data(state: &AppState, params: GuideQuery) -> Result<GuideData, StatusCode> {
    let (category, offset_hours) = parse_query(params);
    let cors_snapshot = state.cors_cache.read().await.clone();
    let live_snapshot: std::collections::HashMap<String, crate::media::resolver::LiveStatus> =
        state
            .live_cache
            .read()
            .await
            .iter()
            .map(|(url, (status, _))| (url.clone(), *status))
            .collect();
    build_guide_data(
        &state.pool,
        &cors_snapshot,
        &live_snapshot,
        &category,
        offset_hours,
    )
    .await
    .map_err(|e| {
        tracing::error!("guide data error: {e}");
        StatusCode::INTERNAL_SERVER_ERROR
    })
}

fn render_or_500<T: Template>(t: T) -> Result<Html<String>, StatusCode> {
    t.render().map(Html).map_err(|e| {
        tracing::error!("template render error: {e}");
        StatusCode::INTERNAL_SERVER_ERROR
    })
}

// ── handlers ───────────────────────────────────────────────────────────────

pub async fn guide_page(
    State(state): State<AppState>,
    Query(params): Query<GuideQuery>,
) -> Result<Html<String>, StatusCode> {
    let data = load_data(&state, params).await?;
    render_or_500(GuidePageTemplate::from(data))
}

pub async fn watch_page(
    State(state): State<AppState>,
    Path(channel_id): Path<i64>,
    Query(params): Query<GuideQuery>,
) -> Result<Html<String>, StatusCode> {
    let exists = channel::get(&state.pool, channel_id)
        .await
        .map_err(|_| StatusCode::INTERNAL_SERVER_ERROR)?
        .is_some();
    let data = load_data(&state, params).await?;
    let mut tpl = GuidePageTemplate::from(data);
    if exists {
        tpl.auto_tune_channel_id = Some(channel_id);
    }
    render_or_500(tpl)
}

pub async fn guide_partial(
    State(state): State<AppState>,
    Query(params): Query<GuideQuery>,
) -> Result<Html<String>, StatusCode> {
    let data = load_data(&state, params).await?;
    render_or_500(EpgContentTemplate::from(data))
}
