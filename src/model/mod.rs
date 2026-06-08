pub mod channel;
pub mod playlist_item;
pub mod source;

use anyhow::Result;
use sqlx::SqlitePool;

/// Shared health-update SQL for `sources` and `playlist_items`.
/// `table` must be a `'static` literal — "sources" or "playlist_items".
pub(crate) async fn update_health_sql(
    pool: &SqlitePool,
    table: &'static str,
    id: i64,
    status: &str,
    reason: Option<&str>,
    consecutive_failures: i64,
    is_active: Option<bool>,
) -> Result<()> {
    if let Some(active) = is_active {
        sqlx::query(&format!(
            "UPDATE {table}
             SET last_checked_at = strftime('%s','now'),
                 last_status = ?,
                 failure_reason = ?,
                 consecutive_failures = ?,
                 is_active = ?
             WHERE id = ?"
        ))
        .bind(status)
        .bind(reason)
        .bind(consecutive_failures)
        .bind(active)
        .bind(id)
        .execute(pool)
        .await?;
    } else {
        sqlx::query(&format!(
            "UPDATE {table}
             SET last_checked_at = strftime('%s','now'),
                 last_status = ?,
                 failure_reason = ?,
                 consecutive_failures = ?
             WHERE id = ?"
        ))
        .bind(status)
        .bind(reason)
        .bind(consecutive_failures)
        .bind(id)
        .execute(pool)
        .await?;
    }
    Ok(())
}
