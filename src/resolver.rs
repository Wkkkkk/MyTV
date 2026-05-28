use anyhow::{bail, Result};
use std::time::Duration;
use tokio::process::Command;

/// Returns true if the URL requires yt-dlp to obtain a playable stream.
/// Direct HLS and plain IPTV stream URLs are used as-is.
pub fn needs_resolution(url: &str) -> bool {
    url.contains("youtube.com")
        || url.contains("youtu.be")
        || url.contains("twitch.tv")
}

/// Returns a directly playable URL.
/// HLS/IPTV URLs are returned unchanged. YouTube/Twitch are resolved via yt-dlp.
pub async fn resolve_url(url: &str) -> Result<String> {
    if !url.starts_with("http://") && !url.starts_with("https://") {
        bail!("invalid URL scheme: {}", url);
    }
    if !needs_resolution(url) {
        return Ok(url.to_string());
    }
    let output = tokio::time::timeout(
        Duration::from_secs(30),
        Command::new("yt-dlp")
            .args(["-g", "--no-playlist", "--", url])
            .output(),
    )
    .await
    .map_err(|_| anyhow::anyhow!("yt-dlp timed out after 30s for {}", url))??;

    if !output.status.success() {
        bail!(
            "yt-dlp failed for {}: {}",
            url,
            String::from_utf8_lossy(&output.stderr).trim()
        );
    }
    let resolved = String::from_utf8_lossy(&output.stdout).into_owned();
    let first_line = resolved.lines().next().unwrap_or("").trim().to_string();
    if first_line.is_empty() {
        bail!("yt-dlp returned empty output for {}", url);
    }
    Ok(first_line)
}

/// Fetches the duration of a video in seconds via yt-dlp.
/// Called once when an admin adds a VOD asset so duration is stored in the DB.
pub async fn fetch_duration_secs(url: &str) -> Result<i64> {
    if !url.starts_with("http://") && !url.starts_with("https://") {
        bail!("invalid URL scheme: {}", url);
    }
    let output = tokio::time::timeout(
        Duration::from_secs(30),
        Command::new("yt-dlp")
            .args(["--print", "duration", "--no-playlist", "--", url])
            .output(),
    )
    .await
    .map_err(|_| anyhow::anyhow!("yt-dlp timed out after 30s for {}", url))??;

    if !output.status.success() {
        bail!(
            "yt-dlp failed for {}: {}",
            url,
            String::from_utf8_lossy(&output.stderr).trim()
        );
    }
    let raw = String::from_utf8_lossy(&output.stdout).into_owned();
    let trimmed = raw.trim();
    let duration: f64 = trimmed
        .parse()
        .map_err(|_| anyhow::anyhow!("could not parse yt-dlp duration: {:?}", trimmed))?;
    if !duration.is_finite() || duration < 0.0 {
        bail!("yt-dlp returned invalid duration: {}", duration);
    }
    Ok(duration.round() as i64)
}

/// Fetches the total duration of an HLS VOD stream by parsing its manifest.
/// Follows master playlists to the first variant. Returns an error for live streams.
pub async fn fetch_hls_duration(client: &reqwest::Client, url: &str) -> Result<i64> {
    if !url.starts_with("http://") && !url.starts_with("https://") {
        bail!("invalid URL scheme: {}", url);
    }
    let text = client
        .get(url)
        .timeout(Duration::from_secs(10))
        .send()
        .await?
        .error_for_status()?
        .text()
        .await?;

    // Master playlist → recurse into first variant
    if text.contains("#EXT-X-STREAM-INF") {
        let variant = first_variant_url(&text, url)
            .ok_or_else(|| anyhow::anyhow!("no variant found in master playlist: {}", url))?;
        return Box::pin(fetch_hls_duration(client, &variant)).await;
    }

    // Live streams have no EXT-X-ENDLIST and no fixed duration
    if !text.contains("#EXT-X-ENDLIST") {
        bail!("live HLS stream has no fixed duration: {}", url);
    }

    let total: f64 = text
        .lines()
        .filter_map(|line| {
            let rest = line.strip_prefix("#EXTINF:")?;
            rest.split(',').next()?.parse::<f64>().ok()
        })
        .sum();

    if total <= 0.0 {
        bail!("could not parse duration from HLS manifest: {}", url);
    }
    Ok(total.ceil() as i64)
}

fn first_variant_url(manifest: &str, base_url: &str) -> Option<String> {
    let base_dir = base_url.rsplit_once('/')?.0;
    let origin = {
        let after = base_url.find("://")? + 3;
        let host_len = base_url[after..].find('/').unwrap_or(base_url[after..].len());
        &base_url[..after + host_len]
    };
    let mut next_is_url = false;
    for line in manifest.lines() {
        if line.starts_with("#EXT-X-STREAM-INF") {
            next_is_url = true;
        } else if next_is_url && !line.starts_with('#') && !line.is_empty() {
            return Some(if line.starts_with("http://") || line.starts_with("https://") {
                line.to_string()
            } else if line.starts_with('/') {
                format!("{}{}", origin, line)
            } else {
                format!("{}/{}", base_dir, line)
            });
        } else if !line.is_empty() {
            next_is_url = false;
        }
    }
    None
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn test_youtube_needs_resolution() {
        assert!(needs_resolution("https://www.youtube.com/watch?v=dQw4w9WgXcQ"));
        assert!(needs_resolution("https://youtu.be/dQw4w9WgXcQ"));
        assert!(needs_resolution("https://www.youtube.com/channel/UCXXXXXX/live"));
        assert!(needs_resolution("https://www.twitch.tv/somestream"));
    }

    #[test]
    fn test_hls_does_not_need_resolution() {
        assert!(!needs_resolution("https://example.com/stream.m3u8"));
        assert!(!needs_resolution("https://live.example.com/hls/index.m3u8"));
        assert!(!needs_resolution("https://iptv.example.com/channel/1"));
        assert!(!needs_resolution("https://vimeo.com/123456789"));
    }

    #[tokio::test]
    async fn test_resolve_url_passthrough_for_hls() {
        let url = "https://example.com/live/stream.m3u8";
        let result = resolve_url(url).await.unwrap();
        assert_eq!(result, url);
    }

    #[tokio::test]
    async fn test_resolve_url_passthrough_for_plain_iptv() {
        let url = "https://iptv.example.com/channel/999/index";
        let result = resolve_url(url).await.unwrap();
        assert_eq!(result, url);
    }

    #[tokio::test]
    #[ignore = "requires yt-dlp installed and network access — run manually"]
    async fn test_resolve_youtube_live_returns_hls_url() {
        let url = "https://www.youtube.com/watch?v=jfKfPfyJRdk";
        let result = resolve_url(url).await;
        assert!(result.is_ok(), "expected resolved URL, got: {:?}", result);
        let resolved = result.unwrap();
        assert!(resolved.starts_with("https://"), "expected HTTPS URL, got: {}", resolved);
    }

    #[tokio::test]
    #[ignore = "requires yt-dlp installed and network access — run manually"]
    async fn test_fetch_duration_returns_seconds() {
        let url = "https://www.youtube.com/watch?v=dQw4w9WgXcQ";
        let result = fetch_duration_secs(url).await;
        assert!(result.is_ok(), "expected duration, got: {:?}", result);
        let secs = result.unwrap();
        assert!(secs > 0, "duration should be positive");
    }
}
