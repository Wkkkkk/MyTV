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
    let s = crate::status::compute(true, "youtube_live", None, None, Some(status));
    let b = crate::status::status_badge(&s);
    LiveStatusBadgeTemplate {
        symbol: b.glyph,
        color: b.color,
        label: b.label,
        title: b.title,
    }
}

/// `GET /admin/live-status?url=<source-url>` — HTMX lazy-load endpoint returning
/// a small badge partial (● live / ◷ upcoming / ◌ ended / ◉ recorded / ▶ vod / ○ offline / · ?). Source rows and discovery
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

        assert_eq!(badge_parts(LiveStatus::PostLive).label, "recorded");
        assert_eq!(badge_parts(LiveStatus::WasLive).label, "recorded");
        assert_eq!(badge_parts(LiveStatus::NotLive).label, "offline");
        assert_eq!(badge_parts(LiveStatus::Offline).label, "offline");
        assert_eq!(badge_parts(LiveStatus::Unknown).label, "?");
    }
}
