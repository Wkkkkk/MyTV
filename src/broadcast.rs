use std::future::Future;

use chrono::{DateTime, Utc};
use sqlx::SqlitePool;

use crate::media::resolver;
use crate::model::{channel, playlist_item, source};

/// Outcome of an ended-live → VOD conversion attempt.
#[derive(Debug, PartialEq, Eq)]
pub enum ConversionOutcome {
    /// This call won the idempotency claim: flipped the channel to vod_loop,
    /// appended the recording, deactivated the live sources.
    Converted,
    /// The channel was already a VOD loop (a concurrent or repeat tune won the
    /// claim first) — nothing to do.
    AlreadyConverted,
}

/// Converts an ended live channel into a VOD loop. The recording's watch URL
/// and duration come from the injected `resolve` closure (yt-dlp in production,
/// a stub in tests), then the atomic flip → append → deactivate runs.
///
/// Resolve runs *before* the claim: claiming first then failing the resolve
/// would leave the channel flipped with no playlist item (an empty VOD → 503).
/// The flip (`set_type_and_anchor_if_live`) is the idempotency gate — an
/// already-converted channel yields `AlreadyConverted` without appending, and
/// two racing tunes append exactly one item.
pub async fn convert_if_ended<F, Fut>(
    pool: &SqlitePool,
    channel_id: i64,
    title: &str,
    source_url: &str,
    anchor: DateTime<Utc>,
    resolve: F,
) -> anyhow::Result<ConversionOutcome>
where
    F: FnOnce(String) -> Fut,
    Fut: Future<Output = anyhow::Result<(String, i64)>>,
{
    let (watch_url, duration) = resolve(source_url.to_string()).await?;
    if !channel::set_type_and_anchor_if_live(pool, channel_id, anchor).await? {
        return Ok(ConversionOutcome::AlreadyConverted);
    }
    playlist_item::create(
        pool,
        playlist_item::NewPlaylistItem {
            channel_id,
            title: title.to_string(),
            url: watch_url,
            duration_secs: duration,
            sort_order: 0,
        },
    )
    .await?;
    source::deactivate_all_for_channel(pool, channel_id).await?;
    Ok(ConversionOutcome::Converted)
}

/// Production resolver: derives the recording's canonical watch URL (from the
/// embedded id, falling back to a yt-dlp id lookup) and its duration via yt-dlp.
pub async fn resolve_recording(source_url: String) -> anyhow::Result<(String, i64)> {
    let watch_url = match resolver::live_url_to_watch_url(&source_url) {
        Some(u) => u,
        None => {
            let id = resolver::fetch_video_id(&source_url).await?;
            format!("https://www.youtube.com/watch?v={id}")
        }
    };
    let duration = resolver::fetch_duration_secs(&watch_url).await?;
    Ok((watch_url, duration))
}

/// Thin adapter: fire the conversion as a detached task using the real yt-dlp
/// resolver. Failures are logged and dropped — the broadcast simply stays live
/// until the next tune retries.
pub fn spawn_conversion(pool: SqlitePool, channel_id: i64, title: String, source_url: String) {
    tokio::spawn(async move {
        if let Err(e) = convert_if_ended(
            &pool,
            channel_id,
            &title,
            &source_url,
            Utc::now(),
            resolve_recording,
        )
        .await
        {
            tracing::warn!(channel_id, error = %e, "ended-live → VOD conversion failed");
        }
    });
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::db;
    use chrono::DateTime;

    async fn test_pool() -> SqlitePool {
        db::connect("sqlite::memory:").await.unwrap()
    }

    async fn make_live_channel(pool: &SqlitePool) -> channel::Channel {
        channel::create(
            pool,
            channel::NewChannel {
                name: "Live Test".into(),
                category: "test".into(),
                logo_url: None,
                channel_type: channel::ChannelType::Live,
                sort_order: 0,
                loop_anchor: None,
            },
        )
        .await
        .unwrap()
    }

    async fn make_live_source(pool: &SqlitePool, channel_id: i64) {
        source::create(
            pool,
            source::NewSource {
                channel_id,
                kind: source::SourceKind::YoutubeLive,
                url: "https://www.youtube.com/live/abc123".into(),
                priority: 1,
            },
        )
        .await
        .unwrap();
    }

    async fn stub_ok(_url: String) -> anyhow::Result<(String, i64)> {
        Ok(("https://www.youtube.com/watch?v=abc123".to_string(), 212))
    }

    async fn stub_err(_url: String) -> anyhow::Result<(String, i64)> {
        Err(anyhow::anyhow!("resolve failed"))
    }

    #[tokio::test]
    async fn convert_if_ended_flips_and_appends() {
        let pool = test_pool().await;
        let ch = make_live_channel(&pool).await;
        make_live_source(&pool, ch.id).await;
        let anchor = DateTime::from_timestamp(1_700_000_000, 0).unwrap();

        let outcome = convert_if_ended(&pool, ch.id, "Live Test", "src", anchor, stub_ok)
            .await
            .unwrap();
        assert_eq!(outcome, ConversionOutcome::Converted);

        let updated = channel::get(&pool, ch.id).await.unwrap().unwrap();
        assert_eq!(updated.channel_type(), channel::ChannelType::VodLoop);
        assert_eq!(updated.loop_anchor, Some(anchor));

        let items = playlist_item::list_active_for_channel(&pool, ch.id)
            .await
            .unwrap();
        assert_eq!(items.len(), 1);
        assert_eq!(items[0].url, "https://www.youtube.com/watch?v=abc123");
        assert_eq!(items[0].duration_secs, 212);
        assert_eq!(items[0].title, "Live Test");

        assert!(source::list_active_for_channel(&pool, ch.id)
            .await
            .unwrap()
            .is_empty());
    }

    #[tokio::test]
    async fn convert_if_ended_is_idempotent() {
        let pool = test_pool().await;
        let ch = make_live_channel(&pool).await;
        make_live_source(&pool, ch.id).await;
        let anchor = DateTime::from_timestamp(1_700_000_000, 0).unwrap();

        convert_if_ended(&pool, ch.id, "Live Test", "src", anchor, stub_ok)
            .await
            .unwrap();
        let outcome = convert_if_ended(&pool, ch.id, "Live Test", "src", anchor, stub_ok)
            .await
            .unwrap();
        assert_eq!(outcome, ConversionOutcome::AlreadyConverted);

        assert_eq!(
            playlist_item::list_active_for_channel(&pool, ch.id)
                .await
                .unwrap()
                .len(),
            1,
            "second conversion must not append a duplicate item"
        );
    }

    #[tokio::test]
    async fn convert_if_ended_concurrent_appends_once() {
        let pool = test_pool().await;
        let ch = make_live_channel(&pool).await;
        make_live_source(&pool, ch.id).await;
        let anchor = DateTime::from_timestamp(1_700_000_000, 0).unwrap();

        let (a, b) = tokio::join!(
            convert_if_ended(&pool, ch.id, "Live Test", "src", anchor, stub_ok),
            convert_if_ended(&pool, ch.id, "Live Test", "src", anchor, stub_ok),
        );
        a.unwrap();
        b.unwrap();

        assert_eq!(
            playlist_item::list_active_for_channel(&pool, ch.id)
                .await
                .unwrap()
                .len(),
            1,
            "two racing conversions must append exactly one item"
        );
    }

    #[tokio::test]
    async fn convert_if_ended_resolve_failure_leaves_channel_live() {
        let pool = test_pool().await;
        let ch = make_live_channel(&pool).await;
        make_live_source(&pool, ch.id).await;
        let anchor = DateTime::from_timestamp(1_700_000_000, 0).unwrap();

        let result = convert_if_ended(&pool, ch.id, "Live Test", "src", anchor, stub_err).await;
        assert!(result.is_err());

        let updated = channel::get(&pool, ch.id).await.unwrap().unwrap();
        assert_eq!(
            updated.channel_type(),
            channel::ChannelType::Live,
            "a failed resolve must not flip the channel"
        );
        assert!(
            playlist_item::list_active_for_channel(&pool, ch.id)
                .await
                .unwrap()
                .is_empty(),
            "a failed resolve must not append an item"
        );
    }
}
