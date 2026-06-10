use anyhow::{bail, Result};
use std::time::Duration;
use tokio::process::Command;
use tokio::sync::Semaphore;

/// Global cap on concurrent `yt-dlp` subprocesses. Each yt-dlp invocation holds
/// ~73 MB; on the 256 MB production VM, an unbounded fan-out of live-status
/// probes OOMs the box. Two permits bounds peak yt-dlp memory to ~150 MB.
fn yt_dlp_semaphore() -> &'static Semaphore {
    static SEM: Semaphore = Semaphore::const_new(2);
    &SEM
}

/// Waits up to `wait` for a yt-dlp permit, then builds and awaits the future via
/// `f`. Returns `None` if no permit becomes free within `wait` (load-shed: the
/// caller is not parked indefinitely on a busy box). `f` is a closure so the
/// future it builds — typically a `timeout` around the yt-dlp command — starts
/// only AFTER the permit is held, so a queued caller never burns its own command
/// timeout budget while waiting for a slot.
async fn run_under_cap<F, Fut>(sem: &Semaphore, wait: Duration, f: F) -> Option<Fut::Output>
where
    F: FnOnce() -> Fut,
    Fut: std::future::Future,
{
    let _permit = match tokio::time::timeout(wait, sem.acquire()).await {
        Ok(Ok(permit)) => permit,
        _ => return None, // no slot within `wait`, or semaphore closed (never)
    };
    Some(f().await)
}

/// Why a yt-dlp invocation produced no usable `Output`.
#[derive(Debug)]
enum YtDlpError {
    InvalidScheme,
    /// No permit free within the wait — load-shed, not queued.
    Busy,
    /// Permit held, but the command exceeded its timeout.
    Timeout,
    Spawn(std::io::Error),
}

/// Single entry point for spawning yt-dlp. Owns the invariants every caller
/// must uphold: the URL scheme check, the global concurrency cap
/// (`run_under_cap`), the command timeout, `kill_on_drop` (a timed-out or
/// cancelled invocation must not leave an orphaned ~73 MB process behind),
/// and the `--` argument guard. A non-zero exit is `Ok` — callers inspect
/// `status`/`stderr` (`probe_live` reads stderr of failed runs).
async fn yt_dlp_output(
    args: &[&str],
    url: &str,
    wait: Duration,
    cmd_timeout: Duration,
) -> Result<std::process::Output, YtDlpError> {
    if !url.starts_with("http://") && !url.starts_with("https://") {
        return Err(YtDlpError::InvalidScheme);
    }
    run_under_cap(yt_dlp_semaphore(), wait, || {
        tokio::time::timeout(
            cmd_timeout,
            Command::new("yt-dlp")
                .kill_on_drop(true)
                .args(args)
                .args(["--", url])
                .output(),
        )
    })
    .await
    .ok_or(YtDlpError::Busy)?
    .map_err(|_| YtDlpError::Timeout)?
    .map_err(YtDlpError::Spawn)
}

/// Maps a `YtDlpError` to the error strings the admin UI already shows.
fn yt_dlp_anyhow(err: YtDlpError, url: &str) -> anyhow::Error {
    match err {
        YtDlpError::InvalidScheme => anyhow::anyhow!("invalid URL scheme: {}", url),
        YtDlpError::Busy => {
            anyhow::anyhow!("yt-dlp resolver busy (no free slot) for {}", url)
        }
        YtDlpError::Timeout => anyhow::anyhow!("yt-dlp timed out after 30s for {}", url),
        YtDlpError::Spawn(e) => e.into(),
    }
}

/// Runs `yt-dlp --print <field>` under the cap and returns trimmed,
/// non-empty stdout. Shared body of `fetch_title`, `fetch_video_id`,
/// and `fetch_duration_secs`.
async fn yt_dlp_print(field: &str, url: &str) -> Result<String> {
    let output = yt_dlp_output(
        &["--print", field, "--no-playlist"],
        url,
        Duration::from_secs(15),
        Duration::from_secs(30),
    )
    .await
    .map_err(|e| yt_dlp_anyhow(e, url))?;
    if !output.status.success() {
        bail!(
            "yt-dlp failed for {}: {}",
            url,
            String::from_utf8_lossy(&output.stderr).trim()
        );
    }
    let value = String::from_utf8_lossy(&output.stdout).trim().to_string();
    if value.is_empty() {
        bail!("yt-dlp returned empty output for {}", url);
    }
    Ok(value)
}

/// Result of probing whether a source URL is currently broadcasting.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum LiveStatus {
    Live,
    Offline,
    Unknown,
}

/// Maps `yt-dlp --print is_live` output to a `LiveStatus`. On success, stdout is
/// authoritative (`True`/`False`; anything else — e.g. `None` for VODs — is
/// Unknown). On failure, a "not currently live" stderr means Offline (yt-dlp
/// exits non-zero for channels with no active broadcast); any other failure is
/// Unknown.
pub fn interpret_is_live(success: bool, stdout: &str, stderr: &str) -> LiveStatus {
    let out = stdout.trim();
    if success && out == "True" {
        return LiveStatus::Live;
    }
    if success && out == "False" {
        return LiveStatus::Offline;
    }
    if !success && stderr.to_ascii_lowercase().contains("not currently live") {
        return LiveStatus::Offline;
    }
    LiveStatus::Unknown
}

/// Probes whether a YouTube/Twitch live URL is currently broadcasting.
/// Times out after 8s; any spawn or timeout failure yields `Unknown`.
pub async fn probe_live(url: &str) -> LiveStatus {
    match yt_dlp_output(
        &["--print", "is_live", "--no-playlist"],
        url,
        Duration::from_secs(8),
        Duration::from_secs(8),
    )
    .await
    {
        Ok(output) => interpret_is_live(
            output.status.success(),
            &String::from_utf8_lossy(&output.stdout),
            &String::from_utf8_lossy(&output.stderr),
        ),
        Err(_) => LiveStatus::Unknown,
    }
}

/// Returns a cached live status if probed within the last 60s, otherwise probes
/// via `probe_live`, stores the result, and returns it.
pub async fn cached_live_status(cache: &crate::LiveStatusCache, url: &str) -> LiveStatus {
    {
        let map = cache.read().await;
        if let Some((status, at)) = map.get(url) {
            let ttl = match status {
                LiveStatus::Unknown => Duration::from_secs(10),
                _ => Duration::from_secs(60),
            };
            if at.elapsed() < ttl {
                return *status;
            }
        }
    }
    let status = probe_live(url).await;
    cache
        .write()
        .await
        .insert(url.to_string(), (status, std::time::Instant::now()));
    status
}

/// Returns true if the URL requires yt-dlp to obtain a playable stream.
/// Direct HLS and plain IPTV stream URLs are used as-is.
pub fn needs_resolution(url: &str) -> bool {
    url.contains("youtube.com") || url.contains("youtu.be") || url.contains("twitch.tv")
}

/// Returns true if a resolved YouTube manifest URL belongs to an ended live
/// broadcast. yt-dlp marks finished live HLS manifests with `force_finished/1`,
/// which leaves the player on a frozen playlist (black screen).
pub fn is_finished_live(resolved_url: &str) -> bool {
    resolved_url.contains("force_finished/1")
}

/// Rewrites a YouTube *live* URL that embeds a video id into the canonical
/// `watch?v=<id>` form, which yt-dlp resolves to the recorded MP4 once the
/// broadcast ends. Returns `None` for forms with no id in the path
/// (channel/handle `/live`) and for URLs already in `watch?v=` form.
pub fn live_url_to_watch_url(source_url: &str) -> Option<String> {
    let tail = source_url
        .split("youtube.com/live/")
        .nth(1)
        .or_else(|| source_url.split("youtu.be/").nth(1))?;
    let id = tail.split(['?', '&', '/']).next().unwrap_or("").trim();
    if id.is_empty() {
        return None;
    }
    Some(format!("https://www.youtube.com/watch?v={id}"))
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
    let output = yt_dlp_output(
        &["-g", "--no-playlist", "-f", "b[ext=mp4]/b"],
        url,
        Duration::from_secs(15),
        Duration::from_secs(30),
    )
    .await
    .map_err(|e| yt_dlp_anyhow(e, url))?;
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
    yt_dlp_print("title", url).await
}

/// Fetches the canonical video id of a YouTube URL via yt-dlp. Used to build a
/// `watch?v=<id>` URL when an ended live source carries no id in its path
/// (channel/handle live URLs).
pub async fn fetch_video_id(url: &str) -> Result<String> {
    yt_dlp_print("id", url).await
}

/// Fetches the duration of a video in seconds via yt-dlp.
/// Called once when an admin adds a VOD asset so duration is stored in the DB.
pub async fn fetch_duration_secs(url: &str) -> Result<i64> {
    let raw = yt_dlp_print("duration", url).await?;
    let duration: f64 = raw
        .parse()
        .map_err(|_| anyhow::anyhow!("could not parse yt-dlp duration: {:?}", raw))?;
    if !duration.is_finite() || duration < 0.0 {
        bail!("yt-dlp returned invalid duration: {}", duration);
    }
    Ok(duration.round() as i64)
}

#[cfg(test)]
mod tests {
    use super::*;

    #[tokio::test]
    async fn run_under_cap_runs_when_permit_available() {
        let sem = tokio::sync::Semaphore::new(1);
        let out = run_under_cap(&sem, Duration::from_secs(1), || async { 42 }).await;
        assert_eq!(out, Some(42));
    }

    #[tokio::test]
    async fn run_under_cap_returns_none_when_no_permit_within_wait() {
        let sem = tokio::sync::Semaphore::new(1);
        let _held = sem.acquire().await.unwrap(); // exhaust the only permit
        let out = run_under_cap(&sem, Duration::from_millis(20), || async { 42 }).await;
        assert_eq!(out, None, "must shed (None) when no permit within the wait");
    }

    #[tokio::test]
    async fn run_under_cap_returns_none_when_both_permits_held() {
        let sem = tokio::sync::Semaphore::new(2);
        let _p1 = sem.acquire().await.unwrap();
        let _p2 = sem.acquire().await.unwrap();
        let out = run_under_cap(&sem, Duration::from_millis(20), || async { 42 }).await;
        assert_eq!(out, None, "third caller sheds when 2 permits are held");
    }

    #[tokio::test]
    async fn run_under_cap_acquires_when_permit_frees_within_wait() {
        let sem = std::sync::Arc::new(tokio::sync::Semaphore::new(1));
        let held = sem.clone().acquire_owned().await.unwrap();
        let sem2 = sem.clone();
        let task = tokio::spawn(async move {
            run_under_cap(sem2.as_ref(), Duration::from_millis(500), || async { 7 }).await
        });
        tokio::time::sleep(Duration::from_millis(50)).await;
        drop(held); // free the permit well within the 500ms wait
        assert_eq!(task.await.unwrap(), Some(7));
    }

    #[tokio::test]
    async fn yt_dlp_output_rejects_non_http_scheme() {
        let err = yt_dlp_output(
            &["--print", "title", "--no-playlist"],
            "ftp://example.com/video",
            Duration::from_secs(1),
            Duration::from_secs(1),
        )
        .await
        .unwrap_err();
        assert!(matches!(err, YtDlpError::InvalidScheme));
    }

    #[tokio::test]
    async fn yt_dlp_print_maps_invalid_scheme_to_existing_message() {
        let err = yt_dlp_print("title", "ftp://example.com/video")
            .await
            .unwrap_err();
        assert!(err.to_string().contains("invalid URL scheme"));
    }

    #[tokio::test]
    async fn probe_live_non_http_is_unknown() {
        assert_eq!(probe_live("not-a-url").await, LiveStatus::Unknown);
    }

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

    #[test]
    fn test_is_finished_live() {
        assert!(is_finished_live(
            "https://r5---sn-x.googlevideo.com/a/force_finished/1/b/index.m3u8"
        ));
        assert!(!is_finished_live(
            "https://r5---sn-x.googlevideo.com/a/id/abc/b/index.m3u8"
        ));
    }

    #[test]
    fn test_live_url_to_watch_url() {
        assert_eq!(
            live_url_to_watch_url("https://www.youtube.com/live/abc123"),
            Some("https://www.youtube.com/watch?v=abc123".to_string())
        );
        assert_eq!(
            live_url_to_watch_url("https://youtu.be/abc123?feature=share"),
            Some("https://www.youtube.com/watch?v=abc123".to_string())
        );
        assert_eq!(
            live_url_to_watch_url("https://www.youtube.com/@somechannel/live"),
            None
        );
        assert_eq!(
            live_url_to_watch_url("https://www.youtube.com/watch?v=abc123"),
            None
        );
    }

    #[tokio::test]
    #[ignore = "requires yt-dlp installed and network access — run manually"]
    async fn test_fetch_video_id_returns_id() {
        let url = "https://www.youtube.com/watch?v=dQw4w9WgXcQ";
        let id = fetch_video_id(url).await.unwrap();
        assert_eq!(id, "dQw4w9WgXcQ");
    }

    #[test]
    fn interpret_is_live_maps_all_cases() {
        assert_eq!(interpret_is_live(true, "True\n", ""), LiveStatus::Live);
        assert_eq!(interpret_is_live(true, "False\n", ""), LiveStatus::Offline);
        assert_eq!(
            interpret_is_live(
                false,
                "",
                "ERROR: [youtube:tab] UCxx: The channel is not currently live"
            ),
            LiveStatus::Offline
        );
        assert_eq!(
            interpret_is_live(false, "", "ERROR: network unreachable"),
            LiveStatus::Unknown
        );
        assert_eq!(interpret_is_live(true, "", ""), LiveStatus::Unknown);
        // success=true with a "not currently live" stderr is still Unknown (stdout is authoritative on success)
        assert_eq!(
            interpret_is_live(true, "", "not currently live"),
            LiveStatus::Unknown
        );
        // yt-dlp prints "None" for a null is_live field → Unknown
        assert_eq!(interpret_is_live(true, "None\n", ""), LiveStatus::Unknown);
    }

    #[tokio::test]
    async fn cached_live_status_returns_fresh_cache_hit() {
        let cache: crate::LiveStatusCache =
            std::sync::Arc::new(tokio::sync::RwLock::new(std::collections::HashMap::new()));
        cache.write().await.insert(
            "https://www.youtube.com/@x/live".to_string(),
            (LiveStatus::Live, std::time::Instant::now()),
        );
        // A fresh cache hit returns immediately without invoking yt-dlp.
        assert_eq!(
            cached_live_status(&cache, "https://www.youtube.com/@x/live").await,
            LiveStatus::Live
        );
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
