use anyhow::Result;
use chrono::{DateTime, NaiveDateTime, Utc};
use serde::{Deserialize, Serialize};
use sqlx::{FromRow, SqlitePool};

use super::IntakeError;

/// A channel row as stored in the database.
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

/// Channel playback mode.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash)]
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

impl ChannelType {
    pub fn as_str(&self) -> &'static str {
        match self {
            ChannelType::Live => "live",
            ChannelType::VodLoop => "vod_loop",
        }
    }
}

impl std::str::FromStr for ChannelType {
    type Err = anyhow::Error;
    fn from_str(s: &str) -> std::result::Result<Self, Self::Err> {
        match s {
            "live" => Ok(ChannelType::Live),
            "vod_loop" => Ok(ChannelType::VodLoop),
            _ => anyhow::bail!("invalid channel_type: {s}"),
        }
    }
}

impl std::fmt::Display for ChannelType {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        f.write_str(self.as_str())
    }
}

/// Input for creating a new channel.
pub struct NewChannel {
    pub name: String,
    pub category: String,
    pub logo_url: Option<String>,
    pub channel_type: ChannelType,
    pub sort_order: i64,
    pub loop_anchor: Option<DateTime<Utc>>,
}

/// Insert a new channel and return it.
pub async fn create(pool: &SqlitePool, input: NewChannel) -> Result<Channel> {
    let id = sqlx::query(
        "INSERT INTO channels (name, category, logo_url, type, sort_order, loop_anchor)
         VALUES (?, ?, ?, ?, ?, ?)",
    )
    .bind(&input.name)
    .bind(&input.category)
    .bind(&input.logo_url)
    .bind(input.channel_type.as_str())
    .bind(input.sort_order)
    .bind(input.loop_anchor)
    .execute(pool)
    .await?
    .last_insert_rowid();

    get(pool, id)
        .await?
        .ok_or_else(|| anyhow::anyhow!("channel not found after insert"))
}

/// Input for updating an existing channel.
#[derive(Debug)]
pub struct UpdateChannel {
    pub name: String,
    pub category: String,
    pub logo_url: Option<String>,
    pub channel_type: ChannelType,
    pub sort_order: i64,
    pub loop_anchor: Option<DateTime<Utc>>,
}

/// Update a channel by id; returns None if not found.
pub async fn update(pool: &SqlitePool, id: i64, input: UpdateChannel) -> Result<Option<Channel>> {
    let rows = sqlx::query(
        "UPDATE channels SET name = ?, category = ?, logo_url = ?, type = ?, sort_order = ?, loop_anchor = ? WHERE id = ?",
    )
    .bind(&input.name)
    .bind(&input.category)
    .bind(&input.logo_url)
    .bind(input.channel_type.as_str())
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

/// Set a channel's playback type and loop anchor. Used by the ended-live → VOD
/// conversion to flip a `live` channel into a `vod_loop`.
pub async fn set_type_and_anchor(
    pool: &SqlitePool,
    id: i64,
    channel_type: ChannelType,
    loop_anchor: Option<DateTime<Utc>>,
) -> Result<()> {
    sqlx::query("UPDATE channels SET type = ?, loop_anchor = ? WHERE id = ?")
        .bind(channel_type.as_str())
        .bind(loop_anchor)
        .bind(id)
        .execute(pool)
        .await?;
    Ok(())
}

/// Atomically flip a channel from `live` to `vod_loop`, setting the loop anchor.
/// Returns `true` only for the caller that performs the transition; a channel
/// that is already `vod_loop` (a concurrent or repeat conversion) yields
/// `false` and is left untouched. SQLite serializes the conditional `UPDATE`,
/// so this is the gate that keeps an ended-live channel — and its recording —
/// from being converted more than once.
pub async fn set_type_and_anchor_if_live(
    pool: &SqlitePool,
    id: i64,
    loop_anchor: DateTime<Utc>,
) -> Result<bool> {
    let rows = sqlx::query(
        "UPDATE channels SET type = 'vod_loop', loop_anchor = ? WHERE id = ? AND type = 'live'",
    )
    .bind(loop_anchor)
    .bind(id)
    .execute(pool)
    .await?
    .rows_affected();
    Ok(rows == 1)
}

/// Fetch a channel by id.
pub async fn get(pool: &SqlitePool, id: i64) -> Result<Option<Channel>> {
    sqlx::query_as::<_, Channel>("SELECT * FROM channels WHERE id = ?")
        .bind(id)
        .fetch_optional(pool)
        .await
        .map_err(Into::into)
}

/// List all channels ordered by sort_order.
pub async fn list(pool: &SqlitePool) -> Result<Vec<Channel>> {
    sqlx::query_as::<_, Channel>("SELECT * FROM channels ORDER BY sort_order ASC, name ASC")
        .fetch_all(pool)
        .await
        .map_err(Into::into)
}

/// Delete a channel by id; returns true if a row was removed.
pub async fn delete(pool: &SqlitePool, id: i64) -> Result<bool> {
    let rows = sqlx::query("DELETE FROM channels WHERE id = ?")
        .bind(id)
        .execute(pool)
        .await?
        .rows_affected();
    Ok(rows > 0)
}

/// Return sorted, deduplicated category names from a channel list.
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

/// Raw, transport-decoded channel fields awaiting validation.
/// `sort_order` is already an `i64` (form adapter parses its string field first).
pub struct ChannelInput {
    pub name: String,
    pub category: String,
    pub channel_type: String,
    pub sort_order: i64,
    pub logo_url: Option<String>,
    pub loop_anchor: Option<String>,
}

fn parse_loop_anchor(s: &str) -> Option<DateTime<Utc>> {
    let trimmed = s.trim();
    if trimmed.is_empty() {
        return None;
    }
    NaiveDateTime::parse_from_str(trimmed, "%Y-%m-%dT%H:%M")
        .ok()
        .map(|dt| DateTime::from_naive_utc_and_offset(dt, Utc))
}

fn normalize_logo(logo: Option<String>) -> Option<String> {
    logo.map(|s| s.trim().to_string()).filter(|s| !s.is_empty())
}

fn validate_names(name: String, category: String) -> Result<(String, String), IntakeError> {
    let name = name.trim();
    let category = category.trim();
    if name.is_empty() || category.is_empty() {
        return Err(IntakeError("name and category are required".into()));
    }
    Ok((name.to_string(), category.to_string()))
}

fn resolve_anchor(
    channel_type: ChannelType,
    raw: Option<&str>,
    existing: Option<DateTime<Utc>>,
) -> Option<DateTime<Utc>> {
    if channel_type == ChannelType::VodLoop {
        raw.and_then(parse_loop_anchor)
            .or(existing)
            .or_else(|| Some(Utc::now()))
    } else {
        None
    }
}

impl ChannelInput {
    pub fn validate_new(self) -> Result<NewChannel, IntakeError> {
        let channel_type = self
            .channel_type
            .parse::<ChannelType>()
            .map_err(|_| IntakeError(format!("invalid channel type: {}", self.channel_type)))?;
        let (name, category) = validate_names(self.name, self.category)?;
        let loop_anchor = resolve_anchor(channel_type, self.loop_anchor.as_deref(), None);
        Ok(NewChannel {
            name,
            category,
            logo_url: normalize_logo(self.logo_url),
            channel_type,
            sort_order: self.sort_order,
            loop_anchor,
        })
    }

    pub fn validate_update(
        self,
        existing_anchor: Option<DateTime<Utc>>,
    ) -> Result<UpdateChannel, IntakeError> {
        let channel_type = self
            .channel_type
            .parse::<ChannelType>()
            .map_err(|_| IntakeError(format!("invalid channel type: {}", self.channel_type)))?;
        let (name, category) = validate_names(self.name, self.category)?;
        let loop_anchor =
            resolve_anchor(channel_type, self.loop_anchor.as_deref(), existing_anchor);
        Ok(UpdateChannel {
            name,
            category,
            logo_url: normalize_logo(self.logo_url),
            channel_type,
            sort_order: self.sort_order,
            loop_anchor,
        })
    }
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
            channel_type: ChannelType::Live,
            sort_order: 0,
            loop_anchor: None,
        }
    }

    #[tokio::test]
    async fn test_create_and_get_channel() {
        let pool = test_pool().await;

        let ch = create(&pool, live("CNN International", "news"))
            .await
            .unwrap();

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
                channel_type: ChannelType::Live,
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
                channel_type: ChannelType::Live,
                sort_order: 0,
                loop_anchor: None,
            },
        )
        .await
        .unwrap();
        assert!(result.is_none());
    }

    #[tokio::test]
    async fn test_set_type_and_anchor_flips_to_vod_loop() {
        use chrono::TimeZone;
        let pool = test_pool().await;
        let ch = create(
            &pool,
            NewChannel {
                name: "X".into(),
                category: "c".into(),
                logo_url: None,
                channel_type: ChannelType::Live,
                sort_order: 0,
                loop_anchor: None,
            },
        )
        .await
        .unwrap();
        assert_eq!(ch.channel_type(), ChannelType::Live);

        let anchor = Utc.timestamp_opt(1_700_000_000, 0).unwrap();
        set_type_and_anchor(&pool, ch.id, ChannelType::VodLoop, Some(anchor))
            .await
            .unwrap();

        let updated = get(&pool, ch.id).await.unwrap().unwrap();
        assert_eq!(updated.channel_type(), ChannelType::VodLoop);
        assert_eq!(updated.loop_anchor, Some(anchor));
    }

    #[tokio::test]
    async fn test_set_type_and_anchor_if_live_claims_exactly_once() {
        use chrono::TimeZone;
        let pool = test_pool().await;
        let ch = create(&pool, live("Race", "c")).await.unwrap();

        let anchor = Utc.timestamp_opt(1_700_000_000, 0).unwrap();
        assert!(
            set_type_and_anchor_if_live(&pool, ch.id, anchor)
                .await
                .unwrap(),
            "first claim on a live channel must win"
        );
        let after = get(&pool, ch.id).await.unwrap().unwrap();
        assert_eq!(after.channel_type(), ChannelType::VodLoop);
        assert_eq!(after.loop_anchor, Some(anchor));

        let later = Utc.timestamp_opt(1_800_000_000, 0).unwrap();
        assert!(
            !set_type_and_anchor_if_live(&pool, ch.id, later)
                .await
                .unwrap(),
            "second claim on an already-converted channel must lose"
        );
        let unchanged = get(&pool, ch.id).await.unwrap().unwrap();
        assert_eq!(
            unchanged.loop_anchor,
            Some(anchor),
            "a lost claim must not move the anchor"
        );
    }

    #[tokio::test]
    async fn test_set_type_and_anchor_if_live_missing_channel_is_false() {
        use chrono::TimeZone;
        let pool = test_pool().await;
        let anchor = Utc.timestamp_opt(1_700_000_000, 0).unwrap();
        assert!(!set_type_and_anchor_if_live(&pool, 9999, anchor)
            .await
            .unwrap());
    }

    #[test]
    fn validate_new_live_trims_and_drops_anchor() {
        let new = ChannelInput {
            name: "  CNN  ".into(),
            category: " news ".into(),
            channel_type: "live".into(),
            sort_order: 3,
            logo_url: Some("  ".into()),
            loop_anchor: Some("2021-01-01T00:00".into()),
        }
        .validate_new()
        .unwrap();
        assert_eq!(new.name, "CNN");
        assert_eq!(new.category, "news");
        assert_eq!(new.channel_type, ChannelType::Live);
        assert_eq!(new.sort_order, 3);
        assert_eq!(new.logo_url, None);
        assert_eq!(new.loop_anchor, None);
    }

    #[test]
    fn validate_new_rejects_empty_name_and_bad_type() {
        let bad_name = ChannelInput {
            name: "   ".into(),
            category: "news".into(),
            channel_type: "live".into(),
            sort_order: 0,
            logo_url: None,
            loop_anchor: None,
        }
        .validate_new();
        assert!(bad_name.is_err());

        let bad_type = ChannelInput {
            name: "CNN".into(),
            category: "news".into(),
            channel_type: "bogus".into(),
            sort_order: 0,
            logo_url: None,
            loop_anchor: None,
        }
        .validate_new();
        assert!(bad_type.is_err());
    }

    #[test]
    fn validate_new_vod_parses_explicit_anchor() {
        let new = ChannelInput {
            name: "VOD".into(),
            category: "movies".into(),
            channel_type: "vod_loop".into(),
            sort_order: 0,
            logo_url: None,
            loop_anchor: Some("2021-05-05T10:00".into()),
        }
        .validate_new()
        .unwrap();
        let anchor = new.loop_anchor.expect("vod_loop must have an anchor");
        assert_eq!(
            anchor.format("%Y-%m-%dT%H:%M").to_string(),
            "2021-05-05T10:00"
        );
    }

    #[test]
    fn validate_new_vod_defaults_anchor_to_now_when_blank() {
        let new = ChannelInput {
            name: "VOD".into(),
            category: "movies".into(),
            channel_type: "vod_loop".into(),
            sort_order: 0,
            logo_url: None,
            loop_anchor: Some("".into()),
        }
        .validate_new()
        .unwrap();
        assert!(new.loop_anchor.is_some());
    }

    #[test]
    fn validate_update_prefers_existing_anchor_when_blank() {
        let existing = DateTime::from_naive_utc_and_offset(
            NaiveDateTime::parse_from_str("2020-02-02T08:00", "%Y-%m-%dT%H:%M").unwrap(),
            Utc,
        );
        let upd = ChannelInput {
            name: "VOD".into(),
            category: "movies".into(),
            channel_type: "vod_loop".into(),
            sort_order: 0,
            logo_url: None,
            loop_anchor: None,
        }
        .validate_update(Some(existing))
        .unwrap();
        assert_eq!(upd.loop_anchor, Some(existing));
    }
}
