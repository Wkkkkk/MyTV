use anyhow::Result;
use serde::{Deserialize, Serialize};
use sqlx::{FromRow, SqlitePool};

#[derive(Debug, Clone, Serialize, Deserialize, FromRow)]
pub struct PlaylistItem {
    pub id: i64,
    pub channel_id: i64,
    pub title: String,
    pub url: String,
    pub duration_secs: i64,
    pub sort_order: i64,
}

pub struct NewPlaylistItem {
    pub channel_id: i64,
    pub title: String,
    pub url: String,
    pub duration_secs: i64,
    pub sort_order: i64,
}

pub async fn create(pool: &SqlitePool, input: NewPlaylistItem) -> Result<PlaylistItem> {
    let id = sqlx::query(
        "INSERT INTO playlist_items (channel_id, title, url, duration_secs, sort_order)
         VALUES (?, ?, ?, ?, ?)",
    )
    .bind(input.channel_id)
    .bind(&input.title)
    .bind(&input.url)
    .bind(input.duration_secs)
    .bind(input.sort_order)
    .execute(pool)
    .await?
    .last_insert_rowid();

    sqlx::query_as::<_, PlaylistItem>("SELECT * FROM playlist_items WHERE id = ?")
        .bind(id)
        .fetch_one(pool)
        .await
        .map_err(Into::into)
}

pub async fn list_for_channel(pool: &SqlitePool, channel_id: i64) -> Result<Vec<PlaylistItem>> {
    sqlx::query_as::<_, PlaylistItem>(
        "SELECT * FROM playlist_items WHERE channel_id = ? ORDER BY sort_order ASC",
    )
    .bind(channel_id)
    .fetch_all(pool)
    .await
    .map_err(Into::into)
}

pub async fn delete(pool: &SqlitePool, id: i64) -> Result<bool> {
    let rows = sqlx::query("DELETE FROM playlist_items WHERE id = ?")
        .bind(id)
        .execute(pool)
        .await?
        .rows_affected();
    Ok(rows > 0)
}

pub fn total_duration_secs(items: &[PlaylistItem]) -> i64 {
    items.iter().map(|i| i.duration_secs).sum()
}

/// Given a playlist and unix timestamps (seconds), returns the index of the
/// currently playing item and the playback offset in seconds within that item.
/// Returns None if the playlist is empty.
pub fn current_position(
    items: &[PlaylistItem],
    now_secs: i64,
    anchor_secs: i64,
) -> Option<(usize, i64)> {
    if items.is_empty() {
        return None;
    }
    let total = total_duration_secs(items);
    if total <= 0 {
        return None;
    }
    let elapsed = (now_secs - anchor_secs).rem_euclid(total);
    let mut acc = 0i64;
    for (i, item) in items.iter().enumerate() {
        acc += item.duration_secs;
        if elapsed < acc {
            let offset = elapsed - (acc - item.duration_secs);
            return Some((i, offset));
        }
    }
    Some((items.len() - 1, items.last().unwrap().duration_secs))
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::{channel, db};

    async fn test_pool() -> SqlitePool {
        db::connect("sqlite::memory:").await.unwrap()
    }

    async fn make_channel(pool: &SqlitePool) -> channel::Channel {
        channel::create(
            pool,
            channel::NewChannel {
                name: "VOD Loop".to_string(),
                category: "test".to_string(),
                logo_url: None,
                channel_type: "vod_loop".to_string(),
                sort_order: 0,
                loop_anchor: None,
            },
        )
        .await
        .unwrap()
    }

    fn item(channel_id: i64, title: &str, duration_secs: i64, sort_order: i64) -> NewPlaylistItem {
        NewPlaylistItem {
            channel_id,
            title: title.to_string(),
            url: format!("https://example.com/{}.mp4", title),
            duration_secs,
            sort_order,
        }
    }

    #[tokio::test]
    async fn test_create_and_list_playlist_items_in_order() {
        let pool = test_pool().await;
        let ch = make_channel(&pool).await;

        create(&pool, item(ch.id, "ep1", 1800, 0)).await.unwrap();
        create(&pool, item(ch.id, "ep2", 2400, 1)).await.unwrap();

        let items = list_for_channel(&pool, ch.id).await.unwrap();
        assert_eq!(items.len(), 2);
        assert_eq!(items[0].title, "ep1");
        assert_eq!(items[1].title, "ep2");
        assert_eq!(total_duration_secs(&items), 4200);
    }

    #[tokio::test]
    async fn test_current_position_within_first_item() {
        let items = vec![
            PlaylistItem { id: 1, channel_id: 1, title: "A".into(), url: "u".into(), duration_secs: 3600, sort_order: 0 },
            PlaylistItem { id: 2, channel_id: 1, title: "B".into(), url: "u".into(), duration_secs: 1800, sort_order: 1 },
        ];
        // 500 seconds into the loop — still in item A
        let (idx, offset) = current_position(&items, 1500, 1000).unwrap();
        assert_eq!(idx, 0);
        assert_eq!(offset, 500);
    }

    #[tokio::test]
    async fn test_current_position_within_second_item() {
        let items = vec![
            PlaylistItem { id: 1, channel_id: 1, title: "A".into(), url: "u".into(), duration_secs: 3600, sort_order: 0 },
            PlaylistItem { id: 2, channel_id: 1, title: "B".into(), url: "u".into(), duration_secs: 1800, sort_order: 1 },
        ];
        // 4000 seconds in — 400 seconds into item B (after A's 3600)
        let (idx, offset) = current_position(&items, 4000, 0).unwrap();
        assert_eq!(idx, 1);
        assert_eq!(offset, 400);
    }

    #[tokio::test]
    async fn test_current_position_wraps_around_to_start() {
        let items = vec![
            PlaylistItem { id: 1, channel_id: 1, title: "A".into(), url: "u".into(), duration_secs: 3600, sort_order: 0 },
            PlaylistItem { id: 2, channel_id: 1, title: "B".into(), url: "u".into(), duration_secs: 1800, sort_order: 1 },
        ];
        // total = 5400; 5500 seconds in wraps to 100 seconds into item A
        let (idx, offset) = current_position(&items, 5500, 0).unwrap();
        assert_eq!(idx, 0);
        assert_eq!(offset, 100);
    }

    #[tokio::test]
    async fn test_current_position_empty_playlist_returns_none() {
        let result = current_position(&[], 1000, 0);
        assert!(result.is_none());
    }

    #[tokio::test]
    async fn test_playlist_items_deleted_when_channel_is_deleted() {
        let pool = test_pool().await;
        let ch = make_channel(&pool).await;

        create(&pool, item(ch.id, "ep1", 1800, 0)).await.unwrap();

        channel::delete(&pool, ch.id).await.unwrap();

        let items = list_for_channel(&pool, ch.id).await.unwrap();
        assert!(items.is_empty(), "ON DELETE CASCADE should remove playlist items");
    }
}
