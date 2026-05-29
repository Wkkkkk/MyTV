use std::time::Duration;

use sqlx::SqlitePool;

use crate::model::source::{self, Source};

const CHECK_INTERVAL: Duration = Duration::from_secs(15 * 60);
const HTTP_TIMEOUT: Duration = Duration::from_secs(5);
pub(crate) const FAILURE_THRESHOLD: i64 = 3;

pub fn start(pool: SqlitePool, client: reqwest::Client) {
    tokio::spawn(async move {
        let mut interval = tokio::time::interval(CHECK_INTERVAL);
        interval.set_missed_tick_behavior(tokio::time::MissedTickBehavior::Skip);
        interval.tick().await; // consume the immediate first tick so we don't check at startup
        loop {
            interval.tick().await;
            check_all(&pool, &client).await;
        }
    });
}

async fn check_all(pool: &SqlitePool, client: &reqwest::Client) {
    let sources = match source::list_all(pool).await {
        Ok(s) => s,
        Err(e) => {
            tracing::error!("health: failed to fetch sources: {e}");
            return;
        }
    };
    for src in sources {
        check_one(pool, client, &src).await;
    }
}

async fn check_one(pool: &SqlitePool, client: &reqwest::Client, src: &Source) {
    let (ok, reason) = do_http_check(client, src).await;
    let (new_failures, set_inactive) = process_result(src, ok);

    if let Err(e) = source::update_health(
        pool,
        src.id,
        if ok { "ok" } else { "error" },
        reason.as_deref(),
        new_failures,
        set_inactive,
    )
    .await
    {
        tracing::error!("health: failed to update source {}: {e}", src.id);
        return;
    }

    if set_inactive {
        tracing::warn!(
            "health: source {} auto-disabled after {} consecutive failures",
            src.id,
            new_failures
        );
    }
}

async fn do_http_check(client: &reqwest::Client, src: &Source) -> (bool, Option<String>) {
    let mut resp = match client
        .get(&src.url)
        .timeout(HTTP_TIMEOUT)
        .send()
        .await
    {
        Ok(r) => r,
        Err(e) => return (false, Some(format!("request failed: {e}"))),
    };

    let status = resp.status();
    if !status.is_success() && !status.is_redirection() {
        return (false, Some(format!("HTTP {}", status.as_u16())));
    }

    // YouTube live: HTTP 200 is sufficient — yt-dlp resolution is too slow for background checks
    if src.kind == "youtube_live" {
        return (true, None);
    }

    // HLS / IPTV: read one chunk to verify the stream actually delivers bytes
    match resp.chunk().await {
        Ok(Some(_)) => (true, None),
        Ok(None) => (false, Some("stream returned no data".to_string())),
        Err(e) => (false, Some(format!("read failed: {e}"))),
    }
}

fn process_result(src: &Source, ok: bool) -> (i64, bool) {
    let new_failures = if ok { 0 } else { src.consecutive_failures + 1 };
    let set_inactive = !ok && new_failures >= FAILURE_THRESHOLD && src.is_active;
    (new_failures, set_inactive)
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
        let src = Source { consecutive_failures: 2, ..mock_source() };
        let (failures, disable) = process_result(&src, true);
        assert_eq!(failures, 0);
        assert!(!disable);
    }

    #[test]
    fn test_process_result_error_increments_failures() {
        let src = Source { consecutive_failures: 1, ..mock_source() };
        let (failures, disable) = process_result(&src, false);
        assert_eq!(failures, 2);
        assert!(!disable);
    }

    #[test]
    fn test_process_result_triggers_disable_at_threshold() {
        let src = Source { consecutive_failures: 2, ..mock_source() };
        let (failures, disable) = process_result(&src, false);
        assert_eq!(failures, 3);
        assert!(disable);
    }

    #[test]
    fn test_process_result_already_inactive_not_disabled_again() {
        let src = Source { consecutive_failures: 2, is_active: false, ..mock_source() };
        let (failures, disable) = process_result(&src, false);
        assert_eq!(failures, 3);
        assert!(!disable);
    }
}
