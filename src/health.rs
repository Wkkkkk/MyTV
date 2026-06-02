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

pub fn start(pool: SqlitePool, client: reqwest::Client, cors_cache: CorsCache) {
    tokio::spawn(async move {
        let mut interval = tokio::time::interval(CHECK_INTERVAL);
        interval.set_missed_tick_behavior(tokio::time::MissedTickBehavior::Skip);
        interval.tick().await;
        loop {
            interval.tick().await;
            check_all(&pool, &client, &cors_cache).await;
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

    // Only probe CORS for reachable HTTPS sources: a down source would just
    // incur a second timeout, and its cached budget is best left as-is.
    if ok && src.url.starts_with("https://") {
        if let Some(result) = crate::media::hls::probe_source_cors(client, &src.url).await {
            let host_key = crate::media::hls::extract_manifest_host(&src.url);
            cors_cache.write().await.insert(host_key.clone(), result);
            tracing::debug!(source_id = src.id, host = %host_key, cors = result, "CORS probe cached");
        }
    }
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
}
