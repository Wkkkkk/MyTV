use chrono::{DateTime, Utc};

use crate::media::resolver::LiveStatus;
use crate::status::{self, SourceStatus};
use crate::{
    budget::{status_for_url, BudgetStatus},
    model::{channel::ChannelType, playlist_item},
};

/// Maps a free-form channel category to an on-brand inline SVG icon. The icon
/// uses `currentColor` (no hardcoded fill) so it tints to the surrounding text
/// in both the guide channel column and the player info-bar. The same
/// category→icon mapping is mirrored in `channelIcon()` in `base.html` for the
/// client-rendered info-bar.
pub(super) fn category_icon(category: &str) -> &'static str {
    let c = category.to_lowercase();
    if c.contains("news") {
        return CAT_NEWS;
    }
    if c.contains("sport") {
        return CAT_SPORT;
    }
    if c.contains("movie") || c.contains("film") || c.contains("cinema") {
        return CAT_MOVIE;
    }
    if c.contains("music") {
        return CAT_MUSIC;
    }
    if c.contains("kid") || c.contains("child") {
        return CAT_KIDS;
    }
    if c.contains("documentary") || c.contains("docu") {
        return CAT_DOCUMENTARY;
    }
    if c.contains("entertainment") {
        return CAT_ENTERTAINMENT;
    }
    if c.contains("cooking") || c.contains("food") {
        return CAT_COOKING;
    }
    if c.contains("travel") {
        return CAT_TRAVEL;
    }
    if c.contains("science") || c.contains("tech") {
        return CAT_SCIENCE;
    }
    CAT_GENERAL
}

const CAT_NEWS: &str = r#"<svg class="cat-icon" data-cat="news" viewBox="0 0 100 100" aria-hidden="true"><rect x="22" y="22" width="56" height="56" rx="11" fill="none" stroke="currentColor" stroke-width="7"/><g stroke="currentColor" stroke-linecap="round"><line x1="33" y1="38" x2="67" y2="38" stroke-width="7"/><line x1="33" y1="51" x2="67" y2="51" stroke-width="5"/><line x1="33" y1="61" x2="56" y2="61" stroke-width="5"/></g></svg>"#;
const CAT_SPORT: &str = r#"<svg class="cat-icon" data-cat="sport" viewBox="0 0 100 100" aria-hidden="true"><g fill="none" stroke="currentColor" stroke-linecap="round"><circle cx="50" cy="50" r="29" stroke-width="7"/><line x1="50" y1="21" x2="50" y2="79" stroke-width="5"/><line x1="21" y1="50" x2="79" y2="50" stroke-width="5"/><path d="M 31 28 Q 45 50 31 72" stroke-width="5"/><path d="M 69 28 Q 55 50 69 72" stroke-width="5"/></g></svg>"#;
const CAT_MOVIE: &str = r#"<svg class="cat-icon" data-cat="movie" viewBox="0 0 100 100" aria-hidden="true"><rect x="24" y="30" width="52" height="40" rx="9" fill="none" stroke="currentColor" stroke-width="7"/><g fill="currentColor"><circle cx="34" cy="37" r="2.6"/><circle cx="46" cy="37" r="2.6"/><circle cx="58" cy="37" r="2.6"/><circle cx="66" cy="37" r="2.6"/><circle cx="34" cy="63" r="2.6"/><circle cx="46" cy="63" r="2.6"/><circle cx="58" cy="63" r="2.6"/><circle cx="66" cy="63" r="2.6"/></g><path d="M 46 45 L 46 55 L 58 50 Z" fill="currentColor" stroke="currentColor" stroke-width="3" stroke-linejoin="round"/></svg>"#;
const CAT_MUSIC: &str = r#"<svg class="cat-icon" data-cat="music" viewBox="0 0 100 100" aria-hidden="true"><circle cx="38" cy="68" r="11" fill="currentColor"/><path d="M 49 68 L 49 30" fill="none" stroke="currentColor" stroke-width="7" stroke-linecap="round"/><path d="M 49 30 Q 68 33 65 52" fill="none" stroke="currentColor" stroke-width="7" stroke-linecap="round"/></svg>"#;
const CAT_KIDS: &str = r#"<svg class="cat-icon" data-cat="kids" viewBox="0 0 100 100" aria-hidden="true"><circle cx="50" cy="50" r="29" fill="none" stroke="currentColor" stroke-width="7"/><circle cx="40" cy="44" r="3.6" fill="currentColor"/><circle cx="60" cy="44" r="3.6" fill="currentColor"/><path d="M 37 58 Q 50 70 63 58" fill="none" stroke="currentColor" stroke-width="6" stroke-linecap="round"/></svg>"#;
const CAT_DOCUMENTARY: &str = r#"<svg class="cat-icon" data-cat="documentary" viewBox="0 0 100 100" aria-hidden="true"><circle cx="50" cy="50" r="28" fill="none" stroke="currentColor" stroke-width="7"/><line x1="22" y1="50" x2="78" y2="50" stroke="currentColor" stroke-width="5"/><ellipse cx="50" cy="50" rx="13" ry="28" fill="none" stroke="currentColor" stroke-width="5"/></svg>"#;
const CAT_ENTERTAINMENT: &str = r#"<svg class="cat-icon" data-cat="entertainment" viewBox="0 0 100 100" aria-hidden="true"><path d="M 50 18 Q 55 45 82 50 Q 55 55 50 82 Q 45 55 18 50 Q 45 45 50 18 Z" fill="currentColor" stroke="currentColor" stroke-width="3" stroke-linejoin="round"/></svg>"#;
const CAT_COOKING: &str = r#"<svg class="cat-icon" data-cat="cooking" viewBox="0 0 100 100" aria-hidden="true"><g fill="currentColor"><circle cx="37" cy="44" r="13"/><circle cx="63" cy="44" r="13"/><circle cx="50" cy="37" r="15"/><rect x="33" y="44" width="34" height="20" rx="8"/><rect x="35" y="60" width="30" height="16" rx="4"/></g></svg>"#;
const CAT_TRAVEL: &str = r#"<svg class="cat-icon" data-cat="travel" viewBox="0 0 100 100" aria-hidden="true"><g fill="none" stroke="currentColor" stroke-width="6" stroke-linejoin="round" stroke-linecap="round"><path d="M 22 52 L 82 24 L 62 80 L 50 58 Z"/><path d="M 82 24 L 50 58"/></g></svg>"#;
const CAT_SCIENCE: &str = r#"<svg class="cat-icon" data-cat="science" viewBox="0 0 100 100" aria-hidden="true"><g fill="none" stroke="currentColor" stroke-width="5"><ellipse cx="50" cy="50" rx="31" ry="12"/><ellipse cx="50" cy="50" rx="31" ry="12" transform="rotate(60 50 50)"/><ellipse cx="50" cy="50" rx="31" ry="12" transform="rotate(120 50 50)"/></g><circle cx="50" cy="50" r="6.5" fill="currentColor"/></svg>"#;
const CAT_GENERAL: &str = r#"<svg class="cat-icon" data-cat="general" viewBox="0 0 100 100" aria-hidden="true"><rect x="16" y="24" width="68" height="50" rx="14" fill="none" stroke="currentColor" stroke-width="7" stroke-linejoin="round"/><path d="M 43 40 L 43 58 L 61 49 Z" fill="currentColor" stroke="currentColor" stroke-width="5" stroke-linejoin="round" stroke-linecap="round"/></svg>"#;

/// Minimal per-source facts the guide needs to compute a channel's status.
pub(super) struct SourceFacts {
    pub kind: String,
    pub is_active: bool,
    pub last_status: Option<String>,
    pub failure_reason: Option<String>,
}

/// The channel's aggregated Status = the most-optimistic status across its
/// sources. For `youtube_live` sources the live status comes from the warm cache
/// snapshot (cold → Unchecked, never Down). VOD channels (no sources) are `Ok`
/// (reachable) — matching the prior "VOD always healthy" behavior.
pub(super) fn derive_channel_status(
    channel_type: &ChannelType,
    sources: &[SourceFacts],
    live_snapshot: &std::collections::HashMap<String, LiveStatus>,
    source_urls: &[String],
) -> SourceStatus {
    match channel_type {
        ChannelType::VodLoop | ChannelType::VodOnDemand => SourceStatus::Ok,
        ChannelType::Live => {
            if sources.is_empty() {
                return SourceStatus::Unchecked;
            }
            status::most_optimistic(sources.iter().zip(source_urls).map(|(f, url)| {
                let live = if f.kind == "youtube_live" {
                    live_snapshot.get(url).copied()
                } else {
                    None
                };
                status::compute(
                    f.is_active,
                    &f.kind,
                    f.last_status.as_deref(),
                    f.failure_reason.as_deref(),
                    live,
                )
            }))
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
            disabled_at: None,
        }
    }

    fn has_cat(category: &str, key: &str) -> bool {
        let svg = category_icon(category);
        svg.starts_with("<svg") && svg.contains(&format!(r#"data-cat="{key}""#))
    }

    #[test]
    fn test_category_icon_known_categories() {
        assert!(has_cat("News", "news"));
        assert!(has_cat("SPORTS", "sport"));
        assert!(has_cat("Movies", "movie"));
        assert!(has_cat("Films", "movie"));
        assert!(has_cat("cinema", "movie"));
        assert!(has_cat("Music", "music"));
        assert!(has_cat("Kids", "kids"));
        assert!(has_cat("Children", "kids"));
        assert!(has_cat("Documentary", "documentary"));
        assert!(has_cat("Docu", "documentary"));
        assert!(has_cat("Entertainment", "entertainment"));
        assert!(has_cat("Cooking", "cooking"));
        assert!(has_cat("Food", "cooking"));
        assert!(has_cat("Travel", "travel"));
        assert!(has_cat("Science", "science"));
        assert!(has_cat("Tech", "science"));
        assert!(has_cat("Unknown", "general"));
        assert!(has_cat("", "general"));
    }

    #[test]
    fn test_category_icon_inherits_currentcolor() {
        // Icons must not hardcode a fill color; they inherit currentColor so
        // they tint to the surrounding text in both the guide and info-bar.
        let svg = category_icon("News");
        assert!(svg.contains("currentColor"));
        assert!(!svg.contains("#e94560"));
        assert!(svg.contains(r#"class="cat-icon""#));
    }

    #[test]
    fn test_derive_channel_status_most_optimistic() {
        use std::collections::HashMap;
        let snapshot: HashMap<String, LiveStatus> = HashMap::new();
        let sources = vec![
            SourceFacts {
                kind: "hls".into(),
                is_active: true,
                last_status: Some("error".into()),
                failure_reason: Some("dead".into()),
            },
            SourceFacts {
                kind: "hls".into(),
                is_active: true,
                last_status: Some("ok".into()),
                failure_reason: None,
            },
        ];
        let urls = vec![
            "https://a/s.m3u8".to_string(),
            "https://b/s.m3u8".to_string(),
        ];
        assert_eq!(
            derive_channel_status(&ChannelType::Live, &sources, &snapshot, &urls),
            SourceStatus::Ok,
            "one OK source beats a Down sibling"
        );

        let no_sources: Vec<SourceFacts> = vec![];
        assert_eq!(
            derive_channel_status(&ChannelType::Live, &no_sources, &snapshot, &[]),
            SourceStatus::Unchecked
        );
        assert_eq!(
            derive_channel_status(&ChannelType::VodLoop, &no_sources, &snapshot, &[]),
            SourceStatus::Ok
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
