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
    let base_dir = base_url
        .rsplit_once('/')
        .map(|(b, _)| b)
        .unwrap_or(base_url);
    let origin = {
        let after_scheme = base_url.find("://").map(|i| i + 3).unwrap_or(0);
        let host_len = base_url[after_scheme..]
            .find('/')
            .unwrap_or(base_url[after_scheme..].len());
        &base_url[..after_scheme + host_len]
    };

    content
        .lines()
        .map(|line| {
            if line.starts_with('#') || line.is_empty() {
                return line.to_string();
            }
            let abs = if line.starts_with("http://") || line.starts_with("https://") {
                line.to_string()
            } else if line.starts_with('/') {
                format!("{}{}", origin, line)
            } else {
                format!("{}/{}", base_dir, line)
            };
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

/// Resolves a URI from an HLS manifest relative to the manifest's own URL.
fn resolve_uri(uri: &str, base_url: &str) -> String {
    if uri.starts_with("http://") || uri.starts_with("https://") {
        return uri.to_string();
    }
    if uri.starts_with('/') {
        let after_scheme = base_url.find("://").map(|i| i + 3).unwrap_or(0);
        let host_len = base_url[after_scheme..]
            .find('/')
            .unwrap_or(base_url[after_scheme..].len());
        let origin = &base_url[..after_scheme + host_len];
        return format!("{}{}", origin, uri);
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
#[allow(dead_code)]
pub fn find_first_segment_url(content: &str, base_url: &str) -> Option<String> {
    let base_dir = base_url
        .rsplit_once('/')
        .map(|(b, _)| b)
        .unwrap_or(base_url);
    let after_scheme = base_url.find("://").map(|i| i + 3).unwrap_or(0);
    let host_len = base_url[after_scheme..]
        .find('/')
        .unwrap_or(base_url[after_scheme..].len());
    let origin = &base_url[..after_scheme + host_len];

    for line in content.lines() {
        if line.starts_with('#') || line.is_empty() {
            continue;
        }
        let lower = line.to_lowercase();
        let path = lower.split('?').next().unwrap_or(&lower);
        if path.ends_with(".m3u8") || path.ends_with(".m3u") {
            continue;
        }
        let abs = if line.starts_with("http://") || line.starts_with("https://") {
            line.to_string()
        } else if line.starts_with('/') {
            format!("{}{}", origin, line)
        } else {
            format!("{}/{}", base_dir, line)
        };
        return Some(abs);
    }
    None
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
}
