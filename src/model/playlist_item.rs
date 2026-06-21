use anyhow::Result;
use serde::{Deserialize, Serialize};
use sqlx::{FromRow, SqlitePool};

use super::IntakeError;

/// A playlist item row as stored in the database.
#[derive(Debug, Clone, Serialize, Deserialize, FromRow)]
pub struct PlaylistItem {
    pub id: i64,
    pub channel_id: i64,
    pub title: String,
    pub url: String,
    pub duration_secs: i64,
    pub sort_order: i64,
    pub is_active: bool,
    pub last_checked_at: Option<i64>,
    pub last_status: Option<String>,
    pub consecutive_failures: i64,
    pub failure_reason: Option<String>,
    /// Unix seconds when the item transitioned active→disabled; NULL while active.
    /// Drives the stale-disabled reaper (see `health::reap_stale_disabled_items`).
    pub disabled_at: Option<i64>,
}

/// Input for creating a new playlist item.
pub struct NewPlaylistItem {
    pub channel_id: i64,
    pub title: String,
    pub url: String,
    pub duration_secs: i64,
    pub sort_order: i64,
}

/// Insert a new playlist item and return it.
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

/// The sort_order that appends an item at the end of a channel's playlist:
/// `max(sort_order) + 1`, or 0 when the channel has no items yet. Callers that
/// don't pin an explicit position use this so items keep a sequential number
/// instead of all collapsing onto the default 0.
pub async fn next_sort_order(pool: &SqlitePool, channel_id: i64) -> Result<i64> {
    let max: Option<i64> =
        sqlx::query_scalar("SELECT MAX(sort_order) FROM playlist_items WHERE channel_id = ?")
            .bind(channel_id)
            .fetch_one(pool)
            .await?;
    Ok(max.map(|m| m + 1).unwrap_or(0))
}

/// Find an existing item on the channel that already has this `url` *or* `title`
/// — the dedup key. Returns the earliest match (by sort_order, then id) so
/// repeated appends of the same recording become idempotent. `None` means the
/// item is new to the channel.
pub async fn find_duplicate(
    pool: &SqlitePool,
    channel_id: i64,
    url: &str,
    title: &str,
) -> Result<Option<PlaylistItem>> {
    sqlx::query_as::<_, PlaylistItem>(
        "SELECT * FROM playlist_items
         WHERE channel_id = ? AND (url = ? OR title = ?)
         ORDER BY sort_order ASC, id ASC
         LIMIT 1",
    )
    .bind(channel_id)
    .bind(url)
    .bind(title)
    .fetch_optional(pool)
    .await
    .map_err(Into::into)
}

/// Fetch a playlist item by id.
pub async fn get(pool: &SqlitePool, id: i64) -> Result<Option<PlaylistItem>> {
    sqlx::query_as::<_, PlaylistItem>("SELECT * FROM playlist_items WHERE id = ?")
        .bind(id)
        .fetch_optional(pool)
        .await
        .map_err(Into::into)
}

/// List all playlist items for a channel ordered by sort_order.
pub async fn list_for_channel(pool: &SqlitePool, channel_id: i64) -> Result<Vec<PlaylistItem>> {
    sqlx::query_as::<_, PlaylistItem>(
        "SELECT * FROM playlist_items WHERE channel_id = ? ORDER BY sort_order ASC, id ASC",
    )
    .bind(channel_id)
    .fetch_all(pool)
    .await
    .map_err(Into::into)
}

/// List all playlist items across all channels ordered by channel_id then sort_order.
pub async fn list_all(pool: &SqlitePool) -> Result<Vec<PlaylistItem>> {
    sqlx::query_as::<_, PlaylistItem>(
        "SELECT * FROM playlist_items ORDER BY channel_id, sort_order ASC, id ASC",
    )
    .fetch_all(pool)
    .await
    .map_err(Into::into)
}

/// Delete a playlist item by id; returns true if a row was removed.
pub async fn delete(pool: &SqlitePool, id: i64) -> Result<bool> {
    let rows = sqlx::query("DELETE FROM playlist_items WHERE id = ?")
        .bind(id)
        .execute(pool)
        .await?
        .rows_affected();
    Ok(rows > 0)
}

/// List only active items for a channel ordered by sort_order.
pub async fn list_active_for_channel(
    pool: &SqlitePool,
    channel_id: i64,
) -> Result<Vec<PlaylistItem>> {
    sqlx::query_as::<_, PlaylistItem>(
        "SELECT * FROM playlist_items WHERE channel_id = ? AND is_active = 1 ORDER BY sort_order ASC, id ASC",
    )
    .bind(channel_id)
    .fetch_all(pool)
    .await
    .map_err(Into::into)
}

/// Set the is_active flag on a playlist item; returns true if a row was updated.
/// Maintains the reap clock invariant: disabling stamps `disabled_at` (preserving
/// any existing stamp, so a re-disable never resets the clock); enabling clears it.
pub async fn set_active(pool: &SqlitePool, id: i64, active: bool) -> Result<bool> {
    let rows = sqlx::query(
        "UPDATE playlist_items
         SET is_active = ?,
             disabled_at = CASE WHEN ? THEN NULL
                                ELSE COALESCE(disabled_at, strftime('%s','now')) END
         WHERE id = ?",
    )
    .bind(active)
    .bind(active)
    .bind(id)
    .execute(pool)
    .await?
    .rows_affected();
    Ok(rows > 0)
}

/// Input for updating an existing playlist item.
pub struct UpdatePlaylistItem {
    pub title: String,
    pub url: String,
    pub duration_secs: i64,
    pub sort_order: i64,
}

/// Update a playlist item by id; returns None if not found.
pub async fn update(
    pool: &SqlitePool,
    id: i64,
    input: UpdatePlaylistItem,
) -> Result<Option<PlaylistItem>> {
    let rows = sqlx::query(
        "UPDATE playlist_items SET title = ?, url = ?, duration_secs = ?, sort_order = ? WHERE id = ?",
    )
    .bind(&input.title)
    .bind(&input.url)
    .bind(input.duration_secs)
    .bind(input.sort_order)
    .bind(id)
    .execute(pool)
    .await?
    .rows_affected();
    if rows == 0 {
        return Ok(None);
    }
    get(pool, id).await
}

/// Update health check fields on a playlist item; optionally changes is_active.
pub async fn update_health(
    pool: &SqlitePool,
    id: i64,
    status: &str,
    reason: Option<&str>,
    consecutive_failures: i64,
    is_active: Option<bool>,
) -> Result<()> {
    if let Some(active) = is_active {
        // Maintain the reap clock invariant alongside is_active (see `set_active`).
        sqlx::query(
            "UPDATE playlist_items
             SET last_checked_at = strftime('%s','now'),
                 last_status = ?,
                 failure_reason = ?,
                 consecutive_failures = ?,
                 is_active = ?,
                 disabled_at = CASE WHEN ? THEN NULL
                                    ELSE COALESCE(disabled_at, strftime('%s','now')) END
             WHERE id = ?",
        )
        .bind(status)
        .bind(reason)
        .bind(consecutive_failures)
        .bind(active)
        .bind(active)
        .bind(id)
        .execute(pool)
        .await?;
    } else {
        sqlx::query(
            "UPDATE playlist_items
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

/// Hard-delete playlist items that have stayed disabled longer than
/// `older_than_secs`, regardless of how they were disabled (auto via health, or a
/// manual admin toggle). Returns the `(id, title)` of each reaped item so the
/// caller can log what vanished. Active items (`disabled_at IS NULL`) and items
/// disabled more recently are left untouched.
pub async fn reap_stale_disabled(
    pool: &SqlitePool,
    older_than_secs: i64,
) -> Result<Vec<(i64, String)>> {
    sqlx::query_as::<_, (i64, String)>(
        "DELETE FROM playlist_items
         WHERE is_active = 0
           AND disabled_at IS NOT NULL
           AND disabled_at < strftime('%s','now') - ?
         RETURNING id, title",
    )
    .bind(older_than_secs)
    .fetch_all(pool)
    .await
    .map_err(Into::into)
}

/// An on-demand/VOD playlist item is "dead" when its last health probe errored
/// and it has failed at least `source::FAILURE_THRESHOLD` consecutive times.
/// Unlike `source::is_observed_down`, there is no `youtube_live` exemption: a
/// playlist item is never a live broadcast, so an errored item past threshold is
/// always dead (a deleted R2 object never recovers on its own).
pub fn is_dead(last_status: Option<&str>, consecutive_failures: i64) -> bool {
    last_status == Some("error") && consecutive_failures >= crate::model::source::FAILURE_THRESHOLD
}

/// Records one health-probe result against an item and applies the auto-disable
/// rule. This is the single owner of the disable decision: both the background
/// health loop and the interactive tune path call it, so the rule lives in one
/// place even with two writers.
///
/// - `ok == true` resets failures (status "ok"); never re-enables — re-enabling
///   a disabled item is a manual admin action. `reason` is recorded only on
///   failure; it is ignored when `ok == true`.
/// - `ok == false` counts a failure (status "error"); disables once `is_dead`.
///
/// Returns `ok` for the caller's convenience.
pub async fn apply_health_result(
    pool: &SqlitePool,
    item: &PlaylistItem,
    ok: bool,
    reason: Option<&str>,
) -> Result<bool> {
    let reason = if ok { None } else { reason };
    let new_failures = if ok { 0 } else { item.consecutive_failures + 1 };
    let status = if ok { "ok" } else { "error" };
    let is_active = if is_dead(Some(status), new_failures) {
        Some(false)
    } else {
        None
    };
    update_health(pool, item.id, status, reason, new_failures, is_active).await?;
    Ok(ok)
}

/// Raw, transport-decoded playlist-item fields awaiting validation.
/// `duration_secs` and `sort_order` are already resolved by the adapter
/// (the form auto-fetches duration and derives sort_order from the DB max).
pub struct PlaylistInput {
    pub title: String,
    pub url: String,
    pub duration_secs: i64,
    pub sort_order: i64,
}

fn validate_title_url(title: String, url: String) -> Result<(String, String), IntakeError> {
    let title = title.trim();
    let url = url.trim();
    if title.is_empty() || url.is_empty() {
        return Err(IntakeError("title and url are required".into()));
    }
    Ok((title.to_string(), url.to_string()))
}

impl PlaylistInput {
    pub fn validate_new(self, channel_id: i64) -> Result<NewPlaylistItem, IntakeError> {
        let (title, url) = validate_title_url(self.title, self.url)?;
        if self.duration_secs <= 0 {
            return Err(IntakeError("duration_secs must be > 0".into()));
        }
        Ok(NewPlaylistItem {
            channel_id,
            title,
            url,
            duration_secs: self.duration_secs,
            sort_order: self.sort_order,
        })
    }

    pub fn validate_update(self) -> Result<UpdatePlaylistItem, IntakeError> {
        let (title, url) = validate_title_url(self.title, self.url)?;
        if self.duration_secs <= 0 {
            return Err(IntakeError("duration_secs must be > 0".into()));
        }
        Ok(UpdatePlaylistItem {
            title,
            url,
            duration_secs: self.duration_secs,
            sort_order: self.sort_order,
        })
    }
}

/// Sum the duration of all items in the playlist.
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
    unreachable!(
        "elapsed ({elapsed}) < total ({total}) guaranteed by rem_euclid, \
         but for-loop found no matching item"
    )
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
                name: "VOD Loop".to_string(),
                category: "test".to_string(),
                logo_url: None,
                channel_type: channel::ChannelType::VodLoop,
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
    async fn list_active_ties_break_by_insertion_order() {
        // All same sort_order (as the admin add-item form leaves them): order
        // must fall back to id (insertion / add-time), deterministically.
        let pool = test_pool().await;
        let ch = make_channel(&pool).await;
        let a = create(&pool, item(ch.id, "first", 10, 0)).await.unwrap();
        let b = create(&pool, item(ch.id, "second", 10, 0)).await.unwrap();
        let c = create(&pool, item(ch.id, "third", 10, 0)).await.unwrap();

        let items = list_active_for_channel(&pool, ch.id).await.unwrap();
        let ids: Vec<i64> = items.iter().map(|i| i.id).collect();
        assert_eq!(ids, vec![a.id, b.id, c.id]);
    }

    #[tokio::test]
    async fn next_sort_order_appends_after_max() {
        let pool = test_pool().await;
        let ch = make_channel(&pool).await;
        assert_eq!(
            next_sort_order(&pool, ch.id).await.unwrap(),
            0,
            "empty channel starts at 0"
        );

        create(&pool, item(ch.id, "a", 10, 0)).await.unwrap();
        create(&pool, item(ch.id, "b", 10, 5)).await.unwrap();
        assert_eq!(
            next_sort_order(&pool, ch.id).await.unwrap(),
            6,
            "appends after the current max"
        );
    }

    #[tokio::test]
    async fn find_duplicate_matches_url_or_title_scoped_to_channel() {
        let pool = test_pool().await;
        let ch = make_channel(&pool).await;
        let other = make_channel(&pool).await;
        let it = create(&pool, item(ch.id, "Ep 1", 10, 0)).await.unwrap();

        // Same URL → match.
        assert_eq!(
            find_duplicate(&pool, ch.id, &it.url, "different title")
                .await
                .unwrap()
                .map(|x| x.id),
            Some(it.id)
        );
        // Same title, different URL → still a match (re-encoded duplicate).
        assert_eq!(
            find_duplicate(&pool, ch.id, "https://example.com/other.mp4", "Ep 1")
                .await
                .unwrap()
                .map(|x| x.id),
            Some(it.id)
        );
        // Neither matches → None.
        assert!(
            find_duplicate(&pool, ch.id, "https://example.com/new.mp4", "New")
                .await
                .unwrap()
                .is_none()
        );
        // A match on another channel must not leak across channels.
        assert!(find_duplicate(&pool, other.id, &it.url, "Ep 1")
            .await
            .unwrap()
            .is_none());
    }

    #[tokio::test]
    async fn test_current_position_within_first_item() {
        let items = vec![
            PlaylistItem {
                id: 1,
                channel_id: 1,
                title: "A".into(),
                url: "u".into(),
                duration_secs: 3600,
                sort_order: 0,
                is_active: true,
                last_checked_at: None,
                last_status: None,
                consecutive_failures: 0,
                failure_reason: None,
                disabled_at: None,
            },
            PlaylistItem {
                id: 2,
                channel_id: 1,
                title: "B".into(),
                url: "u".into(),
                duration_secs: 1800,
                sort_order: 1,
                is_active: true,
                last_checked_at: None,
                last_status: None,
                consecutive_failures: 0,
                failure_reason: None,
                disabled_at: None,
            },
        ];
        // 500 seconds into the loop — still in item A
        let (idx, offset) = current_position(&items, 1500, 1000).unwrap();
        assert_eq!(idx, 0);
        assert_eq!(offset, 500);
    }

    #[tokio::test]
    async fn test_current_position_within_second_item() {
        let items = vec![
            PlaylistItem {
                id: 1,
                channel_id: 1,
                title: "A".into(),
                url: "u".into(),
                duration_secs: 3600,
                sort_order: 0,
                is_active: true,
                last_checked_at: None,
                last_status: None,
                consecutive_failures: 0,
                failure_reason: None,
                disabled_at: None,
            },
            PlaylistItem {
                id: 2,
                channel_id: 1,
                title: "B".into(),
                url: "u".into(),
                duration_secs: 1800,
                sort_order: 1,
                is_active: true,
                last_checked_at: None,
                last_status: None,
                consecutive_failures: 0,
                failure_reason: None,
                disabled_at: None,
            },
        ];
        // 4000 seconds in — 400 seconds into item B (after A's 3600)
        let (idx, offset) = current_position(&items, 4000, 0).unwrap();
        assert_eq!(idx, 1);
        assert_eq!(offset, 400);
    }

    #[tokio::test]
    async fn test_current_position_wraps_around_to_start() {
        let items = vec![
            PlaylistItem {
                id: 1,
                channel_id: 1,
                title: "A".into(),
                url: "u".into(),
                duration_secs: 3600,
                sort_order: 0,
                is_active: true,
                last_checked_at: None,
                last_status: None,
                consecutive_failures: 0,
                failure_reason: None,
                disabled_at: None,
            },
            PlaylistItem {
                id: 2,
                channel_id: 1,
                title: "B".into(),
                url: "u".into(),
                duration_secs: 1800,
                sort_order: 1,
                is_active: true,
                last_checked_at: None,
                last_status: None,
                consecutive_failures: 0,
                failure_reason: None,
                disabled_at: None,
            },
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
    async fn test_list_all_returns_items_across_channels() {
        let pool = test_pool().await;
        let ch = make_channel(&pool).await;
        create(&pool, item(ch.id, "ep1", 1800, 0)).await.unwrap();
        create(&pool, item(ch.id, "ep2", 2400, 1)).await.unwrap();

        let all = list_all(&pool).await.unwrap();
        assert_eq!(all.len(), 2);
    }

    #[tokio::test]
    async fn test_playlist_items_deleted_when_channel_is_deleted() {
        let pool = test_pool().await;
        let ch = make_channel(&pool).await;

        create(&pool, item(ch.id, "ep1", 1800, 0)).await.unwrap();

        channel::delete(&pool, ch.id).await.unwrap();

        let items = list_for_channel(&pool, ch.id).await.unwrap();
        assert!(
            items.is_empty(),
            "ON DELETE CASCADE should remove playlist items"
        );
    }

    #[tokio::test]
    async fn test_list_active_excludes_inactive_items() {
        let pool = test_pool().await;
        let ch = make_channel(&pool).await;

        let first = create(&pool, item(ch.id, "ep1", 1800, 0)).await.unwrap();
        create(&pool, item(ch.id, "ep2", 2400, 1)).await.unwrap();

        set_active(&pool, first.id, false).await.unwrap();

        let active = list_active_for_channel(&pool, ch.id).await.unwrap();
        assert_eq!(active.len(), 1);
        assert_eq!(active[0].title, "ep2");
    }

    #[tokio::test]
    async fn set_active_stamps_and_clears_disabled_at() {
        let pool = test_pool().await;
        let ch = make_channel(&pool).await;
        let it = create(&pool, item(ch.id, "ep1", 1800, 0)).await.unwrap();
        assert!(
            it.disabled_at.is_none(),
            "active item starts with NULL clock"
        );

        set_active(&pool, it.id, false).await.unwrap();
        let disabled = get(&pool, it.id).await.unwrap().unwrap();
        assert!(
            disabled.disabled_at.is_some(),
            "disabling stamps disabled_at"
        );

        set_active(&pool, it.id, true).await.unwrap();
        let reenabled = get(&pool, it.id).await.unwrap().unwrap();
        assert!(
            reenabled.disabled_at.is_none(),
            "re-enabling clears the clock"
        );
    }

    #[tokio::test]
    async fn set_active_false_preserves_original_disabled_at() {
        // Re-disabling an already-disabled item must NOT reset the clock — otherwise
        // a repeatedly-failing item's reap clock would never elapse.
        let pool = test_pool().await;
        let ch = make_channel(&pool).await;
        let it = create(&pool, item(ch.id, "ep1", 1800, 0)).await.unwrap();

        set_active(&pool, it.id, false).await.unwrap();
        let first = get(&pool, it.id).await.unwrap().unwrap().disabled_at;
        // Backdate so a re-stamp (if it wrongly happened) would be observably different.
        sqlx::query("UPDATE playlist_items SET disabled_at = 1000 WHERE id = ?")
            .bind(it.id)
            .execute(&pool)
            .await
            .unwrap();
        assert!(first.is_some());

        set_active(&pool, it.id, false).await.unwrap();
        let second = get(&pool, it.id).await.unwrap().unwrap().disabled_at;
        assert_eq!(second, Some(1000), "re-disable must preserve the clock");
    }

    #[tokio::test]
    async fn test_set_active_toggles_item() {
        let pool = test_pool().await;
        let ch = make_channel(&pool).await;

        let it = create(&pool, item(ch.id, "ep1", 1800, 0)).await.unwrap();
        assert!(it.is_active);

        set_active(&pool, it.id, false).await.unwrap();
        assert!(list_active_for_channel(&pool, ch.id)
            .await
            .unwrap()
            .is_empty());

        set_active(&pool, it.id, true).await.unwrap();
        assert_eq!(
            list_active_for_channel(&pool, ch.id).await.unwrap().len(),
            1
        );
    }

    #[tokio::test]
    async fn test_update_health_ok_resets_failures() {
        let pool = test_pool().await;
        let ch = make_channel(&pool).await;
        let it = create(&pool, item(ch.id, "ep1", 1800, 0)).await.unwrap();

        update_health(&pool, it.id, "error", Some("timeout"), 2, None)
            .await
            .unwrap();
        update_health(&pool, it.id, "ok", None, 0, None)
            .await
            .unwrap();

        let updated = get(&pool, it.id).await.unwrap().unwrap();
        assert_eq!(updated.last_status.as_deref(), Some("ok"));
        assert_eq!(updated.consecutive_failures, 0);
        assert!(updated.failure_reason.is_none());
        assert!(updated.is_active);
    }

    #[tokio::test]
    async fn test_update_health_disables_after_threshold() {
        let pool = test_pool().await;
        let ch = make_channel(&pool).await;
        let it = create(&pool, item(ch.id, "ep1", 1800, 0)).await.unwrap();

        update_health(
            &pool,
            it.id,
            "error",
            Some("connection refused"),
            3,
            Some(false),
        )
        .await
        .unwrap();

        let updated = get(&pool, it.id).await.unwrap().unwrap();
        assert!(!updated.is_active);
        assert_eq!(updated.consecutive_failures, 3);
        assert_eq!(updated.last_status.as_deref(), Some("error"));
        assert_eq!(
            updated.failure_reason.as_deref(),
            Some("connection refused")
        );
    }

    #[tokio::test]
    async fn update_health_maintains_disabled_at_clock() {
        let pool = test_pool().await;
        let ch = make_channel(&pool).await;
        let it = create(&pool, item(ch.id, "ep1", 1800, 0)).await.unwrap();

        // is_active = None must NOT touch the clock.
        update_health(&pool, it.id, "error", Some("timeout"), 1, None)
            .await
            .unwrap();
        assert!(
            get(&pool, it.id)
                .await
                .unwrap()
                .unwrap()
                .disabled_at
                .is_none(),
            "a health update that doesn't disable must leave the clock NULL"
        );

        // Disabling via the health path stamps the clock.
        update_health(&pool, it.id, "error", Some("dead"), 3, Some(false))
            .await
            .unwrap();
        assert!(
            get(&pool, it.id)
                .await
                .unwrap()
                .unwrap()
                .disabled_at
                .is_some(),
            "disabling via update_health stamps disabled_at"
        );

        // Re-disable must preserve the original stamp.
        sqlx::query("UPDATE playlist_items SET disabled_at = 1000 WHERE id = ?")
            .bind(it.id)
            .execute(&pool)
            .await
            .unwrap();
        update_health(&pool, it.id, "error", Some("dead"), 4, Some(false))
            .await
            .unwrap();
        assert_eq!(
            get(&pool, it.id).await.unwrap().unwrap().disabled_at,
            Some(1000),
            "repeated disable must not reset the clock"
        );

        // Re-enabling clears the clock.
        update_health(&pool, it.id, "ok", None, 0, Some(true))
            .await
            .unwrap();
        assert!(
            get(&pool, it.id)
                .await
                .unwrap()
                .unwrap()
                .disabled_at
                .is_none(),
            "re-enabling clears the clock"
        );
    }

    #[tokio::test]
    async fn test_update_health_reenables_disabled_item() {
        let pool = test_pool().await;
        let ch = make_channel(&pool).await;
        let it = create(&pool, item(ch.id, "ep1", 1800, 0)).await.unwrap();

        update_health(&pool, it.id, "error", Some("timeout"), 3, Some(false))
            .await
            .unwrap();
        assert!(!get(&pool, it.id).await.unwrap().unwrap().is_active);

        update_health(&pool, it.id, "ok", None, 0, Some(true))
            .await
            .unwrap();
        let reenabled = get(&pool, it.id).await.unwrap().unwrap();
        assert!(reenabled.is_active);
        assert_eq!(reenabled.consecutive_failures, 0);
        assert_eq!(reenabled.last_status.as_deref(), Some("ok"));
        assert!(reenabled.failure_reason.is_none());
    }

    #[tokio::test]
    async fn update_changes_fields() {
        let pool = test_pool().await;
        let ch = make_channel(&pool).await;
        let it = create(&pool, item(ch.id, "ep1", 1800, 0)).await.unwrap();

        let updated = update(
            &pool,
            it.id,
            UpdatePlaylistItem {
                title: "Renamed".into(),
                url: "https://vod.example.com/new.mp4".into(),
                duration_secs: 123,
                sort_order: 7,
            },
        )
        .await
        .unwrap()
        .expect("item exists");
        assert_eq!(updated.title, "Renamed");
        assert_eq!(updated.url, "https://vod.example.com/new.mp4");
        assert_eq!(updated.duration_secs, 123);
        assert_eq!(updated.sort_order, 7);
    }

    #[tokio::test]
    async fn update_unknown_id_returns_none() {
        let pool = test_pool().await;
        let r = update(
            &pool,
            999999,
            UpdatePlaylistItem {
                title: "x".into(),
                url: "y".into(),
                duration_secs: 1,
                sort_order: 1,
            },
        )
        .await
        .unwrap();
        assert!(r.is_none());
    }

    #[test]
    fn validate_new_trims_and_keeps_fields() {
        let new = PlaylistInput {
            title: "  Ep 1  ".into(),
            url: "  https://x.example/e1.mp4  ".into(),
            duration_secs: 1800,
            sort_order: 4,
        }
        .validate_new(7)
        .unwrap();
        assert_eq!(new.channel_id, 7);
        assert_eq!(new.title, "Ep 1");
        assert_eq!(new.url, "https://x.example/e1.mp4");
        assert_eq!(new.duration_secs, 1800);
        assert_eq!(new.sort_order, 4);
    }

    #[test]
    fn validate_new_rejects_empty_title_url_and_nonpositive_duration() {
        assert!(PlaylistInput {
            title: "   ".into(),
            url: "https://x.example/e.mp4".into(),
            duration_secs: 10,
            sort_order: 0,
        }
        .validate_new(1)
        .is_err());

        assert!(PlaylistInput {
            title: "Ep".into(),
            url: "  ".into(),
            duration_secs: 10,
            sort_order: 0,
        }
        .validate_new(1)
        .is_err());

        assert!(PlaylistInput {
            title: "Ep".into(),
            url: "https://x.example/e.mp4".into(),
            duration_secs: 0,
            sort_order: 0,
        }
        .validate_new(1)
        .is_err());
    }

    #[test]
    fn validate_update_trims_and_keeps_fields() {
        let upd = PlaylistInput {
            title: " Ep 2 ".into(),
            url: " https://x.example/e2.mp4 ".into(),
            duration_secs: 600,
            sort_order: 2,
        }
        .validate_update()
        .unwrap();
        assert_eq!(upd.title, "Ep 2");
        assert_eq!(upd.url, "https://x.example/e2.mp4");
        assert_eq!(upd.duration_secs, 600);
        assert_eq!(upd.sort_order, 2);
    }

    #[tokio::test]
    async fn reap_stale_disabled_deletes_only_long_disabled_items() {
        let pool = test_pool().await;
        let ch = make_channel(&pool).await;
        let ttl = 3 * 86_400;

        let active = create(&pool, item(ch.id, "active", 60, 0)).await.unwrap();
        let fresh = create(&pool, item(ch.id, "fresh", 60, 1)).await.unwrap();
        let stale = create(&pool, item(ch.id, "stale", 60, 2)).await.unwrap();

        // `fresh` disabled just now; `stale` disabled 4 days ago.
        set_active(&pool, fresh.id, false).await.unwrap();
        set_active(&pool, stale.id, false).await.unwrap();
        sqlx::query(
            "UPDATE playlist_items SET disabled_at = strftime('%s','now') - ? WHERE id = ?",
        )
        .bind(4 * 86_400)
        .bind(stale.id)
        .execute(&pool)
        .await
        .unwrap();

        let reaped = reap_stale_disabled(&pool, ttl).await.unwrap();

        assert_eq!(reaped, vec![(stale.id, "stale".to_string())]);
        assert!(get(&pool, stale.id).await.unwrap().is_none(), "stale gone");
        assert!(get(&pool, fresh.id).await.unwrap().is_some(), "fresh kept");
        assert!(
            get(&pool, active.id).await.unwrap().is_some(),
            "active kept"
        );
    }

    #[tokio::test]
    async fn reap_stale_disabled_noop_when_nothing_stale() {
        let pool = test_pool().await;
        let ch = make_channel(&pool).await;
        create(&pool, item(ch.id, "a", 60, 0)).await.unwrap();
        let reaped = reap_stale_disabled(&pool, 3 * 86_400).await.unwrap();
        assert!(reaped.is_empty());
    }

    #[test]
    fn test_is_dead_truth_table() {
        let t = crate::model::source::FAILURE_THRESHOLD;
        // ok / null status → never dead, even past threshold
        assert!(!is_dead(Some("ok"), t + 5));
        assert!(!is_dead(None, t + 5));
        // errored but below threshold → not dead
        assert!(!is_dead(Some("error"), t - 1));
        // errored exactly at threshold → dead
        assert!(is_dead(Some("error"), t));
        // errored above threshold → dead
        assert!(is_dead(Some("error"), t + 1));
    }

    #[tokio::test]
    async fn apply_health_result_disables_only_at_threshold() {
        let pool = test_pool().await;
        let ch = make_channel(&pool).await;
        let it = create(&pool, item(ch.id, "dead", 60, 0)).await.unwrap();

        // Fail up to (threshold - 1): stays active.
        let mut cur = it.clone();
        for _ in 0..(crate::model::source::FAILURE_THRESHOLD - 1) {
            apply_health_result(&pool, &cur, false, Some("HTTP 404"))
                .await
                .unwrap();
            cur = get(&pool, it.id).await.unwrap().unwrap();
            assert!(cur.is_active, "must stay active below threshold");
        }

        // The failure that reaches threshold disables it.
        apply_health_result(&pool, &cur, false, Some("HTTP 404"))
            .await
            .unwrap();
        let after = get(&pool, it.id).await.unwrap().unwrap();
        assert!(!after.is_active, "must be disabled at threshold");
        assert_eq!(after.last_status.as_deref(), Some("error"));
        assert_eq!(after.failure_reason.as_deref(), Some("HTTP 404"));

        // A disabled item is gone from the active list → skipped on playback.
        let active = list_active_for_channel(&pool, ch.id).await.unwrap();
        assert!(active.iter().all(|i| i.id != it.id));
    }

    #[tokio::test]
    async fn apply_health_result_recovery_resets_failures_but_not_active() {
        let pool = test_pool().await;
        let ch = make_channel(&pool).await;
        let it = create(&pool, item(ch.id, "recovers", 60, 0)).await.unwrap();

        // Drive it dead.
        let mut cur = it.clone();
        for _ in 0..crate::model::source::FAILURE_THRESHOLD {
            apply_health_result(&pool, &cur, false, Some("HTTP 404"))
                .await
                .unwrap();
            cur = get(&pool, it.id).await.unwrap().unwrap();
        }
        assert!(!cur.is_active);

        // A later OK probe resets failures/status but does NOT auto-re-enable.
        apply_health_result(&pool, &cur, true, None).await.unwrap();
        let after = get(&pool, it.id).await.unwrap().unwrap();
        assert_eq!(after.consecutive_failures, 0);
        assert_eq!(after.last_status.as_deref(), Some("ok"));
        assert!(
            !after.is_active,
            "recovery never re-enables; admin does that"
        );
    }
}
