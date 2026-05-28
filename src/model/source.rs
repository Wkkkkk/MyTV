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
}

pub struct NewSource {
    pub channel_id: i64,
    pub kind: String,
    pub url: String,
    pub priority: i64,
}

pub async fn create(pool: &SqlitePool, input: NewSource) -> Result<Source> {
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
    sqlx::query_as::<_, Source>(
        "SELECT * FROM sources WHERE channel_id = ? ORDER BY priority ASC",
    )
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

#[cfg(test)]
mod tests {
    use super::*;
    use crate::{model::channel, db};

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

        create(&pool, hls(ch.id, "https://primary.example.com/stream.m3u8", 1)).await.unwrap();
        create(&pool, hls(ch.id, "https://backup.example.com/stream.m3u8", 2)).await.unwrap();

        let sources = list_for_channel(&pool, ch.id).await.unwrap();
        assert_eq!(sources.len(), 2);
        assert_eq!(sources[0].priority, 1);
        assert_eq!(sources[1].priority, 2);
    }

    #[tokio::test]
    async fn test_list_active_excludes_inactive_sources() {
        let pool = test_pool().await;
        let ch = make_channel(&pool).await;

        let primary = create(&pool, hls(ch.id, "https://primary.example.com/stream.m3u8", 1)).await.unwrap();
        create(&pool, hls(ch.id, "https://backup.example.com/stream.m3u8", 2)).await.unwrap();

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

        create(&pool, hls(ch.id, "https://primary.example.com/stream.m3u8", 1)).await.unwrap();

        channel::delete(&pool, ch.id).await.unwrap();

        let sources = list_for_channel(&pool, ch.id).await.unwrap();
        assert!(sources.is_empty(), "ON DELETE CASCADE should remove sources");
    }

    #[tokio::test]
    async fn test_set_active_toggles_source() {
        let pool = test_pool().await;
        let ch = make_channel(&pool).await;

        let src = create(&pool, hls(ch.id, "https://primary.example.com/stream.m3u8", 1)).await.unwrap();
        assert!(src.is_active);

        let updated = set_active(&pool, src.id, false).await.unwrap();
        assert!(updated);

        let sources = list_active_for_channel(&pool, ch.id).await.unwrap();
        assert!(sources.is_empty());

        set_active(&pool, src.id, true).await.unwrap();
        let sources = list_active_for_channel(&pool, ch.id).await.unwrap();
        assert_eq!(sources.len(), 1);
    }
}
