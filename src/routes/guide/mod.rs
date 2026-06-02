mod badges;
mod data;
mod layout;

use askama::Template;
use axum::{
    extract::{Query, State},
    http::StatusCode,
    response::Html,
};
use serde::Deserialize;

use crate::AppState;

use data::{build_guide_data, ChannelRow, GuideData};
use layout::TimeLabel;

// ── template structs ───────────────────────────────────────────────────────

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
}

#[derive(Template)]
#[template(path = "partials/epg_content.html")]
struct EpgContentTemplate {
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

/// Builds a guide template of type `$t` from a `GuideData`, moving every field across.
macro_rules! guide_template {
    ($t:ident, $d:expr) => {{
        let d = $d;
        $t {
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
    }};
}

async fn load_data(state: &AppState, params: GuideQuery) -> Result<GuideData, StatusCode> {
    let (category, offset_hours) = parse_query(params);
    let cors_snapshot = state.cors_cache.read().await.clone();
    build_guide_data(&state.pool, &cors_snapshot, &category, offset_hours)
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
    render_or_500(guide_template!(GuidePageTemplate, data))
}

pub async fn guide_partial(
    State(state): State<AppState>,
    Query(params): Query<GuideQuery>,
) -> Result<Html<String>, StatusCode> {
    let data = load_data(&state, params).await?;
    render_or_500(guide_template!(EpgContentTemplate, data))
}
