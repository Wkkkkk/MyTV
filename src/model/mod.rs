pub mod channel;
pub mod playlist_item;
pub mod source;

use anyhow::Result;
use sqlx::SqlitePool;

/// A validation failure produced by the `*Input::validate_*` intake validators.
/// Carries a human-readable message; adapters decide the transport status code.
#[derive(Debug)]
pub struct IntakeError(pub String);

impl std::fmt::Display for IntakeError {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        f.write_str(&self.0)
    }
}

/// Default source priority when the intake field is blank/absent.
pub const DEFAULT_PRIORITY: i64 = 1;
/// Default sort order when the intake field is blank/absent.
pub const DEFAULT_SORT_ORDER: i64 = 0;

/// Coerce a form numeric field to `i64`: trimmed-blank/absent → `default`;
/// present-but-unparseable → `IntakeError` (strict — the adapter surfaces it as 422).
/// The single source of truth for intake numeric coercion across the form and JSON doors.
pub fn coerce_i64(raw: &str, default: i64) -> std::result::Result<i64, IntakeError> {
    let trimmed = raw.trim();
    if trimmed.is_empty() {
        Ok(default)
    } else {
        trimmed
            .parse()
            .map_err(|_| IntakeError(format!("expected an integer, got {trimmed:?}")))
    }
}

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

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn test_coerce_i64_blank_and_whitespace_use_default() {
        assert_eq!(coerce_i64("", 7).unwrap(), 7);
        assert_eq!(coerce_i64("   ", 7).unwrap(), 7);
    }

    #[test]
    fn test_coerce_i64_parses_trimmed_value() {
        assert_eq!(coerce_i64("5", 0).unwrap(), 5);
        assert_eq!(coerce_i64("  42 ", 0).unwrap(), 42);
        assert_eq!(coerce_i64("-3", 0).unwrap(), -3);
    }

    #[test]
    fn test_coerce_i64_garbage_is_error() {
        assert!(coerce_i64("abc", 0).is_err());
        assert!(coerce_i64("1.5", 0).is_err());
    }

    #[test]
    fn test_default_constants() {
        assert_eq!(DEFAULT_PRIORITY, 1);
        assert_eq!(DEFAULT_SORT_ORDER, 0);
    }
}
