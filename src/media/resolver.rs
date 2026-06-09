use anyhow::{bail, Result};
use std::time::Duration;
use tokio::process::Command;

/// Returns true if the URL requires yt-dlp to obtain a playable stream.
/// Direct HLS and plain IPTV stream URLs are used as-is.
pub fn needs_resolution(url: &str) -> bool {
    url.contains("youtube.com") || url.contains("youtu.be") || url.contains("twitch.tv")
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
            .args(["-g", "--no-playlist", "-f", "b[ext=mp4]/b", "--", url])
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

/// Fetches the title of a video via yt-dlp.
/// Used to pre-populate the channel name field in the manual URL resolve flow.
pub async fn fetch_title(url: &str) -> Result<String> {
    if !url.starts_with("http://") && !url.starts_with("https://") {
        bail!("invalid URL scheme: {}", url);
    }
    let output = tokio::time::timeout(
        Duration::from_secs(30),
        Command::new("yt-dlp")
            .args(["--print", "title", "--no-playlist", "--", url])
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
    let title = String::from_utf8_lossy(&output.stdout).trim().to_string();
    if title.is_empty() {
        bail!("yt-dlp returned empty title for {}", url);
    }
    Ok(title)
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

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn test_youtube_needs_resolution() {
        assert!(needs_resolution(
            "https://www.youtube.com/watch?v=dQw4w9WgXcQ"
        ));
        assert!(needs_resolution("https://youtu.be/dQw4w9WgXcQ"));
        assert!(needs_resolution(
            "https://www.youtube.com/channel/UCXXXXXX/live"
        ));
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
        assert!(
            resolved.starts_with("https://"),
            "expected HTTPS URL, got: {}",
            resolved
        );
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

    #[tokio::test]
    #[ignore = "requires yt-dlp installed and network access — run manually"]
    async fn test_resolve_youtube_vod_returns_single_line_mp4_url() {
        let url = "https://www.youtube.com/watch?v=dQw4w9WgXcQ";
        let result = resolve_url(url).await;
        assert!(result.is_ok(), "expected resolved URL, got: {:?}", result);
        let resolved = result.unwrap();
        assert!(
            !resolved.contains('\n'),
            "expected single-line URL (no separate audio stream), got multiple lines: {}",
            resolved
        );
        assert!(
            resolved.contains("mime=video%2Fmp4") || resolved.contains("video/mp4"),
            "expected video/mp4 URL, got: {}",
            resolved
        );
    }
}
