use askama::Template;
use axum::{
    extract::{Query, State},
    http::StatusCode,
    response::Html,
};
use serde::Deserialize;

use crate::media::resolver::{self, LiveStatus};
use crate::routes::render;
use crate::AppState;

#[derive(Deserialize)]
pub struct LiveStatusQuery {
    pub url: String,
}

#[derive(Template)]
#[template(path = "admin/partials/live_status_badge.html")]
struct LiveStatusBadgeTemplate {
    symbol: &'static str,
    color: &'static str,
    label: &'static str,
    title: &'static str,
}

fn badge_parts(status: LiveStatus) -> LiveStatusBadgeTemplate {
    match status {
        LiveStatus::Live => LiveStatusBadgeTemplate {
            symbol: "●",
            color: "#4caf50",
            label: "live",
            title: "Currently live",
        },
        LiveStatus::Offline => LiveStatusBadgeTemplate {
            symbol: "○",
            color: "#888",
            label: "offline",
            title: "Not currently live",
        },
        LiveStatus::Unknown => LiveStatusBadgeTemplate {
            symbol: "·",
            color: "#666",
            label: "?",
            title: "Live status unknown",
        },
    }
}

pub async fn live_status_badge(
    State(state): State<AppState>,
    Query(q): Query<LiveStatusQuery>,
) -> Result<Html<String>, StatusCode> {
    let status = if resolver::needs_resolution(&q.url) {
        resolver::cached_live_status(&state.live_cache, &q.url).await
    } else {
        LiveStatus::Unknown
    };
    render(badge_parts(status))
}
