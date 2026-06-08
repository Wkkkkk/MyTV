use chrono::{DateTime, Utc};

use crate::{
    budget::{status_for_url, BudgetStatus},
    model::{channel::ChannelType, playlist_item},
};

#[derive(Debug, Clone, Copy, PartialEq)]
pub(super) enum HealthStatus {
    Healthy,
    Down,
    Unknown,
}

pub(super) fn category_icon(category: &str) -> &'static str {
    let c = category.to_lowercase();
    if c.contains("news") {
        return "📰";
    }
    if c.contains("sport") {
        return "⚽";
    }
    if c.contains("movie") || c.contains("film") || c.contains("cinema") {
        return "🎬";
    }
    if c.contains("music") {
        return "🎵";
    }
    if c.contains("kid") || c.contains("child") {
        return "🧒";
    }
    if c.contains("documentary") || c.contains("docu") {
        return "🎥";
    }
    if c.contains("entertainment") {
        return "🎭";
    }
    if c.contains("cooking") || c.contains("food") {
        return "🍳";
    }
    if c.contains("travel") {
        return "✈️";
    }
    if c.contains("science") || c.contains("tech") {
        return "🔬";
    }
    "📺"
}

pub(super) fn derive_health_status(
    channel_id: i64,
    channel_type: &ChannelType,
    all_source_ids: &std::collections::HashSet<i64>,
    active_source_ids: &std::collections::HashSet<i64>,
) -> HealthStatus {
    match channel_type {
        ChannelType::VodLoop => HealthStatus::Healthy,
        ChannelType::Live => {
            if !all_source_ids.contains(&channel_id) {
                return HealthStatus::Unknown;
            }
            if active_source_ids.contains(&channel_id) {
                HealthStatus::Healthy
            } else {
                HealthStatus::Down
            }
        }
    }
}

pub(super) fn budget_for_url(
    url: Option<&str>,
    cors_cache: &std::collections::HashMap<String, bool>,
) -> BudgetStatus {
    match url {
        Some(u) => status_for_url(u, cors_cache),
        None => BudgetStatus::Unknown,
    }
}

/// The URL whose host determines a VOD channel's guide budget badge: the
/// currently-playing item (via the loop anchor), falling back to the first item
/// when there is no anchor. `None` for an empty playlist.
pub(super) fn vod_budget_url(
    items: &[playlist_item::PlaylistItem],
    anchor: Option<DateTime<Utc>>,
    now: DateTime<Utc>,
) -> Option<String> {
    if items.is_empty() {
        return None;
    }
    let idx = match anchor {
        Some(a) => playlist_item::current_position(items, now.timestamp(), a.timestamp())
            .map(|(i, _)| i)
            .unwrap_or(0),
        None => 0,
    };
    Some(items[idx].url.clone())
}

pub(super) fn health_badge(status: HealthStatus) -> (&'static str, &'static str) {
    match status {
        HealthStatus::Healthy => ("health-ok", "●"),
        HealthStatus::Down => ("health-down", "●"),
        HealthStatus::Unknown => ("health-unknown", "○"),
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use std::collections::HashMap;

    fn dt(secs: i64) -> DateTime<Utc> {
        DateTime::from_timestamp(secs, 0).unwrap()
    }

    fn mk_item(url: &str, dur: i64) -> playlist_item::PlaylistItem {
        playlist_item::PlaylistItem {
            id: 0,
            channel_id: 1,
            title: "t".into(),
            url: url.into(),
            duration_secs: dur,
            sort_order: 0,
            is_active: true,
            last_checked_at: None,
            last_status: None,
            consecutive_failures: 0,
            failure_reason: None,
        }
    }

    #[test]
    fn test_category_icon_known_categories() {
        assert_eq!(category_icon("News"), "📰");
        assert_eq!(category_icon("SPORTS"), "⚽");
        assert_eq!(category_icon("Movies"), "🎬");
        assert_eq!(category_icon("Films"), "🎬");
        assert_eq!(category_icon("cinema"), "🎬");
        assert_eq!(category_icon("Music"), "🎵");
        assert_eq!(category_icon("Kids"), "🧒");
        assert_eq!(category_icon("Children"), "🧒");
        assert_eq!(category_icon("Documentary"), "🎥");
        assert_eq!(category_icon("Docu"), "🎥");
        assert_eq!(category_icon("Entertainment"), "🎭");
        assert_eq!(category_icon("Cooking"), "🍳");
        assert_eq!(category_icon("Food"), "🍳");
        assert_eq!(category_icon("Travel"), "✈️");
        assert_eq!(category_icon("Science"), "🔬");
        assert_eq!(category_icon("Tech"), "🔬");
        assert_eq!(category_icon("Unknown"), "📺");
        assert_eq!(category_icon(""), "📺");
    }

    #[test]
    fn test_derive_health_status_live_has_active_source() {
        use std::collections::HashSet;
        let all: HashSet<i64> = [1].into_iter().collect();
        let active: HashSet<i64> = [1].into_iter().collect();
        assert_eq!(
            derive_health_status(1, &ChannelType::Live, &all, &active),
            HealthStatus::Healthy
        );
    }

    #[test]
    fn test_derive_health_status_live_all_inactive() {
        use std::collections::HashSet;
        let all: HashSet<i64> = [1].into_iter().collect();
        let active: HashSet<i64> = HashSet::new();
        assert_eq!(
            derive_health_status(1, &ChannelType::Live, &all, &active),
            HealthStatus::Down
        );
    }

    #[test]
    fn test_derive_health_status_no_sources_unknown() {
        use std::collections::HashSet;
        let all: HashSet<i64> = HashSet::new();
        let active: HashSet<i64> = HashSet::new();
        assert_eq!(
            derive_health_status(1, &ChannelType::Live, &all, &active),
            HealthStatus::Unknown
        );
    }

    #[test]
    fn test_derive_health_status_vod_always_healthy() {
        use std::collections::HashSet;
        let all: HashSet<i64> = [1].into_iter().collect();
        let active: HashSet<i64> = HashSet::new();
        assert_eq!(
            derive_health_status(1, &ChannelType::VodLoop, &all, &active),
            HealthStatus::Healthy
        );
    }

    #[test]
    fn test_derive_health_status_vod_no_sources_still_healthy() {
        use std::collections::HashSet;
        let all: HashSet<i64> = HashSet::new();
        let active: HashSet<i64> = HashSet::new();
        assert_eq!(
            derive_health_status(1, &ChannelType::VodLoop, &all, &active),
            HealthStatus::Healthy
        );
    }

    #[test]
    fn test_budget_for_url_none_is_unknown() {
        assert_eq!(budget_for_url(None, &HashMap::new()), BudgetStatus::Unknown);
    }

    #[test]
    fn test_budget_for_url_http_is_proxied() {
        assert_eq!(
            budget_for_url(Some("http://x.example.com/s.m3u8"), &HashMap::new()),
            BudgetStatus::Proxied
        );
    }

    #[test]
    fn test_budget_for_url_https_cache_hit_direct() {
        let mut cache = HashMap::new();
        cache.insert("https://x.example.com".to_string(), true);
        assert_eq!(
            budget_for_url(Some("https://x.example.com/s.m3u8"), &cache),
            BudgetStatus::Direct
        );
    }

    #[test]
    fn test_vod_budget_url_empty_is_none() {
        assert_eq!(vod_budget_url(&[], None, dt(0)), None);
    }

    #[test]
    fn test_vod_budget_url_no_anchor_uses_first_item() {
        let items = vec![
            mk_item("https://a/1.mp4", 100),
            mk_item("https://b/2.mp4", 100),
        ];
        assert_eq!(
            vod_budget_url(&items, None, dt(150)).as_deref(),
            Some("https://a/1.mp4")
        );
    }

    #[test]
    fn test_vod_budget_url_uses_currently_playing_item() {
        let items = vec![
            mk_item("https://a/1.mp4", 100),
            mk_item("https://b/2.mp4", 100),
        ];
        // anchor=0, now=150 → 150s into the loop → second item (after the first 100s)
        assert_eq!(
            vod_budget_url(&items, Some(dt(0)), dt(150)).as_deref(),
            Some("https://b/2.mp4")
        );
    }

    #[test]
    fn test_vod_budget_url_zero_duration_falls_back_to_first_item() {
        // All items have duration 0 → current_position returns None → fall back to item 0.
        let items = vec![mk_item("https://a/1.mp4", 0), mk_item("https://b/2.mp4", 0)];
        assert_eq!(
            vod_budget_url(&items, Some(dt(0)), dt(150)).as_deref(),
            Some("https://a/1.mp4")
        );
    }
}
