use std::collections::HashMap;

use crate::media::hls::extract_manifest_host;
use crate::media::resolver::is_direct_media_file;

#[derive(Debug, Clone, Copy, PartialEq)]
pub enum BudgetStatus {
    Direct,
    Proxied,
    Unknown,
}

/// Derives the network-budget status for a single source URL from the CORS cache.
/// HTTP URLs are always `Proxied` (mixed content) without a cache lookup.
/// Direct media files (e.g. self-hosted MP4) play via `<video src>` and skip the
/// proxy entirely (idea #44); a media element loads cross-origin without CORS, so
/// the CORS verdict is irrelevant and they are always `Direct`.
pub fn status_for_url(url: &str, cors_cache: &HashMap<String, bool>) -> BudgetStatus {
    if url.starts_with("http://") {
        return BudgetStatus::Proxied;
    }
    if is_direct_media_file(url) {
        return BudgetStatus::Direct;
    }
    match cors_cache.get(&extract_manifest_host(url)) {
        Some(&true) => BudgetStatus::Direct,
        Some(&false) => BudgetStatus::Proxied,
        None => BudgetStatus::Unknown,
    }
}

/// Convenience: the (CSS class, glyph) badge pair for a URL given the CORS cache.
pub fn badge_for_url(
    url: &str,
    cors_cache: &HashMap<String, bool>,
) -> (&'static str, &'static str) {
    budget_badge(status_for_url(url, cors_cache))
}

/// Maps a budget status to a (CSS class, glyph) pair. Unknown renders an empty glyph.
pub fn budget_badge(status: BudgetStatus) -> (&'static str, &'static str) {
    match status {
        BudgetStatus::Direct => ("budget-direct", "⚡"),
        BudgetStatus::Proxied => ("budget-proxied", "☁"),
        BudgetStatus::Unknown => ("budget-unknown", ""),
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn test_status_for_url_http_always_proxied() {
        assert_eq!(
            status_for_url("http://example.com/stream.m3u8", &HashMap::new()),
            BudgetStatus::Proxied
        );
    }

    #[test]
    fn test_status_for_url_https_cache_hit_direct() {
        let mut cache = HashMap::new();
        cache.insert("https://example.com".to_string(), true);
        assert_eq!(
            status_for_url("https://example.com/stream.m3u8", &cache),
            BudgetStatus::Direct
        );
    }

    #[test]
    fn test_status_for_url_https_cache_hit_proxied() {
        let mut cache = HashMap::new();
        cache.insert("https://example.com".to_string(), false);
        assert_eq!(
            status_for_url("https://example.com/stream.m3u8", &cache),
            BudgetStatus::Proxied
        );
    }

    #[test]
    fn test_status_for_url_https_cache_miss_unknown() {
        assert_eq!(
            status_for_url("https://example.com/stream.m3u8", &HashMap::new()),
            BudgetStatus::Unknown
        );
    }

    #[test]
    fn test_status_for_url_direct_media_file_is_direct_without_cors() {
        // A self-hosted .mp4 plays via `<video src>` and skips the proxy (idea #44),
        // so it is Direct even when the host sends no CORS header (empty cache).
        assert_eq!(
            status_for_url("https://pub-abc.r2.dev/video/clip.mp4", &HashMap::new()),
            BudgetStatus::Direct
        );
        // Query/fragment must not defeat the extension check.
        assert_eq!(
            status_for_url("https://cdn.example.com/a.MP4?v=2", &HashMap::new()),
            BudgetStatus::Direct
        );
    }

    #[test]
    fn test_status_for_url_http_direct_media_still_proxied() {
        // Mixed content: an http:// media file must still proxy regardless of type.
        assert_eq!(
            status_for_url("http://example.com/clip.mp4", &HashMap::new()),
            BudgetStatus::Proxied
        );
    }
}
