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
        check_source(pool, client, cors_cache, &src).await;
    }
    probe_all_playlist_cors(pool, client, cors_cache).await;
}

async fn probe_all_playlist_cors(
    pool: &SqlitePool,
    client: &reqwest::Client,
    cors_cache: &CorsCache,
) {
    let items = match crate::model::playlist_item::list_all(pool).await {
        Ok(i) => i,
        Err(e) => {
            tracing::error!("health: failed to fetch playlist items: {e}");
            return;
        }
    };
    // Dedupe by host so each CDN is probed at most once per cycle.
    let mut probed_hosts = std::collections::HashSet::new();
    for item in items {
        if !item.url.starts_with("https://") || crate::media::resolver::needs_resolution(&item.url)
        {
            continue;
        }
        let host = crate::media::hls::extract_manifest_host(&item.url);
        if !probed_hosts.insert(host) {
            continue;
        }
        probe_and_cache_cors(client, cors_cache, &item.url).await;
    }
}

/// Probes a source's health and updates stats without touching `is_active`.
/// Used by the admin Test button — respects the admin's manual enable/disable choice.
pub async fn probe_source(
    pool: &SqlitePool,
    client: &reqwest::Client,
    cors_cache: &CorsCache,
    src: &Source,
) {
    let (ok, reason) = do_http_check(client, src).await;
    let new_failures = if ok { 0 } else { src.consecutive_failures + 1 };

    if let Err(e) = source::update_health(
        pool,
        src.id,
        if ok { "ok" } else { "error" },
        reason.as_deref(),
        new_failures,
        None, // never change is_active
    )
    .await
    {
        tracing::error!("health: failed to update source {}: {e}", src.id);
        return;
    }

    if ok {
        probe_and_cache_cors(client, cors_cache, &src.url).await;
    }
}

pub async fn check_source(
    pool: &SqlitePool,
    client: &reqwest::Client,
    cors_cache: &CorsCache,
    src: &Source,
) {
    let (ok, reason) = do_http_check(client, src).await;
    let (new_failures, action) = process_result(src, ok);

    let is_active = match action {
        HealthAction::Disable => Some(false),
        HealthAction::Reenable => Some(true),
        HealthAction::None => None,
    };

    if let Err(e) = source::update_health(
        pool,
        src.id,
        if ok { "ok" } else { "error" },
        reason.as_deref(),
        new_failures,
        is_active,
    )
    .await
    {
        tracing::error!("health: failed to update source {}: {e}", src.id);
        return;
    }

    match action {
        HealthAction::Disable => tracing::warn!(
            "health: source {} auto-disabled after {} consecutive failures",
            src.id,
            new_failures
        ),
        HealthAction::Reenable => tracing::info!(
            "health: source {} auto-re-enabled after passing health check",
            src.id
        ),
        HealthAction::None => {}
    }

    // Only probe CORS for reachable sources: a down source would just incur a
    // second timeout, and its cached budget is best left as-is.
    if ok {
        probe_and_cache_cors(client, cors_cache, &src.url).await;
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
    let result = crate::media::hls::probe_source_cors(client, url).await?;
    let host = crate::media::hls::extract_manifest_host(url);
    tracing::debug!(host = %host, cors = result, "CORS probe cached");
    cors_cache.write().await.insert(host, result);
    Some(result)
}

async fn do_http_check(client: &reqwest::Client, src: &Source) -> (bool, Option<String>) {
    let mut resp = match client.get(&src.url).timeout(HTTP_TIMEOUT).send().await {
        Ok(r) => r,
        Err(e) => return (false, Some(format!("request failed: {e}"))),
    };

    let status = resp.status();
    if !status.is_success() && !status.is_redirection() {
        return (false, Some(format!("HTTP {}", status.as_u16())));
    }

    if src.kind == "youtube_live" {
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

fn process_result(src: &Source, ok: bool) -> (i64, HealthAction) {
    let new_failures = if ok { 0 } else { src.consecutive_failures + 1 };
    let action = if ok && !src.is_active {
        HealthAction::Reenable
    } else if !ok && new_failures >= FAILURE_THRESHOLD && src.is_active {
        HealthAction::Disable
    } else {
        HealthAction::None
    };
    (new_failures, action)
}

#[cfg(test)]
mod tests {
    use super::*;

    fn mock_source() -> Source {
        Source {
            id: 1,
            channel_id: 1,
            kind: "hls".to_string(),
            url: "https://example.com/stream.m3u8".to_string(),
            priority: 1,
            is_active: true,
            last_checked_at: None,
            last_status: None,
            consecutive_failures: 0,
            failure_reason: None,
        }
    }

    #[test]
    fn test_process_result_ok_resets_failures() {
        let src = Source {
            consecutive_failures: 2,
            ..mock_source()
        };
        let (failures, action) = process_result(&src, true);
        assert_eq!(failures, 0);
        assert!(matches!(action, HealthAction::None));
    }

    #[test]
    fn test_process_result_error_increments_failures() {
        let src = Source {
            consecutive_failures: 1,
            ..mock_source()
        };
        let (failures, action) = process_result(&src, false);
        assert_eq!(failures, 2);
        assert!(matches!(action, HealthAction::None));
    }

    #[test]
    fn test_process_result_triggers_disable_at_threshold() {
        let src = Source {
            consecutive_failures: 2,
            ..mock_source()
        };
        let (failures, action) = process_result(&src, false);
        assert_eq!(failures, 3);
        assert!(matches!(action, HealthAction::Disable));
    }

    #[test]
    fn test_process_result_already_inactive_not_disabled_again() {
        let src = Source {
            consecutive_failures: 2,
            is_active: false,
            ..mock_source()
        };
        let (failures, action) = process_result(&src, false);
        assert_eq!(failures, 3);
        assert!(matches!(action, HealthAction::None));
    }

    #[test]
    fn test_process_result_reenables_inactive_source_on_success() {
        let src = Source {
            is_active: false,
            consecutive_failures: 3,
            ..mock_source()
        };
        let (failures, action) = process_result(&src, true);
        assert_eq!(failures, 0);
        assert!(matches!(action, HealthAction::Reenable));
    }

    #[test]
    fn test_process_result_active_source_ok_no_action() {
        let src = mock_source();
        let (failures, action) = process_result(&src, true);
        assert_eq!(failures, 0);
        assert!(matches!(action, HealthAction::None));
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
}
