use std::collections::HashMap;

use crate::media::hls::extract_manifest_host;

#[derive(Debug, Clone, Copy, PartialEq)]
pub enum BudgetStatus {
    Direct,
    Proxied,
    Unknown,
}

/// Derives the network-budget status for a single source URL from the CORS cache.
/// HTTP URLs are always `Proxied` (mixed content) without a cache lookup.
pub fn status_for_url(url: &str, cors_cache: &HashMap<String, bool>) -> BudgetStatus {
    if url.starts_with("http://") {
        return BudgetStatus::Proxied;
    }
    match cors_cache.get(&extract_manifest_host(url)) {
        Some(&true) => BudgetStatus::Direct,
        Some(&false) => BudgetStatus::Proxied,
        None => BudgetStatus::Unknown,
    }
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
}
