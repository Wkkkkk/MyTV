use anyhow::Result;
use serde::{Deserialize, Serialize};
use sqlx::{FromRow, SqlitePool};

/// Source media kind.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash)]
pub enum SourceKind {
    Hls,
    YoutubeLive,
    YoutubeVod,
    Iptv,
    Dash,
}

impl SourceKind {
    pub fn as_str(&self) -> &'static str {
        match self {
            SourceKind::Hls => "hls",
            SourceKind::YoutubeLive => "youtube_live",
            SourceKind::YoutubeVod => "youtube_vod",
            SourceKind::Iptv => "iptv",
            SourceKind::Dash => "dash",
        }
    }

    /// Infers the kind from a URL using the same rules as the discover UI.
    pub fn detect(url: &str) -> Self {
        if url.contains("youtube.com") || url.contains("youtu.be") {
            SourceKind::YoutubeLive
        } else if url.contains(".mpd") {
            SourceKind::Dash
        } else if url.contains(".m3u8") {
            SourceKind::Hls
        } else {
            SourceKind::Iptv
        }
    }
}

impl std::str::FromStr for SourceKind {
    type Err = anyhow::Error;
    fn from_str(s: &str) -> std::result::Result<Self, Self::Err> {
        match s {
            "hls" => Ok(SourceKind::Hls),
            "youtube_live" => Ok(SourceKind::YoutubeLive),
            "youtube_vod" => Ok(SourceKind::YoutubeVod),
            "iptv" => Ok(SourceKind::Iptv),
            "dash" => Ok(SourceKind::Dash),
            _ => anyhow::bail!("invalid source kind: {s}"),
        }
    }
}

impl std::fmt::Display for SourceKind {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        f.write_str(self.as_str())
    }
}

/// A source row as stored in the database.
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

/// Input for creating a new source.
pub struct NewSource {
    pub channel_id: i64,
    pub kind: SourceKind,
    pub url: String,
    pub priority: i64,
}

/// Insert a new source and return it.
pub async fn create(pool: &SqlitePool, input: NewSource) -> Result<Source> {
    let id = sqlx::query(
        "INSERT INTO sources (channel_id, kind, url, priority, is_active) VALUES (?, ?, ?, ?, 1)",
    )
    .bind(input.channel_id)
    .bind(input.kind.as_str())
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

/// Fetch a source by id.
pub async fn get(pool: &SqlitePool, id: i64) -> Result<Option<Source>> {
    sqlx::query_as::<_, Source>("SELECT * FROM sources WHERE id = ?")
        .bind(id)
        .fetch_optional(pool)
        .await
        .map_err(Into::into)
}

/// List all sources for a channel ordered by priority.
pub async fn list_for_channel(pool: &SqlitePool, channel_id: i64) -> Result<Vec<Source>> {
    sqlx::query_as::<_, Source>("SELECT * FROM sources WHERE channel_id = ? ORDER BY priority ASC")
        .bind(channel_id)
        .fetch_all(pool)
        .await
        .map_err(Into::into)
}

/// List only active sources for a channel ordered by priority.
pub async fn list_active_for_channel(pool: &SqlitePool, channel_id: i64) -> Result<Vec<Source>> {
    sqlx::query_as::<_, Source>(
        "SELECT * FROM sources WHERE channel_id = ? AND is_active = 1 ORDER BY priority ASC",
    )
    .bind(channel_id)
    .fetch_all(pool)
    .await
    .map_err(Into::into)
}

/// Sources the tune path may try, ordered by priority: active and not
/// observed-Down. A regular source is Down once `last_status='error'` and
/// `consecutive_failures >= 3`; `youtube_live` sources are exempt (kept in
/// rotation so the resolve-time waiting/backoff for idea #38 can fire). `is_active`
/// is the manual gate and is never mutated by health.
pub async fn list_tunable_for_channel(pool: &SqlitePool, channel_id: i64) -> Result<Vec<Source>> {
    sqlx::query_as::<_, Source>(
        "SELECT * FROM sources \
         WHERE channel_id = ? AND is_active = 1 \
           AND NOT (kind != 'youtube_live' AND last_status = 'error' AND consecutive_failures >= 3) \
         ORDER BY priority ASC",
    )
    .bind(channel_id)
    .fetch_all(pool)
    .await
    .map_err(Into::into)
}

/// Delete a source by id; returns true if a row was removed.
pub async fn delete(pool: &SqlitePool, id: i64) -> Result<bool> {
    let rows = sqlx::query("DELETE FROM sources WHERE id = ?")
        .bind(id)
        .execute(pool)
        .await?
        .rows_affected();
    Ok(rows > 0)
}

/// Set the is_active flag on a source; returns true if a row was updated.
pub async fn set_active(pool: &SqlitePool, id: i64, active: bool) -> Result<bool> {
    let rows = sqlx::query("UPDATE sources SET is_active = ? WHERE id = ?")
        .bind(active)
        .bind(id)
        .execute(pool)
        .await?
        .rows_affected();
    Ok(rows > 0)
}

/// Deactivate every source for a channel. Used when an ended YouTube live is
/// converted to a VOD loop; rows are kept for reference, only is_active flips.
pub async fn deactivate_all_for_channel(pool: &SqlitePool, channel_id: i64) -> Result<()> {
    sqlx::query("UPDATE sources SET is_active = 0 WHERE channel_id = ?")
        .bind(channel_id)
        .execute(pool)
        .await?;
    Ok(())
}

/// List all sources across all channels ordered by channel_id then priority.
pub async fn list_all(pool: &SqlitePool) -> Result<Vec<Source>> {
    sqlx::query_as::<_, Source>("SELECT * FROM sources ORDER BY channel_id ASC, priority ASC")
        .fetch_all(pool)
        .await
        .map_err(Into::into)
}

/// Update health check fields on a source; optionally changes is_active.
pub async fn update_health(
    pool: &SqlitePool,
    id: i64,
    status: &str,
    reason: Option<&str>,
    consecutive_failures: i64,
    is_active: Option<bool>,
) -> Result<()> {
    super::update_health_sql(
        pool,
        "sources",
        id,
        status,
        reason,
        consecutive_failures,
        is_active,
    )
    .await
}

/// Returns the set of channel IDs that have at least one source (active or not).
pub async fn channel_ids_with_any_sources(
    pool: &SqlitePool,
) -> Result<std::collections::HashSet<i64>> {
    sqlx::query_scalar::<_, i64>("SELECT DISTINCT channel_id FROM sources")
        .fetch_all(pool)
        .await
        .map(|v| v.into_iter().collect())
        .map_err(Into::into)
}

/// Returns the set of channel IDs that have at least one active source.
pub async fn channel_ids_with_active_sources(
    pool: &SqlitePool,
) -> Result<std::collections::HashSet<i64>> {
    sqlx::query_scalar::<_, i64>("SELECT DISTINCT channel_id FROM sources WHERE is_active = 1")
        .fetch_all(pool)
        .await
        .map(|v| v.into_iter().collect())
        .map_err(Into::into)
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
                channel_type: channel::ChannelType::Live,
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
            kind: SourceKind::Hls,
            url: url.to_string(),
            priority,
        }
    }

    const FAILURE_DOWN_THRESHOLD: i64 = 3;

    #[tokio::test]
    async fn test_list_tunable_skips_down_regular_keeps_youtube_and_disabled_excluded() {
        let pool = test_pool().await;
        let ch = make_channel(&pool).await;

        // 1: active, healthy HLS → tunable
        let ok = create(&pool, hls(ch.id, "https://ok.example.com/s.m3u8", 1))
            .await
            .unwrap();
        // 2: active HLS but Down past threshold → skipped
        let down = create(&pool, hls(ch.id, "https://down.example.com/s.m3u8", 2))
            .await
            .unwrap();
        update_health(
            &pool,
            down.id,
            "error",
            Some("dead"),
            FAILURE_DOWN_THRESHOLD,
            None,
        )
        .await
        .unwrap();
        // 3: active HLS errored but BELOW threshold → still tunable
        let flaky = create(&pool, hls(ch.id, "https://flaky.example.com/s.m3u8", 3))
            .await
            .unwrap();
        update_health(&pool, flaky.id, "error", Some("blip"), 1, None)
            .await
            .unwrap();
        // 4: youtube_live recorded as error past threshold → KEPT (waiting/#38 lane)
        let yt = create(
            &pool,
            NewSource {
                channel_id: ch.id,
                kind: SourceKind::YoutubeLive,
                url: "https://youtube.com/watch?v=z".into(),
                priority: 4,
            },
        )
        .await
        .unwrap();
        update_health(
            &pool,
            yt.id,
            "error",
            Some("not currently live"),
            FAILURE_DOWN_THRESHOLD,
            None,
        )
        .await
        .unwrap();
        // 5: manually disabled → excluded
        let off = create(&pool, hls(ch.id, "https://off.example.com/s.m3u8", 5))
            .await
            .unwrap();
        set_active(&pool, off.id, false).await.unwrap();
        let _ = ok; // ids referenced below by url

        let tunable = list_tunable_for_channel(&pool, ch.id).await.unwrap();
        let urls: Vec<&str> = tunable.iter().map(|s| s.url.as_str()).collect();
        assert_eq!(
            urls,
            vec![
                "https://ok.example.com/s.m3u8",
                "https://flaky.example.com/s.m3u8",
                "https://youtube.com/watch?v=z",
            ],
            "down regular skipped; below-threshold and youtube_live kept; disabled excluded"
        );
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
    async fn test_deactivate_all_for_channel() {
        let pool = test_pool().await;
        let ch = make_channel(&pool).await;
        create(&pool, hls(ch.id, "https://a.example.com/s.m3u8", 1))
            .await
            .unwrap();
        create(&pool, hls(ch.id, "https://b.example.com/s.m3u8", 2))
            .await
            .unwrap();

        deactivate_all_for_channel(&pool, ch.id).await.unwrap();

        assert!(list_active_for_channel(&pool, ch.id)
            .await
            .unwrap()
            .is_empty());
        assert_eq!(
            list_for_channel(&pool, ch.id).await.unwrap().len(),
            2,
            "rows are kept, only is_active flips"
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

    #[test]
    fn youtube_vod_round_trips() {
        assert_eq!(SourceKind::YoutubeVod.as_str(), "youtube_vod");
        assert_eq!(
            "youtube_vod".parse::<SourceKind>().unwrap(),
            SourceKind::YoutubeVod
        );
    }
}
