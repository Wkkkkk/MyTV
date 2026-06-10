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

/// Query string for `GET /admin/live-status` — the source URL to probe.
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
    title: String,
}

fn badge_parts(status: LiveStatus) -> LiveStatusBadgeTemplate {
    match status {
        LiveStatus::Live => LiveStatusBadgeTemplate {
            symbol: "●",
            color: "#4caf50",
            label: "live",
            title: "Currently live".to_string(),
        },
        LiveStatus::Upcoming(ts) => {
            let title = ts
                .and_then(|t| chrono::DateTime::from_timestamp(t, 0))
                .map(|dt| format!("Scheduled — starts {}", crate::media::format_utc_short(dt)))
                .unwrap_or_else(|| "Scheduled, start time unknown".to_string());
            LiveStatusBadgeTemplate {
                symbol: "◷",
                color: "#db4",
                label: "upcoming",
                title,
            }
        }
        LiveStatus::PostLive => LiveStatusBadgeTemplate {
            symbol: "◌",
            color: "#f77",
            label: "ended",
            title: "Broadcast just ended (still processing)".to_string(),
        },
        LiveStatus::WasLive => LiveStatusBadgeTemplate {
            symbol: "◉",
            color: "#88f",
            label: "recorded",
            title: "Finished broadcast — recording available".to_string(),
        },
        LiveStatus::NotLive => LiveStatusBadgeTemplate {
            symbol: "▶",
            color: "#88f",
            label: "vod",
            title: "Regular video (never live)".to_string(),
        },
        LiveStatus::Offline => LiveStatusBadgeTemplate {
            symbol: "○",
            color: "#888",
            label: "offline",
            title: "Not currently live".to_string(),
        },
        LiveStatus::Unknown => LiveStatusBadgeTemplate {
            symbol: "·",
            color: "#666",
            label: "?",
            title: "Live status unknown".to_string(),
        },
    }
}

/// `GET /admin/live-status?url=<source-url>` — HTMX lazy-load endpoint returning
/// a small badge partial (● live / ○ offline / · ?). Source rows and discovery
/// results render a "checking…" placeholder with `hx-trigger="load"` pointing
/// here, so the page itself never blocks on yt-dlp. Probes go through
/// `cached_live_status` (60s TTL, 10s for Unknown) and the global yt-dlp
/// concurrency cap; URLs that yt-dlp can't resolve render as Unknown without
/// spawning a probe.
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

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn badge_parts_maps_every_state() {
        assert_eq!(badge_parts(LiveStatus::Live).label, "live");

        let up = badge_parts(LiveStatus::Upcoming(Some(1781287200)));
        assert_eq!(up.label, "upcoming");
        assert_eq!(up.title, "Scheduled — starts Jun 12 18:00 UTC");
        assert_eq!(
            badge_parts(LiveStatus::Upcoming(None)).title,
            "Scheduled, start time unknown"
        );

        assert_eq!(badge_parts(LiveStatus::PostLive).label, "ended");
        assert_eq!(badge_parts(LiveStatus::WasLive).label, "recorded");
        assert_eq!(badge_parts(LiveStatus::NotLive).label, "vod");
        assert_eq!(badge_parts(LiveStatus::Offline).label, "offline");
        assert_eq!(badge_parts(LiveStatus::Unknown).label, "?");
    }
}
