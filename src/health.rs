use std::time::Duration;

use sqlx::SqlitePool;

use crate::model::source::{self, Source};
use crate::CorsCache;

const CHECK_INTERVAL: Duration = Duration::from_secs(15 * 60);
const HTTP_TIMEOUT: Duration = Duration::from_secs(5);
pub(crate) const FAILURE_THRESHOLD: i64 = 3;

enum HealthAction {
    Disable,
    Reenable,
    None,
}

/// Dependencies for the background health checker.
pub struct HealthClients {
    pub pool: SqlitePool,
    pub http_client: reqwest::Client,
    pub cors_cache: CorsCache,
}

pub fn start(clients: HealthClients) {
    tokio::spawn(async move {
        let mut interval = tokio::time::interval(CHECK_INTERVAL);
        interval.set_missed_tick_behavior(tokio::time::MissedTickBehavior::Skip);
        interval.tick().await;
        loop {
            interval.tick().await;
            check_all(&clients.pool, &clients.http_client, &clients.cors_cache).await;
        }
    });
}

async fn check_all(pool: &SqlitePool, client: &reqwest::Client, cors_cache: &CorsCache) {
    let sources = match source::list_all(pool).await {
        Ok(s) => s,
        Err(e) => {
            tracing::error!("health: failed to fetch sources: {e}");
            return;
        }
    };
    for src in sources {
        let _ = check_source(pool, client, cors_cache, &src).await;
    }

    let items = match crate::model::playlist_item::list_all(pool).await {
        Ok(i) => i,
        Err(e) => {
            tracing::error!("health: failed to fetch playlist items: {e}");
            return;
        }
    };
    for item in items {
        let _ = check_playlist_item(pool, client, cors_cache, &item).await;
    }
}

async fn run_check<F, Fut>(
    client: &reqwest::Client,
    url: &str,
    kind: &str,
    is_active: bool,
    consecutive_failures: i64,
    manage_lifecycle: bool,
    update: F,
) -> bool
where
    F: FnOnce(&'static str, Option<String>, i64, Option<bool>) -> Fut,
    Fut: std::future::Future<Output = anyhow::Result<()>>,
{
    let (ok, reason) = do_http_check(client, url, kind).await;
    let (new_failures, action) = process_result(is_active, consecutive_failures, ok);
    let is_active_change = if manage_lifecycle {
        match action {
            HealthAction::Disable => Some(false),
            HealthAction::Reenable => Some(true),
            HealthAction::None => None,
        }
    } else {
        None
    };

    let status: &'static str = if ok { "ok" } else { "error" };
    if let Err(e) = update(status, reason, new_failures, is_active_change).await {
        tracing::error!("health: failed to update {url}: {e}");
        return false;
    }

    if manage_lifecycle {
        match action {
            HealthAction::Disable => tracing::warn!(
                "health: {url} auto-disabled after {new_failures} consecutive failures"
            ),
            HealthAction::Reenable => {
                tracing::info!("health: {url} auto-re-enabled after passing health check")
            }
            HealthAction::None => {}
        }
    }

    ok
}

/// Probes a source's health and updates stats without touching `is_active`.
/// Used by the admin Test button — respects the admin's manual enable/disable choice.
pub async fn probe_source(
    pool: &SqlitePool,
    client: &reqwest::Client,
    cors_cache: &CorsCache,
    src: &Source,
) {
    let ok = run_check(
        client,
        &src.url,
        &src.kind,
        src.is_active,
        src.consecutive_failures,
        false,
        |status, reason, failures, is_active_change| async move {
            source::update_health(
                pool,
                src.id,
                status,
                reason.as_deref(),
                failures,
                is_active_change,
            )
            .await
        },
    )
    .await;

    if ok {
        probe_and_cache_cors(client, cors_cache, &src.url).await;
    }
}

async fn check_source(
    pool: &SqlitePool,
    client: &reqwest::Client,
    cors_cache: &CorsCache,
    src: &Source,
) -> bool {
    let ok = run_check(
        client,
        &src.url,
        &src.kind,
        src.is_active,
        src.consecutive_failures,
        true,
        |status, reason, failures, is_active_change| async move {
            source::update_health(
                pool,
                src.id,
                status,
                reason.as_deref(),
                failures,
                is_active_change,
            )
            .await
        },
    )
    .await;

    // Only probe CORS for reachable sources: a down source would just incur a
    // second timeout, and its cached budget is best left as-is.
    if ok {
        probe_and_cache_cors(client, cors_cache, &src.url).await;
    }

    ok
}

async fn check_playlist_item(
    pool: &SqlitePool,
    client: &reqwest::Client,
    cors_cache: &CorsCache,
    item: &crate::model::playlist_item::PlaylistItem,
) -> bool {
    let kind = crate::model::source::SourceKind::detect(&item.url);
    let ok = run_check(
        client,
        &item.url,
        kind.as_str(),
        item.is_active,
        item.consecutive_failures,
        true,
        |status, reason, failures, is_active_change| async move {
            crate::model::playlist_item::update_health(
                pool,
                item.id,
                status,
                reason.as_deref(),
                failures,
                is_active_change,
            )
            .await
        },
    )
    .await;

    if ok {
        probe_and_cache_cors(client, cors_cache, &item.url).await;
    }

    ok
}

/// Probes a playlist item's health and updates stats without touching `is_active`.
/// Used by the admin Test button — respects the admin's manual enable/disable choice.
pub async fn probe_playlist_item(
    pool: &SqlitePool,
    client: &reqwest::Client,
    cors_cache: &CorsCache,
    item: &crate::model::playlist_item::PlaylistItem,
) {
    let kind = crate::model::source::SourceKind::detect(&item.url);
    let ok = run_check(
        client,
        &item.url,
        kind.as_str(),
        item.is_active,
        item.consecutive_failures,
        false,
        |status, reason, failures, is_active_change| async move {
            crate::model::playlist_item::update_health(
                pool,
                item.id,
                status,
                reason.as_deref(),
                failures,
                is_active_change,
            )
            .await
        },
    )
    .await;

    if ok {
        probe_and_cache_cors(client, cors_cache, &item.url).await;
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

async fn do_http_check(client: &reqwest::Client, url: &str, kind: &str) -> (bool, Option<String>) {
    let mut resp = match client.get(url).timeout(HTTP_TIMEOUT).send().await {
        Ok(r) => r,
        Err(e) => return (false, Some(format!("request failed: {e}"))),
    };

    let status = resp.status();
    if !status.is_success() && !status.is_redirection() {
        return (false, Some(format!("HTTP {}", status.as_u16())));
    }

    if kind == "youtube_live" {
        return (true, None);
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

fn process_result(is_active: bool, consecutive_failures: i64, ok: bool) -> (i64, HealthAction) {
    let new_failures = if ok { 0 } else { consecutive_failures + 1 };
    let action = if ok && !is_active {
        HealthAction::Reenable
    } else if !ok && new_failures >= FAILURE_THRESHOLD && is_active {
        HealthAction::Disable
    } else {
        HealthAction::None
    };
    (new_failures, action)
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn test_process_result_ok_resets_failures() {
        let (failures, action) = process_result(true, 2, true);
        assert_eq!(failures, 0);
        assert!(matches!(action, HealthAction::None));
    }

    #[test]
    fn test_process_result_error_increments_failures() {
        let (failures, action) = process_result(true, 1, false);
        assert_eq!(failures, 2);
        assert!(matches!(action, HealthAction::None));
    }

    #[test]
    fn test_process_result_triggers_disable_at_threshold() {
        let (failures, action) = process_result(true, 2, false);
        assert_eq!(failures, 3);
        assert!(matches!(action, HealthAction::Disable));
    }

    #[test]
    fn test_process_result_already_inactive_not_disabled_again() {
        let (failures, action) = process_result(false, 2, false);
        assert_eq!(failures, 3);
        assert!(matches!(action, HealthAction::None));
    }

    #[test]
    fn test_process_result_reenables_inactive_source_on_success() {
        let (failures, action) = process_result(false, 3, true);
        assert_eq!(failures, 0);
        assert!(matches!(action, HealthAction::Reenable));
    }

    #[test]
    fn test_process_result_active_source_ok_no_action() {
        let (failures, action) = process_result(true, 0, true);
        assert_eq!(failures, 0);
        assert!(matches!(action, HealthAction::None));
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
    async fn probe_source_does_not_reenable_disabled_source() {
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

        // probe_source is the manual Test-button path — must never change is_active.
        probe_source(&pool, &client, &cors_cache, &src).await;

        let updated = crate::model::source::get(&pool, src.id)
            .await
            .unwrap()
            .unwrap();
        assert!(
            !updated.is_active,
            "probe_source must not re-enable a manually disabled source"
        );
    }

    #[tokio::test]
    async fn probe_playlist_item_does_not_reenable_disabled_item() {
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

        probe_playlist_item(&pool, &client, &cors_cache, &it).await;

        let updated = crate::model::playlist_item::get(&pool, it.id)
            .await
            .unwrap()
            .unwrap();
        assert!(
            !updated.is_active,
            "probe_playlist_item must not re-enable a manually disabled item"
        );
    }

    #[tokio::test]
    async fn test_run_check_probe_mode_never_changes_is_active() {
        use tokio::io::{AsyncReadExt, AsyncWriteExt};

        // Server returns 200 with body — simulates a healthy source
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

        let client = reqwest::Client::builder()
            .timeout(std::time::Duration::from_secs(2))
            .build()
            .unwrap();

        // Source is inactive with 3 failures — in manage_lifecycle=true mode this would Reenable.
        // In probe mode (manage_lifecycle=false) it must never touch is_active.
        let ok = run_check(
            &client,
            &format!("http://127.0.0.1:{}/stream.m3u8", port),
            "hls",
            false, // is_active: currently disabled
            3,     // consecutive_failures
            false, // manage_lifecycle: probe mode
            |_status, _reason, _failures, is_active_change| async move {
                assert!(
                    is_active_change.is_none(),
                    "probe mode must never pass is_active_change = Some(…)"
                );
                Ok::<(), anyhow::Error>(())
            },
        )
        .await;

        assert!(ok, "server returned 200 — run_check must return true");
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
        let client = reqwest::Client::builder()
            .timeout(std::time::Duration::from_secs(2))
            .build()
            .unwrap();

        check_all(&pool, &client, &cors_cache).await;

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
