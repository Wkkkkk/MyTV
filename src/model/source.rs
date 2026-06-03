use anyhow::Result;
use serde::{Deserialize, Serialize};
use sqlx::{FromRow, SqlitePool};

#[derive(Debug, Clone, Serialize, Deserialize, FromRow)]
pub struct Source {
    pub id: i64,
    pub channel_id: i64,
    pub kind: String,
    pub url: String,
    pub priority: i64,
    pub is_active: bool,
    pub last_checked_at: Option<i64>,
    pub last_status: Option<String>,
    pub consecutive_failures: i64,
    pub failure_reason: Option<String>,
}

pub struct NewSource {
    pub channel_id: i64,
    pub kind: String,
    pub url: String,
    pub priority: i64,
}

pub async fn create(pool: &SqlitePool, input: NewSource) -> Result<Source> {
    if !["hls", "youtube_live", "iptv"].contains(&input.kind.as_str()) {
        anyhow::bail!("invalid source kind: {}", input.kind);
    }
    let id = sqlx::query(
        "INSERT INTO sources (channel_id, kind, url, priority, is_active) VALUES (?, ?, ?, ?, 1)",
    )
    .bind(input.channel_id)
    .bind(&input.kind)
    .bind(&input.url)
    .bind(input.priority)
    .execute(pool)
    .await?
    .last_insert_rowid();

    sqlx::query_as::<_, Source>("SELECT * FROM sources WHERE id = ?")
        .bind(id)
        .fetch_one(pool)
        .await
        .map_err(Into::into)
}

pub async fn get(pool: &SqlitePool, id: i64) -> Result<Option<Source>> {
    sqlx::query_as::<_, Source>("SELECT * FROM sources WHERE id = ?")
        .bind(id)
        .fetch_optional(pool)
        .await
        .map_err(Into::into)
}

pub async fn list_for_channel(pool: &SqlitePool, channel_id: i64) -> Result<Vec<Source>> {
    sqlx::query_as::<_, Source>("SELECT * FROM sources WHERE channel_id = ? ORDER BY priority ASC")
        .bind(channel_id)
        .fetch_all(pool)
        .await
        .map_err(Into::into)
}

pub async fn list_active_for_channel(pool: &SqlitePool, channel_id: i64) -> Result<Vec<Source>> {
    sqlx::query_as::<_, Source>(
        "SELECT * FROM sources WHERE channel_id = ? AND is_active = 1 ORDER BY priority ASC",
    )
    .bind(channel_id)
    .fetch_all(pool)
    .await
    .map_err(Into::into)
}

pub async fn delete(pool: &SqlitePool, id: i64) -> Result<bool> {
    let rows = sqlx::query("DELETE FROM sources WHERE id = ?")
        .bind(id)
        .execute(pool)
        .await?
        .rows_affected();
    Ok(rows > 0)
}

pub async fn set_active(pool: &SqlitePool, id: i64, active: bool) -> Result<bool> {
    let rows = sqlx::query("UPDATE sources SET is_active = ? WHERE id = ?")
        .bind(active)
        .bind(id)
        .execute(pool)
        .await?
        .rows_affected();
    Ok(rows > 0)
}

pub async fn list_all(pool: &SqlitePool) -> Result<Vec<Source>> {
    sqlx::query_as::<_, Source>("SELECT * FROM sources ORDER BY channel_id ASC, priority ASC")
        .fetch_all(pool)
        .await
        .map_err(Into::into)
}

pub async fn update_health(
    pool: &SqlitePool,
    id: i64,
    status: &str,
    reason: Option<&str>,
    consecutive_failures: i64,
    is_active: Option<bool>,
) -> Result<()> {
    if let Some(active) = is_active {
        sqlx::query(
            "UPDATE sources
             SET last_checked_at = strftime('%s','now'),
                 last_status = ?,
                 failure_reason = ?,
                 consecutive_failures = ?,
                 is_active = ?
             WHERE id = ?",
        )
        .bind(status)
        .bind(reason)
        .bind(consecutive_failures)
        .bind(active)
        .bind(id)
        .execute(pool)
        .await?;
    } else {
        sqlx::query(
            "UPDATE sources
             SET last_checked_at = strftime('%s','now'),
                 last_status = ?,
                 failure_reason = ?,
                 consecutive_failures = ?
             WHERE id = ?",
        )
        .bind(status)
        .bind(reason)
        .bind(consecutive_failures)
        .bind(id)
        .execute(pool)
        .await?;
    }
    Ok(())
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::{db, model::channel};

    async fn test_pool() -> SqlitePool {
        db::connect("sqlite::memory:").await.unwrap()
    }

    async fn make_channel(pool: &SqlitePool) -> channel::Channel {
        channel::create(
            pool,
            channel::NewChannel {
                name: "Test".to_string(),
                category: "test".to_string(),
                logo_url: None,
                channel_type: "live".to_string(),
                sort_order: 0,
                loop_anchor: None,
            },
        )
        .await
        .unwrap()
    }

    fn hls(channel_id: i64, url: &str, priority: i64) -> NewSource {
        NewSource {
            channel_id,
            kind: "hls".to_string(),
            url: url.to_string(),
            priority,
        }
    }

    #[tokio::test]
    async fn test_create_and_list_sources_ordered_by_priority() {
        let pool = test_pool().await;
        let ch = make_channel(&pool).await;

        create(
            &pool,
            hls(ch.id, "https://primary.example.com/stream.m3u8", 1),
        )
        .await
        .unwrap();
        create(
            &pool,
            hls(ch.id, "https://backup.example.com/stream.m3u8", 2),
        )
        .await
        .unwrap();

        let sources = list_for_channel(&pool, ch.id).await.unwrap();
        assert_eq!(sources.len(), 2);
        assert_eq!(sources[0].priority, 1);
        assert_eq!(sources[1].priority, 2);
    }

    #[tokio::test]
    async fn test_list_active_excludes_inactive_sources() {
        let pool = test_pool().await;
        let ch = make_channel(&pool).await;

        let primary = create(
            &pool,
            hls(ch.id, "https://primary.example.com/stream.m3u8", 1),
        )
        .await
        .unwrap();
        create(
            &pool,
            hls(ch.id, "https://backup.example.com/stream.m3u8", 2),
        )
        .await
        .unwrap();

        sqlx::query("UPDATE sources SET is_active = 0 WHERE id = ?")
            .bind(primary.id)
            .execute(&pool)
            .await
            .unwrap();

        let active = list_active_for_channel(&pool, ch.id).await.unwrap();
        assert_eq!(active.len(), 1);
        assert_eq!(active[0].url, "https://backup.example.com/stream.m3u8");
    }

    #[tokio::test]
    async fn test_sources_deleted_when_channel_is_deleted() {
        let pool = test_pool().await;
        let ch = make_channel(&pool).await;

        create(
            &pool,
            hls(ch.id, "https://primary.example.com/stream.m3u8", 1),
        )
        .await
        .unwrap();

        channel::delete(&pool, ch.id).await.unwrap();

        let sources = list_for_channel(&pool, ch.id).await.unwrap();
        assert!(
            sources.is_empty(),
            "ON DELETE CASCADE should remove sources"
        );
    }

    #[tokio::test]
    async fn test_set_active_toggles_source() {
        let pool = test_pool().await;
        let ch = make_channel(&pool).await;

        let src = create(
            &pool,
            hls(ch.id, "https://primary.example.com/stream.m3u8", 1),
        )
        .await
        .unwrap();
        assert!(src.is_active);

        let updated = set_active(&pool, src.id, false).await.unwrap();
        assert!(updated);

        let sources = list_active_for_channel(&pool, ch.id).await.unwrap();
        assert!(sources.is_empty());

        set_active(&pool, src.id, true).await.unwrap();
        let sources = list_active_for_channel(&pool, ch.id).await.unwrap();
        assert_eq!(sources.len(), 1);
    }

    #[tokio::test]
    async fn test_list_all_returns_sources_from_all_channels() {
        let pool = test_pool().await;
        let ch1 = make_channel(&pool).await;
        let ch2 = make_channel(&pool).await;
        create(&pool, hls(ch1.id, "https://a.example.com/stream.m3u8", 1))
            .await
            .unwrap();
        create(&pool, hls(ch2.id, "https://b.example.com/stream.m3u8", 1))
            .await
            .unwrap();
        let all = list_all(&pool).await.unwrap();
        assert_eq!(all.len(), 2);
    }

    #[tokio::test]
    async fn test_update_health_ok_resets_failures() {
        let pool = test_pool().await;
        let ch = make_channel(&pool).await;
        let src = create(
            &pool,
            hls(ch.id, "https://primary.example.com/stream.m3u8", 1),
        )
        .await
        .unwrap();

        update_health(&pool, src.id, "error", Some("timeout"), 2, None)
            .await
            .unwrap();
        update_health(&pool, src.id, "ok", None, 0, None)
            .await
            .unwrap();

        let updated = get(&pool, src.id).await.unwrap().unwrap();
        assert_eq!(updated.last_status.as_deref(), Some("ok"));
        assert_eq!(updated.consecutive_failures, 0);
        assert!(updated.failure_reason.is_none());
        assert!(updated.is_active);
    }

    #[tokio::test]
    async fn test_update_health_disables_after_threshold() {
        let pool = test_pool().await;
        let ch = make_channel(&pool).await;
        let src = create(
            &pool,
            hls(ch.id, "https://primary.example.com/stream.m3u8", 1),
        )
        .await
        .unwrap();

        update_health(
            &pool,
            src.id,
            "error",
            Some("connection refused"),
            3,
            Some(false),
        )
        .await
        .unwrap();

        let updated = get(&pool, src.id).await.unwrap().unwrap();
        assert!(!updated.is_active);
        assert_eq!(updated.consecutive_failures, 3);
        assert_eq!(updated.last_status.as_deref(), Some("error"));
        assert_eq!(
            updated.failure_reason.as_deref(),
            Some("connection refused")
        );
    }

    #[tokio::test]
    async fn test_update_health_reenables_disabled_source() {
        let pool = test_pool().await;
        let ch = make_channel(&pool).await;
        let src = create(
            &pool,
            hls(ch.id, "https://primary.example.com/stream.m3u8", 1),
        )
        .await
        .unwrap();

        // disable it first
        update_health(&pool, src.id, "error", Some("timeout"), 3, Some(false))
            .await
            .unwrap();
        let disabled = get(&pool, src.id).await.unwrap().unwrap();
        assert!(!disabled.is_active);

        // now re-enable it
        update_health(&pool, src.id, "ok", None, 0, Some(true))
            .await
            .unwrap();
        let reenabled = get(&pool, src.id).await.unwrap().unwrap();
        assert!(reenabled.is_active);
        assert_eq!(reenabled.consecutive_failures, 0);
        assert_eq!(reenabled.last_status.as_deref(), Some("ok"));
        assert!(reenabled.failure_reason.is_none());
    }
}
