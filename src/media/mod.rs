pub mod hls;
pub mod m3u;
pub mod mpd;
pub mod resolver;

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
