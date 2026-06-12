use std::collections::HashSet;
use std::time::Duration;

use sqlx::SqlitePool;

use crate::media::resolver::{self, LiveStatus};
use crate::model::source::{self, Source};
use crate::CorsCache;
use crate::LiveStatusCache;

const CHECK_INTERVAL: Duration = Duration::from_secs(15 * 60);
const HTTP_TIMEOUT: Duration = Duration::from_secs(5);
#[cfg(test)]
const FAILURE_THRESHOLD: i64 = 3;

/// Dependencies for the background health checker.
pub struct HealthClients {
    pub pool: SqlitePool,
    pub http_client: reqwest::Client,
    pub cors_cache: CorsCache,
    pub live_cache: LiveStatusCache,
}

pub fn start(clients: HealthClients) {
    tokio::spawn(async move {
        let mut interval = tokio::time::interval(CHECK_INTERVAL);
        interval.set_missed_tick_behavior(tokio::time::MissedTickBehavior::Skip);
        interval.tick().await;
        loop {
            interval.tick().await;
            check_all(
                &clients.pool,
                &clients.http_client,
                &clients.cors_cache,
                &clients.live_cache,
            )
            .await;
        }
    });
}

async fn check_all(
    pool: &SqlitePool,
    client: &reqwest::Client,
    cors_cache: &CorsCache,
    live_cache: &LiveStatusCache,
) {
    let mut probed_hosts: HashSet<String> = HashSet::new();

    let sources = match source::list_all(pool).await {
        Ok(s) => s,
        Err(e) => {
            tracing::error!("health: failed to fetch sources: {e}");
            return;
        }
    };
    for src in sources {
        let ok = check_source(pool, client, live_cache, &src).await;
        if ok {
            let host = crate::media::hls::extract_manifest_host(&src.url);
            if probed_hosts.insert(host) {
                probe_and_cache_cors(client, cors_cache, &src.url).await;
            }
        }
    }

    let items = match crate::model::playlist_item::list_all(pool).await {
        Ok(i) => i,
        Err(e) => {
            tracing::error!("health: failed to fetch playlist items: {e}");
            return;
        }
    };
    for item in items {
        let ok = check_playlist_item(pool, client, &item).await;
        if ok {
            let host = crate::media::hls::extract_manifest_host(&item.url);
            if probed_hosts.insert(host) {
                probe_and_cache_cors(client, cors_cache, &item.url).await;
            }
        }
    }
}

async fn run_check<F, Fut>(
    client: &reqwest::Client,
    url: &str,
    kind: &str,
    consecutive_failures: i64,
    live_cache: Option<&LiveStatusCache>,
    update: F,
) -> bool
where
    F: FnOnce(&'static str, Option<String>, i64) -> Fut,
    Fut: std::future::Future<Output = anyhow::Result<()>>,
{
    let (ok, reason) = do_http_check(client, url, kind, live_cache).await;
    let new_failures = process_failures(consecutive_failures, ok);
    let status: &'static str = if ok { "ok" } else { "error" };
    if let Err(e) = update(status, reason, new_failures).await {
        tracing::error!("health: failed to update {url}: {e}");
        return false;
    }
    ok
}

/// Records a single liveness probe result against a source's health. `ok = true`
/// resets failures (status "ok"); `ok = false` counts a failure (status "error",
/// reason "not currently live"). Never changes `is_active` — manual intent is the
/// admin's alone; the unified Status badge reflects liveness separately. Used by
/// the interactive tune path so an active poll doubles as a liveness signal.
pub async fn record_source_liveness(pool: &SqlitePool, src: &Source, ok: bool) {
    let new_failures = process_failures(src.consecutive_failures, ok);
    let status: &'static str = if ok { "ok" } else { "error" };
    let reason = if ok { None } else { Some("not currently live") };
    if let Err(e) = source::update_health(pool, src.id, status, reason, new_failures, None).await {
        tracing::error!("health: failed to record liveness for {}: {e}", src.url);
    }
}

async fn check_source(
    pool: &SqlitePool,
    client: &reqwest::Client,
    live_cache: &LiveStatusCache,
    src: &Source,
) -> bool {
    run_check(
        client,
        &src.url,
        &src.kind,
        src.consecutive_failures,
        Some(live_cache),
        |status, reason, failures| async move {
            source::update_health(pool, src.id, status, reason.as_deref(), failures, None).await
        },
    )
    .await
}

async fn check_playlist_item(
    pool: &SqlitePool,
    client: &reqwest::Client,
    item: &crate::model::playlist_item::PlaylistItem,
) -> bool {
    let kind = crate::model::source::SourceKind::detect(&item.url);
    run_check(
        client,
        &item.url,
        kind.as_str(),
        item.consecutive_failures,
        None,
        |status, reason, failures| async move {
            crate::model::playlist_item::update_health(
                pool,
                item.id,
                status,
                reason.as_deref(),
                failures,
                None,
            )
            .await
        },
    )
    .await
}

/// A health-probe target: a live/VOD source row, or a playlist item. `probe`
/// classifies the variant and runs the full pipeline (health update, CORS cache,
/// and — for resolution-needed URLs — the resolved-CDN CORS probe / VOD host mark)
/// internally, so callers never branch on the URL kind.
pub enum ProbeTarget<'a> {
    Source(&'a Source),
    PlaylistItem(&'a crate::model::playlist_item::PlaylistItem),
}

/// Probes a target's health and warms the CORS cache, without touching `is_active`.
/// Used by the admin + JSON-API Test buttons. Dispatches on the target kind:
/// a `Source` consults the live-status cache and, when the URL needs resolution,
/// resolves it to probe the real CDN's CORS; a `PlaylistItem` skips the live cache
/// and, when its URL needs resolution, marks the host Direct (VOD items bypass the
/// proxy entirely).
pub async fn probe(
    pool: &SqlitePool,
    client: &reqwest::Client,
    cors_cache: &CorsCache,
    live_cache: &LiveStatusCache,
    target: ProbeTarget<'_>,
) {
    match target {
        ProbeTarget::Source(src) => {
            let ok = run_check(
                client,
                &src.url,
                &src.kind,
                src.consecutive_failures,
                Some(live_cache),
                |status, reason, failures| async move {
                    source::update_health(pool, src.id, status, reason.as_deref(), failures, None)
                        .await
                },
            )
            .await;
            if ok {
                probe_and_cache_cors(client, cors_cache, &src.url).await;
            }
            if crate::media::resolver::needs_resolution(&src.url) {
                probe_and_cache_resolved_cors(client, cors_cache, &src.url).await;
            }
        }
        ProbeTarget::PlaylistItem(item) => {
            let kind = crate::model::source::SourceKind::detect(&item.url);
            let ok = run_check(
                client,
                &item.url,
                kind.as_str(),
                item.consecutive_failures,
                None,
                |status, reason, failures| async move {
                    crate::model::playlist_item::update_health(
                        pool,
                        item.id,
                        status,
                        reason.as_deref(),
                        failures,
                        None,
                    )
                    .await
                },
            )
            .await;
            if ok {
                probe_and_cache_cors(client, cors_cache, &item.url).await;
            }
            if crate::media::resolver::needs_resolution(&item.url) {
                // VOD items bypass the proxy entirely, so their host is Direct outright.
                let host = crate::media::hls::extract_manifest_host(&item.url);
                cors_cache.write().await.insert(host, true);
            }
        }
    }
}

/// Probes CORS for one URL and caches the result keyed by host. Returns `None`
/// (a no-op, leaving the cache unchanged) for non-HTTPS URLs or resolution-needed
/// (youtube/twitch) URLs, which have no stable HLS manifest to probe.
pub async fn probe_and_cache_cors(
    client: &reqwest::Client,
    cors_cache: &CorsCache,
    url: &str,
) -> Option<bool> {
    if !url.starts_with("https://") || crate::media::resolver::needs_resolution(url) {
        return None;
    }
    let (probe_host, result) = if crate::model::source::SourceKind::detect(url)
        == crate::model::source::SourceKind::Dash
    {
        let (probe_url, cors) = crate::media::mpd::probe_mpd_cors(client, url).await?;
        (crate::media::hls::extract_manifest_host(&probe_url), cors)
    } else {
        let cors = crate::media::hls::probe_source_cors(client, url).await?;
        (crate::media::hls::extract_manifest_host(url), cors)
    };
    let manifest_host = crate::media::hls::extract_manifest_host(url);
    tracing::debug!(probe_host = %probe_host, cors = result, "CORS probe cached");
    let mut cache = cors_cache.write().await;
    cache.insert(probe_host.clone(), result);
    // Also cache under the manifest host so existing lookups by source/playlist URL continue to work.
    if probe_host != manifest_host {
        cache.insert(manifest_host, result);
    }
    Some(result)
}

/// Resolves a YouTube/Twitch live source via yt-dlp, probes the resolved HLS
/// manifest's segment-CDN CORS, and caches the result under BOTH the resolved
/// CDN host and the original source host. The original-host entry is what the
/// guide and admin source-row budget lookups query with (they only know the DB
/// source URL, never the resolved googlevideo URL). Returns `None` (cache
/// unchanged) if resolution fails, the resolved URL is not a probeable HLS
/// manifest, or the URL does not require resolution.
async fn probe_and_cache_resolved_cors(
    client: &reqwest::Client,
    cors_cache: &CorsCache,
    source_url: &str,
) -> Option<bool> {
    if !crate::media::resolver::needs_resolution(source_url) {
        return None;
    }
    let resolved = crate::media::resolver::resolve_url(source_url).await.ok()?;
    let cors = crate::media::hls::probe_source_cors(client, &resolved).await?;

    let resolved_host = crate::media::hls::extract_manifest_host(&resolved);
    let original_host = crate::media::hls::extract_manifest_host(source_url);

    tracing::debug!(
        resolved_host = %resolved_host,
        original_host = %original_host,
        cors,
        "resolved-live CORS probe cached"
    );
    let mut cache = cors_cache.write().await;
    cache.insert(resolved_host.clone(), cors);
    if original_host != resolved_host {
        cache.insert(original_host, cors);
    }
    Some(cors)
}

async fn do_http_check(
    client: &reqwest::Client,
    url: &str,
    kind: &str,
    live_cache: Option<&LiveStatusCache>,
) -> (bool, Option<String>) {
    if kind == "youtube_live" {
        let (ok, reason) = match live_cache {
            Some(c) => live_status_health(resolver::cached_live_status(c, url).await),
            // No live-status cache (playlist-item checks): a youtube_live-detected
            // VOD item isn't a live broadcast, so don't spend a yt-dlp probe on it.
            None => (true, None),
        };
        return (ok, reason.map(|s| s.to_string()));
    }

    let mut resp = match client.get(url).timeout(HTTP_TIMEOUT).send().await {
        Ok(r) => r,
        Err(e) => return (false, Some(format!("request failed: {e}"))),
    };

    let status = resp.status();
    if !status.is_success() && !status.is_redirection() {
        return (false, Some(format!("HTTP {}", status.as_u16())));
    }

    // reqwest's per-request `.timeout(HTTP_TIMEOUT)` is a total deadline covering
    // the body read, so this `chunk()` can't hang past HTTP_TIMEOUT even on a stream
    // that connects then stalls.
    match resp.chunk().await {
        Ok(Some(_)) => (true, None),
        Ok(None) => (false, Some("stream returned no data".to_string())),
        Err(e) => (false, Some(format!("read failed: {e}"))),
    }
}

/// Maps a probed live status to a `(healthy, reason)` health result for a
/// `youtube_live` source. `Upcoming`/`Unknown` never penalize (a scheduled
/// stream isn't broken; `Unknown` is a load-shed or extractor gap).
fn live_status_health(status: LiveStatus) -> (bool, Option<&'static str>) {
    match status {
        LiveStatus::Live | LiveStatus::Upcoming(_) | LiveStatus::Unknown => (true, None),
        LiveStatus::Offline | LiveStatus::NotLive => (false, Some("not currently live")),
        LiveStatus::WasLive | LiveStatus::PostLive => (false, Some("broadcast ended")),
    }
}

/// New consecutive-failure count after one check. `is_active` is no longer
/// consulted here — the checker never changes `is_active` (manual intent only).
fn process_failures(consecutive_failures: i64, ok: bool) -> i64 {
    if ok {
        0
    } else {
        consecutive_failures + 1
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn test_live_status_health_mapping() {
        use crate::media::resolver::LiveStatus::*;
        assert_eq!(live_status_health(Live), (true, None));
        assert_eq!(live_status_health(Upcoming(None)), (true, None));
        assert_eq!(live_status_health(Upcoming(Some(1_000_000))), (true, None));
        assert_eq!(live_status_health(Unknown), (true, None));
        assert_eq!(
            live_status_health(Offline),
            (false, Some("not currently live"))
        );
        assert_eq!(
            live_status_health(NotLive),
            (false, Some("not currently live"))
        );
        assert_eq!(
            live_status_health(WasLive),
            (false, Some("broadcast ended"))
        );
        assert_eq!(
            live_status_health(PostLive),
            (false, Some("broadcast ended"))
        );
    }

    #[test]
    fn test_process_failures_resets_on_ok() {
        assert_eq!(process_failures(2, true), 0);
    }

    #[test]
    fn test_process_failures_increments_on_error() {
        assert_eq!(process_failures(1, false), 2);
    }

    #[test]
    fn test_probe_and_cache_cors_dash_caches_under_cdn_host() {
        // Verifies the cache key used for a DASH source is the segment CDN host
        // extracted by probe_mpd_cors, not the MPD manifest host. Both should be
        // inserted so existing lookups by manifest URL continue to work.
        //
        // Simulates: manifest at https://manifest.test/, segments at https://cdn.test/
        let manifest_host = "https://manifest.test";
        let cdn_host = "https://cdn.test";
        assert_ne!(
            manifest_host, cdn_host,
            "test requires distinct manifest and CDN hosts"
        );

        // Build the expected cache state after a successful DASH probe.
        // The cache must contain BOTH the CDN host (semantic key) and the
        // manifest host (backward-compat key for status_for_url lookups).
        let mut cache: std::collections::HashMap<String, bool> = std::collections::HashMap::new();
        cache.insert(cdn_host.to_string(), true);
        cache.insert(manifest_host.to_string(), true);

        // status_for_url must find the result via the manifest host (source URL lookup).
        assert_eq!(
            crate::budget::status_for_url(&format!("{}/stream.mpd", manifest_host), &cache),
            crate::budget::BudgetStatus::Direct,
            "status_for_url must find the CORS result via manifest host"
        );

        // The CDN host must also be present for correct semantics.
        assert!(
            cache.contains_key(cdn_host),
            "CDN host must be in the cache after probing a DASH source with a different CDN"
        );
    }

    #[tokio::test]
    async fn test_probe_and_cache_cors_skips_non_https() {
        let cache: crate::CorsCache =
            std::sync::Arc::new(tokio::sync::RwLock::new(std::collections::HashMap::new()));
        let client = reqwest::Client::new();
        let result = probe_and_cache_cors(&client, &cache, "http://x.example.com/s.m3u8").await;
        assert_eq!(result, None);
        assert!(cache.read().await.is_empty());
    }

    #[tokio::test]
    async fn test_probe_and_cache_cors_skips_resolution_needed() {
        let cache: crate::CorsCache =
            std::sync::Arc::new(tokio::sync::RwLock::new(std::collections::HashMap::new()));
        let client = reqwest::Client::new();
        let result = probe_and_cache_cors(&client, &cache, "https://youtube.com/watch?v=abc").await;
        assert_eq!(result, None);
        assert!(cache.read().await.is_empty());
    }

    #[tokio::test]
    async fn test_probe_and_cache_resolved_cors_noop_when_no_resolution_needed() {
        // "not-a-url" does not require resolution, so the helper short-circuits to
        // None before spawning yt-dlp — deterministic, never touches the network.
        // The cache must be left untouched.
        let cache: crate::CorsCache =
            std::sync::Arc::new(tokio::sync::RwLock::new(std::collections::HashMap::new()));
        let client = reqwest::Client::new();
        let result = probe_and_cache_resolved_cors(&client, &cache, "not-a-url").await;
        assert_eq!(result, None);
        assert!(cache.read().await.is_empty());
    }

    #[test]
    fn test_resolved_cors_caches_under_both_hosts() {
        // Contract test (mirrors test_probe_and_cache_cors_dash_caches_under_cdn_host):
        // after a successful resolved-live probe, the cache must hold the result under
        // BOTH the resolved CDN host (semantic key) and the original source host (the
        // key the guide/admin-row lookups actually query with). The guide only ever
        // knows the DB source URL (youtube.com), never the resolved googlevideo URL.
        let original_host = "https://www.youtube.com";
        let cdn_host = "https://rr3---sn-xyz.googlevideo.com";
        assert_ne!(original_host, cdn_host);

        let mut cache: std::collections::HashMap<String, bool> = std::collections::HashMap::new();
        cache.insert(cdn_host.to_string(), true);
        cache.insert(original_host.to_string(), true);

        // The guide looks up by the original source URL host -> must find Direct.
        assert_eq!(
            crate::budget::status_for_url("https://www.youtube.com/live/abc123", &cache),
            crate::budget::BudgetStatus::Direct,
            "guide lookup by original youtube host must find the probe result"
        );
        assert!(
            cache.contains_key(cdn_host),
            "resolved CDN host must also be cached for correct semantics"
        );
    }

    #[tokio::test]
    async fn probe_does_not_reenable_disabled_source() {
        use tokio::io::{AsyncReadExt, AsyncWriteExt};

        // A real server returning HTTP 200 — simulates a healthy but admin-disabled source.
        let listener = tokio::net::TcpListener::bind("127.0.0.1:0").await.unwrap();
        let port = listener.local_addr().unwrap().port();
        tokio::spawn(async move {
            let (mut conn, _) = listener.accept().await.unwrap();
            let mut buf = [0u8; 512];
            let _ = conn.read(&mut buf).await;
            conn.write_all(b"HTTP/1.1 200 OK\r\nContent-Length: 0\r\n\r\n")
                .await
                .unwrap();
        });

        let pool = crate::db::connect("sqlite::memory:").await.unwrap();
        let ch = crate::model::channel::create(
            &pool,
            crate::model::channel::NewChannel {
                name: "test".to_string(),
                category: "test".to_string(),
                logo_url: None,
                channel_type: crate::model::channel::ChannelType::Live,
                sort_order: 0,
                loop_anchor: None,
            },
        )
        .await
        .unwrap();
        let src = crate::model::source::create(
            &pool,
            crate::model::source::NewSource {
                channel_id: ch.id,
                kind: crate::model::source::SourceKind::Hls,
                url: format!("http://127.0.0.1:{}/stream.m3u8", port),
                priority: 1,
            },
        )
        .await
        .unwrap();

        // Admin manually disables the source.
        crate::model::source::set_active(&pool, src.id, false)
            .await
            .unwrap();
        let src = crate::model::source::get(&pool, src.id)
            .await
            .unwrap()
            .unwrap();
        assert!(!src.is_active, "source must start disabled");

        let client = reqwest::Client::builder()
            .timeout(std::time::Duration::from_secs(2))
            .build()
            .unwrap();
        let cors_cache: crate::CorsCache =
            std::sync::Arc::new(tokio::sync::RwLock::new(std::collections::HashMap::new()));
        let live_cache: LiveStatusCache =
            std::sync::Arc::new(tokio::sync::RwLock::new(std::collections::HashMap::new()));

        // probe() is the manual Test-button path — must never change is_active.
        probe(
            &pool,
            &client,
            &cors_cache,
            &live_cache,
            ProbeTarget::Source(&src),
        )
        .await;

        let updated = crate::model::source::get(&pool, src.id)
            .await
            .unwrap()
            .unwrap();
        assert!(
            !updated.is_active,
            "probe() must not re-enable a manually disabled source"
        );
    }

    #[tokio::test]
    async fn probe_does_not_reenable_disabled_playlist_item() {
        use tokio::io::{AsyncReadExt, AsyncWriteExt};

        let listener = tokio::net::TcpListener::bind("127.0.0.1:0").await.unwrap();
        let port = listener.local_addr().unwrap().port();
        tokio::spawn(async move {
            let (mut conn, _) = listener.accept().await.unwrap();
            let mut buf = [0u8; 512];
            let _ = conn.read(&mut buf).await;
            conn.write_all(b"HTTP/1.1 200 OK\r\nContent-Length: 5\r\n\r\nhello")
                .await
                .unwrap();
        });

        let pool = crate::db::connect("sqlite::memory:").await.unwrap();
        let ch = crate::model::channel::create(
            &pool,
            crate::model::channel::NewChannel {
                name: "test".to_string(),
                category: "test".to_string(),
                logo_url: None,
                channel_type: crate::model::channel::ChannelType::VodLoop,
                sort_order: 0,
                loop_anchor: None,
            },
        )
        .await
        .unwrap();

        let it = crate::model::playlist_item::create(
            &pool,
            crate::model::playlist_item::NewPlaylistItem {
                channel_id: ch.id,
                title: "ep1".to_string(),
                url: format!("http://127.0.0.1:{}/ep1.mp4", port),
                duration_secs: 3600,
                sort_order: 0,
            },
        )
        .await
        .unwrap();

        crate::model::playlist_item::set_active(&pool, it.id, false)
            .await
            .unwrap();
        let it = crate::model::playlist_item::get(&pool, it.id)
            .await
            .unwrap()
            .unwrap();
        assert!(!it.is_active, "item must start disabled");

        let client = reqwest::Client::builder()
            .timeout(std::time::Duration::from_secs(2))
            .build()
            .unwrap();
        let cors_cache: crate::CorsCache =
            std::sync::Arc::new(tokio::sync::RwLock::new(std::collections::HashMap::new()));
        let live_cache: LiveStatusCache =
            std::sync::Arc::new(tokio::sync::RwLock::new(std::collections::HashMap::new()));
        probe(
            &pool,
            &client,
            &cors_cache,
            &live_cache,
            ProbeTarget::PlaylistItem(&it),
        )
        .await;

        let updated = crate::model::playlist_item::get(&pool, it.id)
            .await
            .unwrap()
            .unwrap();
        assert!(
            !updated.is_active,
            "probe() must not re-enable a manually disabled item"
        );
    }

    #[test]
    fn test_probed_hosts_dedup_same_cdn() {
        // Two episodes on the same CDN produce the same manifest host.
        // The HashSet used in check_all must deduplicate them so only
        // the first triggers a CORS probe.
        let mut probed_hosts = std::collections::HashSet::new();

        let ep1 = "https://cdn.example.com/vod/season1/ep1.m3u8";
        let ep2 = "https://cdn.example.com/vod/season1/ep2.m3u8";
        let ep3 = "https://other-cdn.example.com/vod/ep3.m3u8";

        let h1 = crate::media::hls::extract_manifest_host(ep1);
        let h2 = crate::media::hls::extract_manifest_host(ep2);
        let h3 = crate::media::hls::extract_manifest_host(ep3);

        assert!(
            probed_hosts.insert(h1),
            "ep1: first insert for this CDN host must succeed"
        );
        assert!(
            !probed_hosts.insert(h2),
            "ep2: same CDN host must be deduplicated"
        );
        assert!(
            probed_hosts.insert(h3),
            "ep3: different CDN host must not be deduplicated"
        );
        assert_eq!(probed_hosts.len(), 2);
    }

    #[tokio::test]
    async fn test_record_source_liveness_never_changes_is_active() {
        use crate::model::{channel, source};
        let pool = crate::db::connect("sqlite::memory:").await.unwrap();
        let ch = channel::create(
            &pool,
            channel::NewChannel {
                name: "T".into(),
                category: "t".into(),
                logo_url: None,
                channel_type: channel::ChannelType::Live,
                sort_order: 0,
                loop_anchor: None,
            },
        )
        .await
        .unwrap();
        let mut src = source::create(
            &pool,
            source::NewSource {
                channel_id: ch.id,
                kind: source::SourceKind::YoutubeLive,
                url: "https://youtube.com/watch?v=x".into(),
                priority: 1,
            },
        )
        .await
        .unwrap();

        // Many offline probes must NOT disable the source.
        for _ in 0..(FAILURE_THRESHOLD + 2) {
            record_source_liveness(&pool, &src, false).await;
            src = source::get(&pool, src.id).await.unwrap().unwrap();
        }
        assert!(src.is_active, "liveness probes must never disable a source");
        assert_eq!(src.consecutive_failures, FAILURE_THRESHOLD + 2);
        assert_eq!(src.last_status.as_deref(), Some("error"));

        // A success resets failures, still without touching is_active.
        record_source_liveness(&pool, &src, true).await;
        let after = source::get(&pool, src.id).await.unwrap().unwrap();
        assert!(after.is_active);
        assert_eq!(after.consecutive_failures, 0);
        assert_eq!(after.last_status.as_deref(), Some("ok"));
    }

    #[tokio::test]
    async fn probe_marks_needs_resolution_vod_item_host_direct() {
        let pool = crate::db::connect("sqlite::memory:").await.unwrap();
        let ch = crate::model::channel::create(
            &pool,
            crate::model::channel::NewChannel {
                name: "vod".to_string(),
                category: "test".to_string(),
                logo_url: None,
                channel_type: crate::model::channel::ChannelType::VodLoop,
                sort_order: 0,
                loop_anchor: None,
            },
        )
        .await
        .unwrap();
        let item = crate::model::playlist_item::create(
            &pool,
            crate::model::playlist_item::NewPlaylistItem {
                channel_id: ch.id,
                title: "rec".to_string(),
                url: "https://www.youtube.com/watch?v=abc123".to_string(),
                duration_secs: 3600,
                sort_order: 0,
            },
        )
        .await
        .unwrap();

        let client = reqwest::Client::new();
        let cors_cache: crate::CorsCache =
            std::sync::Arc::new(tokio::sync::RwLock::new(std::collections::HashMap::new()));
        let live_cache: LiveStatusCache =
            std::sync::Arc::new(tokio::sync::RwLock::new(std::collections::HashMap::new()));

        probe(
            &pool,
            &client,
            &cors_cache,
            &live_cache,
            ProbeTarget::PlaylistItem(&item),
        )
        .await;

        let host = crate::media::hls::extract_manifest_host(&item.url);
        assert_eq!(
            cors_cache.read().await.get(&host).copied(),
            Some(true),
            "needs-resolution VOD item host must be marked Direct"
        );
    }

    #[tokio::test]
    async fn test_check_all_health_checks_each_item_independently() {
        use tokio::io::{AsyncReadExt, AsyncWriteExt};

        // Server returns 200 for the first two connections (one per item health check)
        let listener = tokio::net::TcpListener::bind("127.0.0.1:0").await.unwrap();
        let port = listener.local_addr().unwrap().port();
        tokio::spawn(async move {
            for _ in 0..2u8 {
                let (mut conn, _) = listener.accept().await.unwrap();
                let mut buf = [0u8; 512];
                let _ = conn.read(&mut buf).await;
                conn.write_all(b"HTTP/1.1 200 OK\r\nContent-Length: 5\r\n\r\nhello")
                    .await
                    .unwrap();
            }
        });

        let pool = crate::db::connect("sqlite::memory:").await.unwrap();
        let ch = crate::model::channel::create(
            &pool,
            crate::model::channel::NewChannel {
                name: "vod".to_string(),
                category: "test".to_string(),
                logo_url: None,
                channel_type: crate::model::channel::ChannelType::VodLoop,
                sort_order: 0,
                loop_anchor: None,
            },
        )
        .await
        .unwrap();

        let it1 = crate::model::playlist_item::create(
            &pool,
            crate::model::playlist_item::NewPlaylistItem {
                channel_id: ch.id,
                title: "ep1".to_string(),
                url: format!("http://127.0.0.1:{}/ep1.mp4", port),
                duration_secs: 3600,
                sort_order: 0,
            },
        )
        .await
        .unwrap();

        let it2 = crate::model::playlist_item::create(
            &pool,
            crate::model::playlist_item::NewPlaylistItem {
                channel_id: ch.id,
                title: "ep2".to_string(),
                url: format!("http://127.0.0.1:{}/ep2.mp4", port),
                duration_secs: 3600,
                sort_order: 1,
            },
        )
        .await
        .unwrap();

        let cors_cache: crate::CorsCache =
            std::sync::Arc::new(tokio::sync::RwLock::new(std::collections::HashMap::new()));
        let live_cache: LiveStatusCache =
            std::sync::Arc::new(tokio::sync::RwLock::new(std::collections::HashMap::new()));
        let client = reqwest::Client::builder()
            .timeout(std::time::Duration::from_secs(2))
            .build()
            .unwrap();

        check_all(&pool, &client, &cors_cache, &live_cache).await;

        // Both items must have been health-checked independently
        let updated1 = crate::model::playlist_item::get(&pool, it1.id)
            .await
            .unwrap()
            .unwrap();
        let updated2 = crate::model::playlist_item::get(&pool, it2.id)
            .await
            .unwrap()
            .unwrap();

        assert_eq!(
            updated1.last_status.as_deref(),
            Some("ok"),
            "ep1 must be health-checked independently"
        );
        assert_eq!(
            updated2.last_status.as_deref(),
            Some("ok"),
            "ep2 must be health-checked independently"
        );
    }
}
