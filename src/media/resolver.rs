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
        YtDlpError::Timeout => anyhow::anyhow!("yt-dlp timed out for {}", url),
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

/// Result of probing a source URL's broadcast lifecycle state, mirroring
/// yt-dlp's `live_status` field. `Upcoming` carries the scheduled start
/// (`release_timestamp`, unix epoch) when yt-dlp reports one.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum LiveStatus {
    Live,
    Upcoming(Option<i64>),
    PostLive,
    WasLive,
    NotLive,
    Offline,
    Unknown,
}

/// Maps a yt-dlp `live_status` token to a `LiveStatus`. Carries no timestamp —
/// `is_upcoming` maps to `Upcoming(None)`; callers that also have a
/// `release_timestamp` attach it themselves.
pub fn live_status_from_str(token: &str) -> LiveStatus {
    match token {
        "is_live" => LiveStatus::Live,
        "is_upcoming" => LiveStatus::Upcoming(None),
        "post_live" => LiveStatus::PostLive,
        "was_live" => LiveStatus::WasLive,
        "not_live" => LiveStatus::NotLive,
        _ => LiveStatus::Unknown,
    }
}

/// Maps `yt-dlp --print "%(live_status)s|%(release_timestamp)s"` output to a
/// `LiveStatus`. On success, stdout is authoritative; `NA`/`None` (extractors
/// without a live_status) are Unknown. On failure, "not currently live" stderr
/// means Offline (yt-dlp exits non-zero for channels with no active broadcast)
/// and "live event will begin" means Upcoming (fallback in case
/// --ignore-no-formats-error does not suppress the error); any other failure
/// is Unknown.
pub fn interpret_live_status(success: bool, stdout: &str, stderr: &str) -> LiveStatus {
    if success {
        let out = stdout.lines().next().unwrap_or("").trim();
        let (status, ts) = out.split_once('|').unwrap_or((out, "NA"));
        return match live_status_from_str(status) {
            LiveStatus::Upcoming(_) => LiveStatus::Upcoming(ts.parse::<i64>().ok()),
            other => other,
        };
    }
    let err = stderr.to_ascii_lowercase();
    if err.contains("not currently live") {
        return LiveStatus::Offline;
    }
    if err.contains("live event will begin") {
        return LiveStatus::Upcoming(None);
    }
    LiveStatus::Unknown
}

/// Classifies a failed yt-dlp resolve. Returns `Some(status)` when the failure
/// is a recoverable broadcast state the player should wait on (`Offline` /
/// `Upcoming`), or `None` when it is a genuine error that should propagate.
pub fn recoverable_status(stderr: &str) -> Option<LiveStatus> {
    match interpret_live_status(false, "", stderr) {
        s @ (LiveStatus::Offline | LiveStatus::Upcoming(_)) => Some(s),
        _ => None,
    }
}

/// Probes a YouTube/Twitch URL's broadcast lifecycle state.
/// `--ignore-no-formats-error` lets yt-dlp print metadata for upcoming streams,
/// which have no formats yet. Times out after 8s; any spawn or timeout failure
/// yields `Unknown`.
pub async fn probe_live(url: &str) -> LiveStatus {
    match yt_dlp_output(
        &[
            "--print",
            "%(live_status)s|%(release_timestamp)s",
            "--ignore-no-formats-error",
            "--no-playlist",
        ],
        url,
        Duration::from_secs(8),
        Duration::from_secs(8),
    )
    .await
    {
        Ok(output) => interpret_live_status(
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

/// True when the URL is a self-contained media container playable directly via
/// the browser's `<video src>` (no manifest, no proxy needed). Strips any query
/// or fragment and is case-insensitive.
pub fn is_direct_media_file(url: &str) -> bool {
    let path = url
        .split(['?', '#'])
        .next()
        .unwrap_or(url)
        .to_ascii_lowercase();
    [".mp4", ".webm", ".m4v", ".mov"]
        .iter()
        .any(|ext| path.ends_with(ext))
}

/// Whether the player should bypass `/stream-proxy` for this URL: either it is
/// resolved via yt-dlp (YouTube/Twitch) or it is a direct media file served
/// from elsewhere (e.g. self-hosted object storage).
pub fn should_skip_proxy(url: &str) -> bool {
    needs_resolution(url) || is_direct_media_file(url)
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

/// Parses `--print live_status --print urls` stdout: line 1 is the status
/// token, line 2 the first playable URL (later lines are additional formats,
/// e.g. separate audio — the first-URL rule matches the old `-g` behavior).
fn parse_status_and_url(stdout: &str) -> Option<(String, LiveStatus)> {
    let mut lines = stdout.lines();
    let status = live_status_from_str(lines.next().unwrap_or("").trim());
    let url = lines.next().unwrap_or("").trim();
    if url.is_empty() {
        return None;
    }
    Some((url.to_string(), status))
}

/// Outcome of resolving a live source. A non-playable state is unrepresentable
/// as a URL: `Ended`/`Waiting` carry no string, so a caller cannot accidentally
/// treat them as playable. This replaces the old `(String, LiveStatus)` shape,
/// where an *empty* URL secretly meant "not playable".
#[derive(Debug, Clone, PartialEq, Eq)]
pub enum LiveResolution {
    /// A directly playable stream URL.
    Playable { url: String },
    /// The broadcast finished (recording available); the caller converts the
    /// channel to a VOD loop.
    Ended,
    /// The stream is offline or upcoming. The source stays active so the player
    /// backoff poll can resume it once the stream returns.
    Waiting,
}

/// Classifies a successfully resolved `(status, url)` pair. A finished live
/// broadcast — `was_live`/`post_live`, or a manifest carrying the
/// `force_finished` marker (the fallback for extractors without `live_status`)
/// — is `Ended`; anything else with a URL is `Playable`.
fn classify_resolved(status: LiveStatus, url: &str) -> LiveResolution {
    if matches!(status, LiveStatus::WasLive | LiveStatus::PostLive) || is_finished_live(url) {
        LiveResolution::Ended
    } else {
        LiveResolution::Playable {
            url: url.to_string(),
        }
    }
}

/// Resolves a source URL to its lifecycle outcome. HLS/IPTV URLs pass through
/// without a yt-dlp spawn — but still through `classify_resolved`, so a manifest
/// already carrying the `force_finished` marker is `Ended`, not `Playable`.
/// YouTube/Twitch are resolved via a single yt-dlp call that also reports
/// `live_status`, distinguishing a `Playable` stream from one that has `Ended`
/// (→ VOD conversion) or is `Waiting` (offline/upcoming). A genuine failure is
/// `Err`.
pub async fn resolve_live(url: &str) -> Result<LiveResolution> {
    if !url.starts_with("http://") && !url.starts_with("https://") {
        bail!("invalid URL scheme: {}", url);
    }
    if !needs_resolution(url) {
        return Ok(classify_resolved(LiveStatus::Unknown, url));
    }
    let output = yt_dlp_output(
        &[
            "--print",
            "live_status",
            "--print",
            "urls",
            "--no-playlist",
            "-f",
            "b[ext=mp4]/b",
        ],
        url,
        Duration::from_secs(15),
        Duration::from_secs(30),
    )
    .await
    .map_err(|e| yt_dlp_anyhow(e, url))?;
    if !output.status.success() {
        let stderr = String::from_utf8_lossy(&output.stderr);
        if recoverable_status(&stderr).is_some() {
            return Ok(LiveResolution::Waiting);
        }
        bail!("yt-dlp failed for {}: {}", url, stderr.trim());
    }
    parse_status_and_url(&String::from_utf8_lossy(&output.stdout))
        .map(|(resolved, status)| classify_resolved(status, &resolved))
        .ok_or_else(|| anyhow::anyhow!("yt-dlp returned empty output for {}", url))
}

/// Returns a directly playable URL.
/// HLS/IPTV URLs are returned unchanged. YouTube/Twitch are resolved via yt-dlp.
pub async fn resolve_url(url: &str) -> Result<String> {
    match resolve_live(url).await? {
        LiveResolution::Playable { url } => Ok(url),
        LiveResolution::Ended => bail!("broadcast ended for {url} (no live stream)"),
        LiveResolution::Waiting => {
            bail!("no playable URL for {url} (stream offline or upcoming)")
        }
    }
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

    #[test]
    fn test_is_direct_media_file() {
        assert!(is_direct_media_file("https://b.r2.dev/k/movie.mp4"));
        assert!(is_direct_media_file("https://b/x.MP4")); // case-insensitive
        assert!(is_direct_media_file("https://b/x.webm"));
        assert!(is_direct_media_file("https://b/x.m4v"));
        assert!(is_direct_media_file("https://b/x.mov"));
        assert!(is_direct_media_file("https://b/x.mp4?sig=abc&e=1")); // query stripped
        assert!(is_direct_media_file("https://b/x.mp4#t=10")); // fragment stripped
        assert!(!is_direct_media_file("https://b/playlist.m3u8"));
        assert!(!is_direct_media_file("https://b/manifest.mpd"));
        assert!(!is_direct_media_file("https://b/readme.txt"));
        assert!(!is_direct_media_file("https://b/video")); // no extension
        assert!(!is_direct_media_file("https://www.youtube.com/watch?v=abc"));
    }

    #[test]
    fn test_should_skip_proxy() {
        // resolved-via-yt-dlp sources skip the proxy as before…
        assert!(should_skip_proxy("https://www.youtube.com/watch?v=abc"));
        // …and so do direct media files (the new case)
        assert!(should_skip_proxy("https://bucket.r2.dev/k/movie.mp4"));
        // manifests and plain IPTV still proxy
        assert!(!should_skip_proxy("https://example.com/stream.m3u8"));
        assert!(!should_skip_proxy("https://iptv.example.com/channel/1"));
    }

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

    #[tokio::test]
    async fn resolve_url_rejects_non_http_scheme_before_passthrough() {
        // The leading scheme check must run BEFORE the needs_resolution
        // passthrough — without it, a non-YouTube ftp:// URL would be
        // returned as-is instead of rejected.
        let err = resolve_url("ftp://example.com/stream.m3u8")
            .await
            .unwrap_err();
        assert!(err.to_string().contains("invalid URL scheme"));
    }

    #[test]
    fn yt_dlp_anyhow_maps_all_variants() {
        let url = "https://example.com/x";
        assert_eq!(
            yt_dlp_anyhow(YtDlpError::InvalidScheme, url).to_string(),
            "invalid URL scheme: https://example.com/x"
        );
        assert_eq!(
            yt_dlp_anyhow(YtDlpError::Busy, url).to_string(),
            "yt-dlp resolver busy (no free slot) for https://example.com/x"
        );
        assert_eq!(
            yt_dlp_anyhow(YtDlpError::Timeout, url).to_string(),
            "yt-dlp timed out for https://example.com/x"
        );
        let spawn_err = std::io::Error::new(std::io::ErrorKind::NotFound, "no yt-dlp binary");
        assert!(yt_dlp_anyhow(YtDlpError::Spawn(spawn_err), url)
            .to_string()
            .contains("no yt-dlp binary"));
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
    fn live_status_from_str_maps_tokens() {
        use LiveStatus::*;
        assert_eq!(live_status_from_str("is_live"), Live);
        assert_eq!(live_status_from_str("is_upcoming"), Upcoming(None));
        assert_eq!(live_status_from_str("post_live"), PostLive);
        assert_eq!(live_status_from_str("was_live"), WasLive);
        assert_eq!(live_status_from_str("not_live"), NotLive);
        assert_eq!(live_status_from_str("NA"), Unknown);
        assert_eq!(live_status_from_str("None"), Unknown);
        assert_eq!(live_status_from_str(""), Unknown);
    }

    #[test]
    fn interpret_live_status_maps_all_cases() {
        use LiveStatus::*;
        assert_eq!(interpret_live_status(true, "is_live|NA\n", ""), Live);
        assert_eq!(
            interpret_live_status(true, "is_upcoming|1781287200\n", ""),
            Upcoming(Some(1781287200))
        );
        assert_eq!(
            interpret_live_status(true, "is_upcoming|1781287200\nis_upcoming|999\n", ""),
            Upcoming(Some(1781287200))
        );
        assert_eq!(
            interpret_live_status(true, "is_upcoming|NA\n", ""),
            Upcoming(None)
        );
        assert_eq!(interpret_live_status(true, "post_live|NA\n", ""), PostLive);
        assert_eq!(interpret_live_status(true, "was_live|NA\n", ""), WasLive);
        assert_eq!(interpret_live_status(true, "not_live|NA\n", ""), NotLive);
        assert_eq!(interpret_live_status(true, "NA|NA\n", ""), Unknown);
        assert_eq!(interpret_live_status(true, "None|None\n", ""), Unknown);
        assert_eq!(interpret_live_status(true, "", ""), Unknown);
        assert_eq!(
            interpret_live_status(
                false,
                "",
                "ERROR: [youtube] xyz: The channel is not currently live"
            ),
            Offline
        );
        assert_eq!(
            interpret_live_status(
                false,
                "",
                "ERROR: [youtube] xyz: This live event will begin in 3 hours"
            ),
            Upcoming(None)
        );
        assert_eq!(
            interpret_live_status(false, "", "ERROR: network unreachable"),
            Unknown
        );
    }

    #[tokio::test]
    async fn cached_live_status_upcoming_is_determinate_60s_ttl() {
        // Inserted 30s ago: within the 60s determinate TTL, outside the 10s
        // Unknown TTL. If Upcoming were treated as Unknown, this would re-probe
        // (spawning yt-dlp) and not return the cached value.
        let cache: crate::LiveStatusCache =
            std::sync::Arc::new(tokio::sync::RwLock::new(std::collections::HashMap::new()));
        let thirty_secs_ago = std::time::Instant::now()
            .checked_sub(std::time::Duration::from_secs(30))
            .expect("system uptime > 30s");
        cache.write().await.insert(
            "https://www.youtube.com/watch?v=up".to_string(),
            (LiveStatus::Upcoming(Some(1781287200)), thirty_secs_ago),
        );
        assert_eq!(
            cached_live_status(&cache, "https://www.youtube.com/watch?v=up").await,
            LiveStatus::Upcoming(Some(1781287200))
        );
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
    #[ignore = "requires yt-dlp and network"]
    async fn probe_live_real_vod_is_not_live() {
        // "Me at the zoo" — a regular upload that was never a live broadcast.
        // Pins that the new probe args (--print "%(live_status)s|%(release_timestamp)s"
        // --ignore-no-formats-error) produce parseable output end-to-end.
        // A VOD stands in for an upcoming stream (those URLs are ephemeral);
        // if yt-dlp exits non-zero for upcoming despite the flag, the stderr
        // fallback still yields Upcoming(None).
        let status = probe_live("https://www.youtube.com/watch?v=jNQXAC9IVRw").await;
        assert_eq!(status, LiveStatus::NotLive);
    }

    #[test]
    fn parse_status_and_url_two_lines() {
        let (url, status) = parse_status_and_url("was_live\nhttps://example.com/v.mp4\n").unwrap();
        assert_eq!(url, "https://example.com/v.mp4");
        assert_eq!(status, LiveStatus::WasLive);
    }

    #[test]
    fn parse_status_and_url_three_lines_takes_first_url() {
        let (url, status) =
            parse_status_and_url("is_live\nhttps://a.test/video\nhttps://a.test/audio\n").unwrap();
        assert_eq!(url, "https://a.test/video");
        assert_eq!(status, LiveStatus::Live);
    }

    #[test]
    fn parse_status_and_url_na_status_is_unknown() {
        let (url, status) = parse_status_and_url("NA\nhttps://a.test/v.m3u8\n").unwrap();
        assert_eq!(status, LiveStatus::Unknown);
        assert_eq!(url, "https://a.test/v.m3u8");
    }

    #[test]
    fn parse_status_and_url_missing_url_line_is_none() {
        assert_eq!(parse_status_and_url("was_live\n"), None);
        assert_eq!(parse_status_and_url(""), None);
    }

    #[test]
    fn test_recoverable_status_offline_and_upcoming() {
        use LiveStatus::*;
        assert_eq!(
            recoverable_status("ERROR: ... This live event is not currently live ..."),
            Some(Offline)
        );
        assert_eq!(
            recoverable_status("ERROR: ... this live event will begin in 2 hours ..."),
            Some(Upcoming(None))
        );
        // genuine failures do not become a recoverable status
        assert_eq!(recoverable_status("ERROR: HTTP Error 404: Not Found"), None);
        assert_eq!(recoverable_status(""), None);
    }

    #[test]
    fn classify_resolved_truth_table() {
        use LiveStatus::*;
        let playable = |u: &str| LiveResolution::Playable { url: u.to_string() };
        // a live/unknown/not-live stream with a URL is playable…
        assert_eq!(
            classify_resolved(Live, "https://x/v.m3u8"),
            playable("https://x/v.m3u8")
        );
        assert_eq!(
            classify_resolved(Unknown, "https://x/v.m3u8"),
            playable("https://x/v.m3u8")
        );
        assert_eq!(
            classify_resolved(NotLive, "https://x/v.m3u8"),
            playable("https://x/v.m3u8")
        );
        // …a finished broadcast (by status) is Ended…
        assert_eq!(
            classify_resolved(WasLive, "https://x/v.mp4"),
            LiveResolution::Ended
        );
        assert_eq!(
            classify_resolved(PostLive, "https://x/v.mp4"),
            LiveResolution::Ended
        );
        // …and so is one detected only by the force_finished manifest marker.
        assert_eq!(
            classify_resolved(Unknown, "https://x/a/force_finished/1/i.m3u8"),
            LiveResolution::Ended
        );
    }

    #[tokio::test]
    async fn resolve_live_passthrough_for_hls() {
        let res = resolve_live("https://example.com/stream.m3u8")
            .await
            .unwrap();
        assert_eq!(
            res,
            LiveResolution::Playable {
                url: "https://example.com/stream.m3u8".to_string()
            }
        );
    }

    #[tokio::test]
    async fn resolve_live_rejects_non_http_scheme() {
        let err = resolve_live("ftp://example.com/stream.m3u8")
            .await
            .unwrap_err();
        assert!(err.to_string().contains("invalid URL scheme"));
    }

    #[tokio::test]
    #[ignore = "requires yt-dlp and network"]
    async fn resolve_live_real_vod_is_playable() {
        // "Me at the zoo" — pins the two-line `--print live_status --print urls`
        // output shape and print ordering against real yt-dlp.
        match resolve_live("https://www.youtube.com/watch?v=jNQXAC9IVRw")
            .await
            .unwrap()
        {
            LiveResolution::Playable { url } => assert!(url.starts_with("http"), "got: {url}"),
            other => panic!("expected Playable, got {other:?}"),
        }
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
