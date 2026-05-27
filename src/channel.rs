use anyhow::Result;
use chrono::{DateTime, Utc};
use serde::{Deserialize, Serialize};
use sqlx::{FromRow, SqlitePool};

#[derive(Debug, Clone, Serialize, Deserialize, FromRow)]
pub struct Channel {
    pub id: i64,
    pub name: String,
    pub category: String,
    pub logo_url: Option<String>,
    pub r#type: String,
    pub sort_order: i64,
    pub loop_anchor: Option<DateTime<Utc>>,
}

#[derive(Debug, Clone, PartialEq)]
pub enum ChannelType {
    Live,
    VodLoop,
}

impl Channel {
    pub fn channel_type(&self) -> ChannelType {
        match self.r#type.as_str() {
            "vod_loop" => ChannelType::VodLoop,
            _ => ChannelType::Live,
        }
    }
}

pub struct NewChannel {
    pub name: String,
    pub category: String,
    pub logo_url: Option<String>,
    pub channel_type: String,
    pub sort_order: i64,
    pub loop_anchor: Option<DateTime<Utc>>,
}

pub async fn create(pool: &SqlitePool, input: NewChannel) -> Result<Channel> {
    let id = sqlx::query(
        "INSERT INTO channels (name, category, logo_url, type, sort_order, loop_anchor)
         VALUES (?, ?, ?, ?, ?, ?)",
    )
    .bind(&input.name)
    .bind(&input.category)
    .bind(&input.logo_url)
    .bind(&input.channel_type)
    .bind(input.sort_order)
    .bind(input.loop_anchor)
    .execute(pool)
    .await?
    .last_insert_rowid();

    get(pool, id)
        .await?
        .ok_or_else(|| anyhow::anyhow!("channel not found after insert"))
}

pub struct UpdateChannel {
    pub name: String,
    pub category: String,
    pub logo_url: Option<String>,
    pub channel_type: String,
    pub sort_order: i64,
    pub loop_anchor: Option<DateTime<Utc>>,
}

pub async fn update(pool: &SqlitePool, id: i64, input: UpdateChannel) -> Result<Option<Channel>> {
    let rows = sqlx::query(
        "UPDATE channels SET name = ?, category = ?, logo_url = ?, type = ?, sort_order = ?, loop_anchor = ? WHERE id = ?",
    )
    .bind(&input.name)
    .bind(&input.category)
    .bind(&input.logo_url)
    .bind(&input.channel_type)
    .bind(input.sort_order)
    .bind(input.loop_anchor)
    .bind(id)
    .execute(pool)
    .await?
    .rows_affected();

    if rows == 0 {
        return Ok(None);
    }
    get(pool, id).await
}

pub async fn get(pool: &SqlitePool, id: i64) -> Result<Option<Channel>> {
    sqlx::query_as::<_, Channel>("SELECT * FROM channels WHERE id = ?")
        .bind(id)
        .fetch_optional(pool)
        .await
        .map_err(Into::into)
}

pub async fn list(pool: &SqlitePool) -> Result<Vec<Channel>> {
    sqlx::query_as::<_, Channel>(
        "SELECT * FROM channels ORDER BY sort_order ASC, name ASC",
    )
    .fetch_all(pool)
    .await
    .map_err(Into::into)
}

pub async fn list_by_category(pool: &SqlitePool, category: &str) -> Result<Vec<Channel>> {
    sqlx::query_as::<_, Channel>(
        "SELECT * FROM channels WHERE category = ? ORDER BY sort_order ASC, name ASC",
    )
    .bind(category)
    .fetch_all(pool)
    .await
    .map_err(Into::into)
}

pub async fn delete(pool: &SqlitePool, id: i64) -> Result<bool> {
    let rows = sqlx::query("DELETE FROM channels WHERE id = ?")
        .bind(id)
        .execute(pool)
        .await?
        .rows_affected();
    Ok(rows > 0)
}

/// Returns a sorted, deduplicated list of category names from a channel slice.
pub fn distinct_categories(channels: &[Channel]) -> Vec<String> {
    let mut cats: Vec<String> = channels
        .iter()
        .map(|c| c.category.clone())
        .collect::<std::collections::HashSet<_>>()
        .into_iter()
        .collect();
    cats.sort();
    cats
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::db;

    async fn test_pool() -> SqlitePool {
        db::connect("sqlite::memory:").await.unwrap()
    }

    fn live(name: &str, category: &str) -> NewChannel {
        NewChannel {
            name: name.to_string(),
            category: category.to_string(),
            logo_url: None,
            channel_type: "live".to_string(),
            sort_order: 0,
            loop_anchor: None,
        }
    }

    #[tokio::test]
    async fn test_create_and_get_channel() {
        let pool = test_pool().await;

        let ch = create(&pool, live("CNN International", "news")).await.unwrap();

        assert_eq!(ch.name, "CNN International");
        assert_eq!(ch.category, "news");
        assert_eq!(ch.channel_type(), ChannelType::Live);

        let fetched = get(&pool, ch.id).await.unwrap().unwrap();
        assert_eq!(fetched.id, ch.id);
    }

    #[tokio::test]
    async fn test_list_returns_all_channels() {
        let pool = test_pool().await;
        create(&pool, live("CNN", "news")).await.unwrap();
        create(&pool, live("ESPN", "sports")).await.unwrap();
        create(&pool, live("BBC", "news")).await.unwrap();

        let all = list(&pool).await.unwrap();
        assert_eq!(all.len(), 3);
    }

    #[tokio::test]
    async fn test_list_by_category_filters_correctly() {
        let pool = test_pool().await;
        create(&pool, live("CNN", "news")).await.unwrap();
        create(&pool, live("ESPN", "sports")).await.unwrap();
        create(&pool, live("BBC", "news")).await.unwrap();

        let news = list_by_category(&pool, "news").await.unwrap();
        assert_eq!(news.len(), 2);
        assert!(news.iter().all(|c| c.category == "news"));
    }

    #[tokio::test]
    async fn test_delete_channel() {
        let pool = test_pool().await;
        let ch = create(&pool, live("TMP", "test")).await.unwrap();

        assert!(delete(&pool, ch.id).await.unwrap());
        assert!(get(&pool, ch.id).await.unwrap().is_none());
    }

    #[tokio::test]
    async fn test_distinct_categories_sorted_deduped() {
        let pool = test_pool().await;
        create(&pool, live("CNN", "news")).await.unwrap();
        create(&pool, live("ESPN", "sports")).await.unwrap();
        create(&pool, live("BBC", "news")).await.unwrap();

        let all = list(&pool).await.unwrap();
        let cats = distinct_categories(&all);
        assert_eq!(cats, vec!["news", "sports"]);
    }

    #[tokio::test]
    async fn test_update_channel_name_and_category() {
        let pool = test_pool().await;
        let ch = create(&pool, live("CNN", "news")).await.unwrap();

        let updated = update(
            &pool,
            ch.id,
            UpdateChannel {
                name: "CNN International".to_string(),
                category: "world".to_string(),
                logo_url: None,
                channel_type: "live".to_string(),
                sort_order: 1,
                loop_anchor: None,
            },
        )
        .await
        .unwrap()
        .unwrap();

        assert_eq!(updated.name, "CNN International");
        assert_eq!(updated.category, "world");
        assert_eq!(updated.sort_order, 1);
    }

    #[tokio::test]
    async fn test_update_nonexistent_channel_returns_none() {
        let pool = test_pool().await;
        let result = update(
            &pool,
            9999,
            UpdateChannel {
                name: "Ghost".to_string(),
                category: "none".to_string(),
                logo_url: None,
                channel_type: "live".to_string(),
                sort_order: 0,
                loop_anchor: None,
            },
        )
        .await
        .unwrap();
        assert!(result.is_none());
    }
}
