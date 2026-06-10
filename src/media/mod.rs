pub mod hls;
pub mod m3u;
pub mod mpd;
pub mod resolver;

/// Resolves `url` against `base_url`:
/// - Absolute (`http(s)://`) → returned unchanged
/// - Root-relative (`/path`) → prepended with `base_url`'s scheme+host
/// - Relative (including `./`) → resolved against `base_url`'s directory
pub(crate) fn resolve_url(url: &str, base_url: &str) -> String {
    if url.starts_with("http://") || url.starts_with("https://") {
        return url.to_string();
    }
    if url.starts_with('/') {
        let after_scheme = base_url.find("://").map(|i| i + 3).unwrap_or(0);
        let host_len = base_url[after_scheme..]
            .find('/')
            .unwrap_or(base_url[after_scheme..].len());
        return format!("{}{}", &base_url[..after_scheme + host_len], url);
    }
    let base_dir = base_url
        .rsplit_once('/')
        .map(|(b, _)| b)
        .unwrap_or(base_url);
    let stripped = url.trim_start_matches("./");
    if stripped.is_empty() {
        format!("{}/", base_dir)
    } else {
        format!("{}/{}", base_dir, stripped)
    }
}

/// Formats a UTC time as e.g. "Jun 12 18:00 UTC" — used by the discover
/// results (scheduled streams) and the live-status badge.
pub(crate) fn format_utc_short(dt: chrono::DateTime<chrono::Utc>) -> String {
    dt.format("%b %d %H:%M UTC").to_string()
}

/// Fetches the duration (seconds) for a VOD URL.
/// Uses yt-dlp for YouTube, MPD parsing for DASH, HLS manifest parsing otherwise.
pub async fn fetch_duration(client: &reqwest::Client, url: &str) -> anyhow::Result<i64> {
    if resolver::needs_resolution(url) {
        resolver::fetch_duration_secs(url).await
    } else if url.contains(".mpd") {
        let text = client
            .get(url)
            .timeout(std::time::Duration::from_secs(10))
            .send()
            .await?
            .error_for_status()?
            .text()
            .await?;
        mpd::parse_mpd_duration(&text)
    } else {
        hls::fetch_hls_duration(client, url).await
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn resolve_url_absolute_passthrough() {
        assert_eq!(
            resolve_url(
                "https://cdn.example.com/seg.ts",
                "https://origin.example.com/live/master.m3u8"
            ),
            "https://cdn.example.com/seg.ts"
        );
    }

    #[test]
    fn resolve_url_relative() {
        assert_eq!(
            resolve_url("variant.m3u8", "https://example.com/live/master.m3u8"),
            "https://example.com/live/variant.m3u8"
        );
    }

    #[test]
    fn resolve_url_root_relative() {
        assert_eq!(
            resolve_url("/hls/variant.m3u8", "https://example.com/live/master.m3u8"),
            "https://example.com/hls/variant.m3u8"
        );
    }

    #[test]
    fn resolve_url_dot_slash() {
        assert_eq!(
            resolve_url("./seg.ts", "https://cdn.example.com/path/manifest.m3u8"),
            "https://cdn.example.com/path/seg.ts"
        );
    }

    #[test]
    fn resolve_url_dot_slash_only() {
        assert_eq!(
            resolve_url("./", "https://cdn.example.com/path/stream.mpd"),
            "https://cdn.example.com/path/"
        );
    }

    #[test]
    fn format_utc_short_formats_epoch() {
        // 1_781_287_200 = 2026-06-12T18:00:00Z
        let dt = chrono::DateTime::from_timestamp(1_781_287_200, 0).unwrap();
        assert_eq!(format_utc_short(dt), "Jun 12 18:00 UTC");
    }
}
