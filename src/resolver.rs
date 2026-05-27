use anyhow::{bail, Result};
use std::process::Command;

/// Returns true if the URL requires yt-dlp to obtain a playable stream.
/// Direct HLS and plain IPTV stream URLs are used as-is.
pub fn needs_resolution(url: &str) -> bool {
    url.contains("youtube.com")
        || url.contains("youtu.be")
        || url.contains("twitch.tv")
}

/// Returns a directly playable URL.
/// HLS/IPTV URLs are returned unchanged. YouTube/Twitch are resolved via yt-dlp.
pub fn resolve_url(url: &str) -> Result<String> {
    if !needs_resolution(url) {
        return Ok(url.to_string());
    }
    let output = Command::new("yt-dlp")
        .args(["-g", "--no-playlist", url])
        .output()?;
    if !output.status.success() {
        bail!(
            "yt-dlp failed for {}: {}",
            url,
            String::from_utf8_lossy(&output.stderr).trim()
        );
    }
    let resolved = String::from_utf8(output.stdout)?;
    let first_line = resolved.lines().next().unwrap_or("").trim().to_string();
    if first_line.is_empty() {
        bail!("yt-dlp returned empty output for {}", url);
    }
    Ok(first_line)
}

/// Fetches the duration of a video in seconds via yt-dlp.
/// Called once when an admin adds a VOD asset so duration is stored in the DB.
pub fn fetch_duration_secs(url: &str) -> Result<i64> {
    let output = Command::new("yt-dlp")
        .args(["--print", "duration", "--no-playlist", url])
        .output()?;
    if !output.status.success() {
        bail!(
            "yt-dlp failed for {}: {}",
            url,
            String::from_utf8_lossy(&output.stderr).trim()
        );
    }
    let raw = String::from_utf8(output.stdout)?;
    let trimmed = raw.trim();
    let duration: f64 = trimmed
        .parse()
        .map_err(|_| anyhow::anyhow!("could not parse yt-dlp duration: {:?}", trimmed))?;
    Ok(duration as i64)
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

    #[test]
    fn test_resolve_url_passthrough_for_hls() {
        let url = "https://example.com/live/stream.m3u8";
        let result = resolve_url(url).unwrap();
        assert_eq!(result, url);
    }

    #[test]
    fn test_resolve_url_passthrough_for_plain_iptv() {
        let url = "https://iptv.example.com/channel/999/index";
        let result = resolve_url(url).unwrap();
        assert_eq!(result, url);
    }

    #[test]
    #[ignore = "requires yt-dlp installed and network access — run manually"]
    fn test_resolve_youtube_live_returns_hls_url() {
        let url = "https://www.youtube.com/watch?v=jfKfPfyJRdk";
        let result = resolve_url(url);
        assert!(result.is_ok(), "expected resolved URL, got: {:?}", result);
        let resolved = result.unwrap();
        assert!(resolved.starts_with("https://"), "expected HTTPS URL, got: {}", resolved);
    }

    #[test]
    #[ignore = "requires yt-dlp installed and network access — run manually"]
    fn test_fetch_duration_returns_seconds() {
        let url = "https://www.youtube.com/watch?v=dQw4w9WgXcQ";
        let result = fetch_duration_secs(url);
        assert!(result.is_ok(), "expected duration, got: {:?}", result);
        let secs = result.unwrap();
        assert!(secs > 0, "duration should be positive");
    }
}
