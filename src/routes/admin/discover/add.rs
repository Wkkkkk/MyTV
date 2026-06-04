use axum::http::StatusCode;
use chrono::Utc;

use crate::model::{channel, playlist_item, source};
use crate::routes::internal_error;

pub struct DiscoverAddParams<'a> {
    pub pool: &'a sqlx::SqlitePool,
    pub client: &'a reqwest::Client,
    pub url: &'a str,
    pub title: &'a str,
    pub source_kind: &'a str,
    pub duration_secs: i64,
    pub channel_choice: &'a str,
    pub new_name: &'a str,
    pub new_category: &'a str,
    pub new_channel_type: &'a str,
}

pub async fn do_discover_add(params: DiscoverAddParams<'_>) -> Result<i64, StatusCode> {
    let DiscoverAddParams {
        pool,
        client,
        url,
        title,
        source_kind,
        duration_secs,
        channel_choice,
        new_name,
        new_category,
        new_channel_type,
    } = params;
    if url.is_empty() || (!url.starts_with("http://") && !url.starts_with("https://")) {
        return Err(StatusCode::UNPROCESSABLE_ENTITY);
    }
    let source_kind = source_kind
        .parse::<source::SourceKind>()
        .map_err(|_| StatusCode::UNPROCESSABLE_ENTITY)?;

    let channel_id = if channel_choice == "new" {
        if new_name.trim().is_empty() {
            return Err(StatusCode::UNPROCESSABLE_ENTITY);
        }
        let new_channel_type = new_channel_type
            .parse::<channel::ChannelType>()
            .map_err(|_| StatusCode::UNPROCESSABLE_ENTITY)?;
        let loop_anchor = if new_channel_type == channel::ChannelType::VodLoop {
            Some(Utc::now())
        } else {
            None
        };
        let ch = channel::create(
            pool,
            channel::NewChannel {
                name: new_name.trim().to_string(),
                category: new_category.trim().to_string(),
                logo_url: None,
                channel_type: new_channel_type,
                sort_order: 0,
                loop_anchor,
            },
        )
        .await
        .map_err(internal_error)?;
        ch.id
    } else {
        channel_choice
            .parse::<i64>()
            .map_err(|_| StatusCode::UNPROCESSABLE_ENTITY)?
    };

    let ch = channel::get(pool, channel_id)
        .await
        .map_err(internal_error)?
        .ok_or(StatusCode::NOT_FOUND)?;

    if ch.channel_type() == channel::ChannelType::VodLoop {
        let mut duration_secs = duration_secs;
        if duration_secs <= 0 {
            duration_secs = crate::media::fetch_duration(client, url)
                .await
                .map_err(|e| {
                    tracing::warn!(url = %url, error = %e, "failed to auto-fetch duration");
                    StatusCode::UNPROCESSABLE_ENTITY
                })?;
        }
        let items = playlist_item::list_for_channel(pool, channel_id)
            .await
            .map_err(internal_error)?;
        playlist_item::create(
            pool,
            playlist_item::NewPlaylistItem {
                channel_id,
                title: title.to_string(),
                url: url.to_string(),
                duration_secs,
                sort_order: items.len() as i64,
            },
        )
        .await
        .map_err(internal_error)?;
    } else {
        source::create(
            pool,
            source::NewSource {
                channel_id,
                kind: source_kind,
                url: url.to_string(),
                priority: 0,
            },
        )
        .await
        .map_err(internal_error)?;
    }

    Ok(channel_id)
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::{
        db,
        model::{channel, playlist_item, source},
    };
    use axum::http::StatusCode;
    use chrono::Utc;

    async fn test_pool() -> sqlx::SqlitePool {
        db::connect("sqlite::memory:").await.unwrap()
    }

    #[tokio::test]
    async fn test_add_new_live_channel_creates_source() {
        let pool = test_pool().await;
        let client = reqwest::Client::new();
        let ch_id = do_discover_add(DiscoverAddParams {
            pool: &pool,
            client: &client,
            url: "https://example.com/s.m3u8",
            title: "CNN",
            source_kind: "hls",
            duration_secs: 0,
            channel_choice: "new",
            new_name: "CNN",
            new_category: "news",
            new_channel_type: "live",
        })
        .await
        .unwrap();
        let ch = channel::get(&pool, ch_id).await.unwrap().unwrap();
        assert_eq!(ch.r#type, "live");
        let sources = source::list_for_channel(&pool, ch_id).await.unwrap();
        assert_eq!(sources.len(), 1);
        assert_eq!(sources[0].kind, "hls");
        assert_eq!(sources[0].url, "https://example.com/s.m3u8");
    }

    #[tokio::test]
    async fn test_add_new_vod_channel_creates_playlist_item() {
        let pool = test_pool().await;
        let client = reqwest::Client::new();
        let ch_id = do_discover_add(DiscoverAddParams {
            pool: &pool,
            client: &client,
            url: "https://example.com/ep1.mp4",
            title: "Ep 1",
            source_kind: "hls",
            duration_secs: 3600,
            channel_choice: "new",
            new_name: "My Show",
            new_category: "entertainment",
            new_channel_type: "vod_loop",
        })
        .await
        .unwrap();
        let ch = channel::get(&pool, ch_id).await.unwrap().unwrap();
        assert_eq!(ch.r#type, "vod_loop");
        assert!(ch.loop_anchor.is_some());
        let items = playlist_item::list_for_channel(&pool, ch_id).await.unwrap();
        assert_eq!(items.len(), 1);
        assert_eq!(items[0].duration_secs, 3600);
        assert_eq!(items[0].title, "Ep 1");
    }

    #[tokio::test]
    async fn test_add_source_to_existing_live_channel() {
        let pool = test_pool().await;
        let existing = channel::create(
            &pool,
            channel::NewChannel {
                name: "Existing".into(),
                category: "news".into(),
                logo_url: None,
                channel_type: channel::ChannelType::Live,
                sort_order: 0,
                loop_anchor: None,
            },
        )
        .await
        .unwrap();
        let client = reqwest::Client::new();
        let ch_id = do_discover_add(DiscoverAddParams {
            pool: &pool,
            client: &client,
            url: "https://example.com/s.m3u8",
            title: "Existing",
            source_kind: "iptv",
            duration_secs: 0,
            channel_choice: &existing.id.to_string(),
            new_name: "",
            new_category: "",
            new_channel_type: "",
        })
        .await
        .unwrap();
        assert_eq!(ch_id, existing.id);
        let sources = source::list_for_channel(&pool, ch_id).await.unwrap();
        assert_eq!(sources.len(), 1);
        assert_eq!(sources[0].kind, "iptv");
    }

    #[tokio::test]
    async fn test_add_playlist_item_to_existing_vod_channel() {
        let pool = test_pool().await;
        let existing = channel::create(
            &pool,
            channel::NewChannel {
                name: "VOD".into(),
                category: "movies".into(),
                logo_url: None,
                channel_type: channel::ChannelType::VodLoop,
                sort_order: 0,
                loop_anchor: Some(Utc::now()),
            },
        )
        .await
        .unwrap();
        let client = reqwest::Client::new();
        let ch_id = do_discover_add(DiscoverAddParams {
            pool: &pool,
            client: &client,
            url: "https://example.com/movie.mp4",
            title: "Movie",
            source_kind: "hls",
            duration_secs: 5400,
            channel_choice: &existing.id.to_string(),
            new_name: "",
            new_category: "",
            new_channel_type: "",
        })
        .await
        .unwrap();
        let items = playlist_item::list_for_channel(&pool, ch_id).await.unwrap();
        assert_eq!(items.len(), 1);
        assert_eq!(items[0].duration_secs, 5400);
    }

    #[tokio::test]
    async fn test_add_returns_422_when_new_name_empty() {
        let pool = test_pool().await;
        let client = reqwest::Client::new();
        let result = do_discover_add(DiscoverAddParams {
            pool: &pool,
            client: &client,
            url: "https://example.com/s.m3u8",
            title: "Test",
            source_kind: "hls",
            duration_secs: 0,
            channel_choice: "new",
            new_name: "",
            new_category: "news",
            new_channel_type: "live",
        })
        .await;
        assert_eq!(result, Err(StatusCode::UNPROCESSABLE_ENTITY));
    }

    #[tokio::test]
    async fn test_add_returns_422_when_vod_duration_zero() {
        let pool = test_pool().await;
        let client = reqwest::Client::new();
        let result = do_discover_add(DiscoverAddParams {
            pool: &pool,
            client: &client,
            url: "https://example.com/v.mp4",
            title: "Test",
            source_kind: "hls",
            duration_secs: 0,
            channel_choice: "new",
            new_name: "Show",
            new_category: "movies",
            new_channel_type: "vod_loop",
        })
        .await;
        assert_eq!(result, Err(StatusCode::UNPROCESSABLE_ENTITY));
    }
}
