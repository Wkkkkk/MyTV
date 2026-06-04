pub mod hls;
pub mod m3u;
pub mod resolver;

/// Fetches the duration (seconds) for a VOD URL.
/// Uses yt-dlp resolution for YouTube/resolvable URLs, HLS manifest parsing otherwise.
pub async fn fetch_duration(client: &reqwest::Client, url: &str) -> anyhow::Result<i64> {
    if resolver::needs_resolution(url) {
        resolver::fetch_duration_secs(url).await
    } else {
        hls::fetch_hls_duration(client, url).await
    }
}
