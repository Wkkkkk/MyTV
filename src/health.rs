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

    let items = match crate::model::playlist_item::list_all(pool).await {
        Ok(i) => i,
        Err(e) => {
            tracing::error!("health: failed to fetch playlist items: {e}");
            return;
        }
    };
    for item in items {
        check_playlist_item(pool, client, cors_cache, &item).await;
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
    let (ok, reason) = do_http_check(client, &src.url, &src.kind).await;
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

async fn check_source(
    pool: &SqlitePool,
    client: &reqwest::Client,
    cors_cache: &CorsCache,
    src: &Source,
) {
    let (ok, reason) = do_http_check(client, &src.url, &src.kind).await;
    let (new_failures, action) = process_result(src.is_active, src.consecutive_failures, ok);

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

async fn check_playlist_item(
    pool: &SqlitePool,
    client: &reqwest::Client,
    cors_cache: &CorsCache,
    item: &crate::model::playlist_item::PlaylistItem,
) {
    let kind = crate::model::source::SourceKind::detect(&item.url);
    let (ok, reason) = do_http_check(client, &item.url, kind.as_str()).await;
    let (new_failures, action) = process_result(item.is_active, item.consecutive_failures, ok);

    let is_active = match action {
        HealthAction::Disable => Some(false),
        HealthAction::Reenable => Some(true),
        HealthAction::None => None,
    };

    if let Err(e) = crate::model::playlist_item::update_health(
        pool,
        item.id,
        if ok { "ok" } else { "error" },
        reason.as_deref(),
        new_failures,
        is_active,
    )
    .await
    {
        tracing::error!("health: failed to update playlist item {}: {e}", item.id);
        return;
    }

    match action {
        HealthAction::Disable => tracing::warn!(
            "health: playlist item {} auto-disabled after {} consecutive failures",
            item.id,
            new_failures
        ),
        HealthAction::Reenable => tracing::info!(
            "health: playlist item {} auto-re-enabled after passing health check",
            item.id
        ),
        HealthAction::None => {}
    }

    if ok {
        probe_and_cache_cors(client, cors_cache, &item.url).await;
    }
}

pub async fn probe_playlist_item(
    pool: &SqlitePool,
    client: &reqwest::Client,
    cors_cache: &CorsCache,
    item: &crate::model::playlist_item::PlaylistItem,
) {
    let kind = crate::model::source::SourceKind::detect(&item.url);
    let (ok, reason) = do_http_check(client, &item.url, kind.as_str()).await;
    let new_failures = if ok { 0 } else { item.consecutive_failures + 1 };

    if let Err(e) = crate::model::playlist_item::update_health(
        pool,
        item.id,
        if ok { "ok" } else { "error" },
        reason.as_deref(),
        new_failures,
        None,
    )
    .await
    {
        tracing::error!("health: failed to update playlist item {}: {e}", item.id);
        return;
    }

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
    let result = if crate::model::source::SourceKind::detect(url)
        == crate::model::source::SourceKind::Dash
    {
        crate::media::mpd::probe_mpd_cors(client, url).await?
    } else {
        crate::media::hls::probe_source_cors(client, url).await?
    };
    let host = crate::media::hls::extract_manifest_host(url);
    tracing::debug!(host = %host, cors = result, "CORS probe cached");
    cors_cache.write().await.insert(host, result);
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
}
