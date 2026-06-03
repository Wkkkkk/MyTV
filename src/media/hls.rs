use anyhow::{bail, Result};
use std::time::Duration;

/// Fetches the total duration of an HLS VOD stream by parsing its manifest.
/// Follows master playlists to the first variant. Returns an error for live streams.
pub async fn fetch_hls_duration(client: &reqwest::Client, url: &str) -> Result<i64> {
    if !url.starts_with("http://") && !url.starts_with("https://") {
        bail!("invalid URL scheme: {}", url);
    }
    let bytes = client
        .get(url)
        .timeout(Duration::from_secs(10))
        .send()
        .await?
        .error_for_status()?
        .bytes()
        .await?;

    match m3u8_rs::parse_playlist_res(&bytes)
        .map_err(|_| anyhow::anyhow!("failed to parse HLS playlist: {}", url))?
    {
        m3u8_rs::Playlist::MasterPlaylist(master) => {
            let uri = master
                .variants
                .first()
                .ok_or_else(|| anyhow::anyhow!("no variants in master playlist: {}", url))?
                .uri
                .clone();
            let variant_url = resolve_uri(&uri, url);
            Box::pin(fetch_hls_duration(client, &variant_url)).await
        }
        m3u8_rs::Playlist::MediaPlaylist(media) => {
            if !media.end_list {
                bail!("live HLS stream has no fixed duration: {}", url);
            }
            let total: f64 = media.segments.iter().map(|s| s.duration as f64).sum();
            if total <= 0.0 {
                bail!("could not parse duration from HLS manifest: {}", url);
            }
            Ok(total.ceil() as i64)
        }
    }
}

/// Rewrites all non-comment URLs in an HLS manifest to route through /stream-proxy.
/// When direct_segments is true, segment URLs (.ts, etc) are written as absolute URLs,
/// but playlist URLs (.m3u8) still route through /stream-proxy.
pub fn rewrite_hls_urls(content: &str, base_url: &str, direct_segments: bool) -> String {
    content
        .lines()
        .map(|line| {
            if line.starts_with('#') || line.is_empty() {
                return line.to_string();
            }
            let abs = resolve_uri(line, base_url);
            let lower = abs.to_lowercase();
            let path = lower.split('?').next().unwrap_or(&lower);
            if direct_segments && !path.ends_with(".m3u8") && !path.ends_with(".m3u") {
                abs
            } else {
                format!("/stream-proxy?url={}", pct_encode(&abs))
            }
        })
        .collect::<Vec<_>>()
        .join("\n")
}

/// Percent-encodes all non-unreserved characters (RFC 3986).
pub fn pct_encode(s: &str) -> String {
    s.bytes()
        .map(|b| match b {
            b'A'..=b'Z' | b'a'..=b'z' | b'0'..=b'9' | b'-' | b'_' | b'.' | b'~' => {
                (b as char).to_string()
            }
            _ => format!("%{:02X}", b),
        })
        .collect()
}

/// Returns the `scheme://host` prefix of a URL (no path, no query).
fn origin_of(url: &str) -> &str {
    let after_scheme = url.find("://").map(|i| i + 3).unwrap_or(0);
    let host_len = url[after_scheme..]
        .find('/')
        .unwrap_or(url[after_scheme..].len());
    &url[..after_scheme + host_len]
}

/// Resolves a URI from an HLS manifest relative to the manifest's own URL.
fn resolve_uri(uri: &str, base_url: &str) -> String {
    if uri.starts_with("http://") || uri.starts_with("https://") {
        return uri.to_string();
    }
    if uri.starts_with('/') {
        return format!("{}{}", origin_of(base_url), uri);
    }
    let base_dir = base_url
        .rsplit_once('/')
        .map(|(b, _)| b)
        .unwrap_or(base_url);
    format!("{}/{}", base_dir, uri)
}

/// Returns the first resolved absolute segment URL from an HLS media playlist.
/// Skips comment lines, empty lines, and sub-playlist lines (`.m3u8`/`.m3u`).
/// Returns `None` for master playlists that contain only sub-playlist lines.
pub fn find_first_segment_url(content: &str, base_url: &str) -> Option<String> {
    for line in content.lines() {
        if line.starts_with('#') || line.is_empty() {
            continue;
        }
        let lower = line.to_lowercase();
        let path = lower.split('?').next().unwrap_or(&lower);
        if path.ends_with(".m3u8") || path.ends_with(".m3u") {
            continue;
        }
        return Some(resolve_uri(line, base_url));
    }
    None
}

/// Extracts `scheme://host` from a URL, stripping any path/query.
/// This is the canonical CORS-cache key (the source-URL host).
pub fn extract_manifest_host(url: &str) -> String {
    origin_of(url).to_string()
}

/// Returns the first sub-playlist (`.m3u8`/`.m3u`) line in a master playlist,
/// resolved to an absolute URL. `None` if there is no sub-playlist line.
pub fn find_first_variant_url(content: &str, base_url: &str) -> Option<String> {
    for line in content.lines() {
        if line.starts_with('#') || line.is_empty() {
            continue;
        }
        let lower = line.to_lowercase();
        let path = lower.split('?').next().unwrap_or(&lower);
        if path.ends_with(".m3u8") || path.ends_with(".m3u") {
            return Some(resolve_uri(line, base_url));
        }
    }
    None
}

/// Finds a segment URL to CORS-probe, descending one level if `content` is a master playlist.
/// Returns `None` if no segment can be found within one descent.
pub async fn find_segment_with_descent(
    client: &reqwest::Client,
    content: &str,
    base_url: &str,
) -> Option<String> {
    if let Some(seg) = find_first_segment_url(content, base_url) {
        return Some(seg);
    }
    let variant = find_first_variant_url(content, base_url)?;
    if crate::ssrf::is_safe_url(&variant).await.is_err() {
        return None;
    }
    let body = fetch_text(client, &variant).await?;
    find_first_segment_url(&body, &variant)
}

/// Determines whether segments for `source_url` can be fetched directly by the browser.
/// `Some(true)` = direct (HTTPS segment with `Access-Control-Allow-Origin: *`),
/// `Some(false)` = must proxy (HTTP segment, or HTTPS without CORS),
/// `None` = could not determine (network error, or no segment after one descent).
pub async fn probe_source_cors(client: &reqwest::Client, source_url: &str) -> Option<bool> {
    let body = fetch_text(client, source_url).await?;
    let segment = find_segment_with_descent(client, &body, source_url).await?;
    if segment.starts_with("http://") {
        return Some(false);
    }
    Some(probe_cors(client, &segment).await)
}

async fn fetch_text(client: &reqwest::Client, url: &str) -> Option<String> {
    client
        .get(url)
        .timeout(Duration::from_secs(10))
        .send()
        .await
        .ok()?
        .text()
        .await
        .ok()
}

/// Returns true if the header map contains `Access-Control-Allow-Origin: *`.
pub fn has_cors_wildcard(headers: &reqwest::header::HeaderMap) -> bool {
    headers
        .get("access-control-allow-origin")
        .and_then(|v| v.to_str().ok())
        .map(|v| v.trim() == "*")
        .unwrap_or(false)
}

/// HEAD-requests `url` and returns true if the response includes `Access-Control-Allow-Origin: *`.
/// Returns false on any network or timeout error (proxy is the safe default).
pub async fn probe_cors(client: &reqwest::Client, url: &str) -> bool {
    if crate::ssrf::is_safe_url(url).await.is_err() {
        return false;
    }
    match client
        .head(url)
        .timeout(std::time::Duration::from_secs(5))
        .send()
        .await
    {
        Ok(resp) => has_cors_wildcard(resp.headers()),
        Err(e) => {
            tracing::debug!(url = %url, error = %e, "CORS probe failed, defaulting to proxy");
            false
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn test_rewrite_hls_urls_absolute() {
        let manifest = "#EXTM3U\nhttps://cdn.example.com/seg1.ts\n";
        let result = rewrite_hls_urls(
            manifest,
            "https://origin.example.com/live/index.m3u8",
            false,
        );
        assert!(result.contains("/stream-proxy?url=https%3A%2F%2Fcdn.example.com%2Fseg1.ts"));
    }

    #[test]
    fn test_rewrite_hls_urls_relative() {
        let manifest = "#EXTM3U\nseg1.ts\n";
        let result = rewrite_hls_urls(manifest, "https://example.com/live/index.m3u8", false);
        assert!(result.contains("/stream-proxy?url=https%3A%2F%2Fexample.com%2Flive%2Fseg1.ts"));
    }

    #[test]
    fn test_rewrite_hls_urls_root_relative() {
        let manifest = "#EXTM3U\n/hls/seg1.ts\n";
        let result = rewrite_hls_urls(manifest, "https://example.com/live/index.m3u8", false);
        assert!(result.contains("/stream-proxy?url=https%3A%2F%2Fexample.com%2Fhls%2Fseg1.ts"));
    }

    #[test]
    fn test_rewrite_hls_urls_leaves_comments_unchanged() {
        let manifest = "#EXTM3U\n#EXT-X-TARGETDURATION:6\nhttps://cdn.example.com/seg.ts\n";
        let result = rewrite_hls_urls(manifest, "https://example.com/index.m3u8", false);
        assert!(result.contains("#EXTM3U"));
        assert!(result.contains("#EXT-X-TARGETDURATION:6"));
    }

    #[test]
    fn test_pct_encode_unreserved_chars_unchanged() {
        assert_eq!(pct_encode("abc-ABC_123.~"), "abc-ABC_123.~");
    }

    #[test]
    fn test_pct_encode_special_chars() {
        assert_eq!(
            pct_encode("https://example.com/a?b=c&d=e"),
            "https%3A%2F%2Fexample.com%2Fa%3Fb%3Dc%26d%3De"
        );
    }

    #[test]
    fn test_resolve_uri_absolute() {
        assert_eq!(
            resolve_uri(
                "https://cdn.example.com/seg.ts",
                "https://example.com/index.m3u8"
            ),
            "https://cdn.example.com/seg.ts"
        );
    }

    #[test]
    fn test_resolve_uri_relative() {
        assert_eq!(
            resolve_uri("variant.m3u8", "https://example.com/live/master.m3u8"),
            "https://example.com/live/variant.m3u8"
        );
    }

    #[test]
    fn test_resolve_uri_root_relative() {
        assert_eq!(
            resolve_uri("/hls/variant.m3u8", "https://example.com/live/master.m3u8"),
            "https://example.com/hls/variant.m3u8"
        );
    }

    #[test]
    fn test_rewrite_hls_urls_direct_mode_segments_are_absolute() {
        let manifest = "#EXTM3U\nseg1.ts\n";
        let result = rewrite_hls_urls(manifest, "https://example.com/live/index.m3u8", true);
        assert!(result.contains("https://example.com/live/seg1.ts"));
        assert!(!result.contains("/stream-proxy"));
    }

    #[test]
    fn test_rewrite_hls_urls_direct_mode_playlists_still_proxied() {
        let manifest = "#EXTM3U\nvariant.m3u8\n";
        let result = rewrite_hls_urls(manifest, "https://example.com/master.m3u8", true);
        assert!(result.contains("/stream-proxy?url="));
        assert!(!result.contains("\nhttps://example.com/variant.m3u8"));
    }

    #[test]
    fn test_rewrite_hls_urls_proxy_mode_all_proxied() {
        let manifest = "#EXTM3U\nseg1.ts\n";
        let result = rewrite_hls_urls(manifest, "https://example.com/live/index.m3u8", false);
        assert!(result.contains("/stream-proxy?url="));
    }

    #[test]
    fn test_find_first_segment_url_returns_resolved_ts() {
        let manifest = "#EXTM3U\n#EXT-X-TARGETDURATION:6\nseg1.ts\nseg2.ts\n";
        let result = find_first_segment_url(manifest, "https://example.com/live/index.m3u8");
        assert_eq!(result, Some("https://example.com/live/seg1.ts".to_string()));
    }

    #[test]
    fn test_find_first_segment_url_skips_m3u8_lines() {
        let manifest = "#EXTM3U\nvariant.m3u8\nseg1.ts\n";
        let result = find_first_segment_url(manifest, "https://example.com/live/index.m3u8");
        assert_eq!(result, Some("https://example.com/live/seg1.ts".to_string()));
    }

    #[test]
    fn test_find_first_segment_url_returns_none_for_master_playlist() {
        let manifest = "#EXTM3U\n#EXT-X-STREAM-INF:BANDWIDTH=1000\nvariant.m3u8\n";
        let result = find_first_segment_url(manifest, "https://example.com/master.m3u8");
        assert_eq!(result, None);
    }

    #[test]
    fn test_find_first_segment_url_absolute_segment() {
        let manifest = "#EXTM3U\nhttps://cdn.example.com/seg1.ts\n";
        let result = find_first_segment_url(manifest, "https://example.com/live/index.m3u8");
        assert_eq!(result, Some("https://cdn.example.com/seg1.ts".to_string()));
    }

    #[test]
    fn test_find_first_segment_url_root_relative() {
        let manifest = "#EXTM3U\n/hls/seg1.ts\n";
        let result = find_first_segment_url(manifest, "https://example.com/live/index.m3u8");
        assert_eq!(result, Some("https://example.com/hls/seg1.ts".to_string()));
    }

    #[test]
    fn test_find_first_segment_url_returns_none_for_empty_manifest() {
        assert_eq!(
            find_first_segment_url("#EXTM3U\n", "https://example.com/index.m3u8"),
            None
        );
    }

    #[test]
    fn test_has_cors_wildcard_returns_true_for_star() {
        let mut headers = reqwest::header::HeaderMap::new();
        headers.insert(
            "access-control-allow-origin",
            reqwest::header::HeaderValue::from_static("*"),
        );
        assert!(has_cors_wildcard(&headers));
    }

    #[test]
    fn test_has_cors_wildcard_returns_false_when_absent() {
        assert!(!has_cors_wildcard(&reqwest::header::HeaderMap::new()));
    }

    #[test]
    fn test_has_cors_wildcard_returns_false_for_specific_origin() {
        let mut headers = reqwest::header::HeaderMap::new();
        headers.insert(
            "access-control-allow-origin",
            reqwest::header::HeaderValue::from_static("https://example.com"),
        );
        assert!(!has_cors_wildcard(&headers));
    }

    #[test]
    fn test_extract_manifest_host_strips_path() {
        assert_eq!(
            extract_manifest_host("https://cdn.example.com/live/index.m3u8"),
            "https://cdn.example.com"
        );
    }

    #[test]
    fn test_extract_manifest_host_no_path() {
        assert_eq!(
            extract_manifest_host("https://cdn.example.com"),
            "https://cdn.example.com"
        );
    }

    #[test]
    fn test_find_first_variant_url_resolves_relative() {
        let master = "#EXTM3U\n#EXT-X-STREAM-INF:BANDWIDTH=1\nvariant/720.m3u8\n";
        assert_eq!(
            find_first_variant_url(master, "https://h.com/live/master.m3u8"),
            Some("https://h.com/live/variant/720.m3u8".to_string())
        );
    }

    #[test]
    fn test_find_first_variant_url_none_for_media_playlist() {
        let media = "#EXTM3U\n#EXTINF:6,\nseg1.ts\n";
        assert_eq!(find_first_variant_url(media, "https://h.com/v.m3u8"), None);
    }

    #[tokio::test]
    async fn test_find_segment_with_descent_depth_zero() {
        // base already a variant: segment found without any network call
        let client = reqwest::Client::new();
        let media = "#EXTM3U\n#EXTINF:6,\nhttps://cdn.com/seg1.ts\n";
        let seg = find_segment_with_descent(&client, media, "https://h.com/v.m3u8").await;
        assert_eq!(seg.as_deref(), Some("https://cdn.com/seg1.ts"));
    }

    #[tokio::test]
    async fn find_segment_with_descent_blocks_variant_to_loopback() {
        let client = reqwest::Client::new();
        // Master playlist whose only variant line points to a loopback address.
        // Without the SSRF guard, find_segment_with_descent would fetch http://127.0.0.1/variant.m3u8.
        let master = "#EXTM3U\n#EXT-X-STREAM-INF:BANDWIDTH=1000\nhttp://127.0.0.1/variant.m3u8\n";
        let result =
            find_segment_with_descent(&client, master, "https://example.com/master.m3u8").await;
        assert_eq!(result, None);
    }

    #[tokio::test]
    async fn probe_cors_blocks_loopback() {
        let client = reqwest::Client::new();
        // Without the SSRF guard, probe_cors would HEAD-request http://127.0.0.1/seg.ts.
        let result = probe_cors(&client, "http://127.0.0.1/seg.ts").await;
        assert!(
            !result,
            "probe_cors must return false (proxy default) for loopback URLs"
        );
    }
}
