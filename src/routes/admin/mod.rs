pub mod channels;
pub mod discover;
pub mod live_status;
pub mod metrics;
pub mod playlist;
pub mod sources;

// Re-export all handlers so main.rs route wiring stays unchanged
pub use channels::{
    admin_index, channel_create, channel_delete, channel_detail, channel_edit_form, channel_list,
    channel_new_form, channel_update,
};
pub use discover::{
    discover_add, discover_add_form, discover_channel_resolve, discover_m3u_search,
    discover_manual_resolve, discover_page, discover_youtube_search,
};
pub use live_status::live_status_badge;
pub use metrics::metrics_json;
pub use playlist::{
    playlist_item_create, playlist_item_delete, playlist_item_test, playlist_item_toggle,
};
pub use sources::{source_create, source_delete, source_test, source_toggle};

use axum::{
    extract::{Request, State},
    http::{header, StatusCode},
    middleware::Next,
    response::{IntoResponse, Response},
};
use base64::{engine::general_purpose, Engine as _};

use std::collections::HashMap;

use crate::{
    model::{channel, playlist_item, source},
    AppState, CorsCache,
};

// ── display types ──────────────────────────────────────────────────────────

pub struct AdminChannelRow {
    pub id: i64,
    pub name: String,
    pub category: String,
    pub type_str: String,
    pub sort_order: i64,
}

pub struct AdminSourceRow {
    pub id: i64,
    pub kind: String,
    pub url: String,
    pub priority: i64,
    pub is_active: bool,
    pub failure_reason: Option<String>,
    pub budget_badge_class: &'static str,
    pub budget_badge_char: &'static str,
    pub status_color: &'static str,
    pub status_glyph: &'static str,
    pub status_title: String,
    /// True only for an active `youtube_live` source: the row lazy-loads its
    /// Status from `/admin/live-status` (yt-dlp probe) instead of rendering inline.
    pub status_lazy: bool,
}

pub struct AdminPlaylistItemRow {
    pub id: i64,
    pub title: String,
    pub url: String,
    pub duration_secs: i64,
    pub sort_order: i64,
    pub budget_badge_class: &'static str,
    pub budget_badge_char: &'static str,
    pub is_active: bool,
    pub failure_reason: Option<String>,
    pub status_color: &'static str,
    pub status_glyph: &'static str,
    pub status_title: String,
}

/// Admin display rows that derive a network-budget badge from their URL.
/// `from_model` is the *only* construction path, so a row can never exist
/// without its badge filled — there is no half-built state to forget.
pub(crate) trait BudgetRow<T>: Sized {
    fn from_model(item: T, cors_cache: &HashMap<String, bool>) -> Self;
}

// ── From impl ──────────────────────────────────────────────────────────────

impl From<channel::Channel> for AdminChannelRow {
    fn from(ch: channel::Channel) -> Self {
        Self {
            id: ch.id,
            name: ch.name,
            category: ch.category,
            type_str: ch.r#type,
            sort_order: ch.sort_order,
        }
    }
}

impl BudgetRow<source::Source> for AdminSourceRow {
    fn from_model(s: source::Source, cors_cache: &HashMap<String, bool>) -> Self {
        let (budget_badge_class, budget_badge_char) =
            crate::budget::badge_for_url(&s.url, cors_cache);
        let status_lazy = s.is_active && s.kind == "youtube_live";
        // Inline status for non-lazy rows (disabled, or non-youtube). Lazy rows
        // ignore these fields and fetch the badge via HTMX.
        let status = crate::status::compute(
            s.is_active,
            &s.kind,
            s.last_status.as_deref(),
            s.failure_reason.as_deref(),
            None,
        );
        let badge = crate::status::status_badge(&status);
        Self {
            id: s.id,
            kind: s.kind,
            url: s.url,
            priority: s.priority,
            is_active: s.is_active,
            failure_reason: s.failure_reason,
            budget_badge_class,
            budget_badge_char,
            status_color: badge.color,
            status_glyph: badge.glyph,
            status_title: badge.title,
            status_lazy,
        }
    }
}

impl BudgetRow<playlist_item::PlaylistItem> for AdminPlaylistItemRow {
    fn from_model(i: playlist_item::PlaylistItem, cors_cache: &HashMap<String, bool>) -> Self {
        let (budget_badge_class, budget_badge_char) =
            crate::budget::badge_for_url(&i.url, cors_cache);
        let status = crate::status::compute(
            i.is_active,
            "hls", // playlist items use health only — never the youtube_live live branch
            i.last_status.as_deref(),
            i.failure_reason.as_deref(),
            None,
        );
        let badge = crate::status::status_badge(&status);
        Self {
            id: i.id,
            title: i.title,
            url: i.url,
            duration_secs: i.duration_secs,
            sort_order: i.sort_order,
            budget_badge_class,
            budget_badge_char,
            is_active: i.is_active,
            failure_reason: i.failure_reason,
            status_color: badge.color,
            status_glyph: badge.glyph,
            status_title: badge.title,
        }
    }
}

/// Reads the CORS-cache snapshot once and builds a budget-badged row per item.
/// Callers never touch the cache or fill a badge — `from_model` does both.
pub(crate) async fn build_rows<R, T, I>(items: I, cors_cache: &CorsCache) -> Vec<R>
where
    I: IntoIterator<Item = T>,
    R: BudgetRow<T>,
{
    let cors = cors_cache.read().await.clone();
    items
        .into_iter()
        .map(|it| R::from_model(it, &cors))
        .collect()
}

/// Single-row variant of [`build_rows`].
pub(crate) async fn build_row<R, T>(item: T, cors_cache: &CorsCache) -> R
where
    R: BudgetRow<T>,
{
    let cors = cors_cache.read().await.clone();
    R::from_model(item, &cors)
}

// ── auth ───────────────────────────────────────────────────────────────────

pub fn check_basic_auth(header_value: &str, expected_password: &str) -> bool {
    header_value
        .strip_prefix("Basic ")
        .and_then(|b64| general_purpose::STANDARD.decode(b64).ok())
        .and_then(|bytes| String::from_utf8(bytes).ok())
        .map(|credentials| {
            let actual = credentials.split_once(':').map(|x| x.1).unwrap_or("");
            actual.len() == expected_password.len()
                && actual
                    .bytes()
                    .zip(expected_password.bytes())
                    .fold(0u8, |acc, (a, b)| acc | (a ^ b))
                    == 0
        })
        .unwrap_or(false)
}

pub async fn basic_auth(State(state): State<AppState>, request: Request, next: Next) -> Response {
    let authorized = request
        .headers()
        .get(header::AUTHORIZATION)
        .and_then(|v| v.to_str().ok())
        .map(|v| check_basic_auth(v, &state.config.admin_password))
        .unwrap_or(false);

    if authorized {
        next.run(request).await
    } else {
        (
            StatusCode::UNAUTHORIZED,
            [(header::WWW_AUTHENTICATE, "Basic realm=\"MyTV Admin\"")],
            "Unauthorized",
        )
            .into_response()
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    use crate::media::hls::extract_manifest_host;
    use crate::model::{playlist_item::PlaylistItem, source::Source};
    use std::collections::HashMap;

    fn sample_source(url: &str) -> Source {
        Source {
            id: 1,
            channel_id: 1,
            kind: "hls".to_string(),
            url: url.to_string(),
            priority: 0,
            is_active: true,
            last_checked_at: None,
            last_status: None,
            consecutive_failures: 0,
            failure_reason: None,
        }
    }

    fn sample_item(url: &str) -> PlaylistItem {
        PlaylistItem {
            id: 1,
            channel_id: 1,
            title: "ep".to_string(),
            url: url.to_string(),
            duration_secs: 60,
            sort_order: 0,
            is_active: true,
            last_checked_at: None,
            last_status: None,
            consecutive_failures: 0,
            failure_reason: None,
        }
    }

    #[test]
    fn from_model_source_cache_hit_true_is_direct() {
        let url = "https://cdn.example.com/live/stream.m3u8";
        let mut cache = HashMap::new();
        cache.insert(extract_manifest_host(url), true);
        let row = AdminSourceRow::from_model(sample_source(url), &cache);
        assert_eq!(row.budget_badge_class, "budget-direct");
        assert_eq!(row.budget_badge_char, "⚡");
    }

    #[test]
    fn from_model_source_cache_hit_false_is_proxied() {
        let url = "https://cdn.example.com/live/stream.m3u8";
        let mut cache = HashMap::new();
        cache.insert(extract_manifest_host(url), false);
        let row = AdminSourceRow::from_model(sample_source(url), &cache);
        assert_eq!(row.budget_badge_class, "budget-proxied");
        assert_eq!(row.budget_badge_char, "☁");
    }

    #[test]
    fn from_model_source_cache_miss_is_unknown() {
        let url = "https://cdn.example.com/live/stream.m3u8";
        let row = AdminSourceRow::from_model(sample_source(url), &HashMap::new());
        assert_eq!(row.budget_badge_class, "budget-unknown");
        assert_eq!(row.budget_badge_char, "");
    }

    #[test]
    fn from_model_playlist_item_cache_hit_true_is_direct() {
        let url = "https://cdn.example.com/vod/ep1.m3u8";
        let mut cache = HashMap::new();
        cache.insert(extract_manifest_host(url), true);
        let row = AdminPlaylistItemRow::from_model(sample_item(url), &cache);
        assert_eq!(row.budget_badge_class, "budget-direct");
        assert_eq!(row.budget_badge_char, "⚡");
    }

    #[test]
    fn from_model_playlist_item_cache_miss_is_unknown() {
        let url = "https://cdn.example.com/vod/ep1.m3u8";
        let row = AdminPlaylistItemRow::from_model(sample_item(url), &HashMap::new());
        assert_eq!(row.budget_badge_class, "budget-unknown");
        assert_eq!(row.budget_badge_char, "");
    }

    #[tokio::test]
    async fn build_rows_fills_every_row() {
        let known = "https://known.example.com/a.m3u8";
        let unknown = "https://unknown.example.com/b.m3u8";
        let mut map = HashMap::new();
        map.insert(extract_manifest_host(known), true);
        let cache: crate::CorsCache = std::sync::Arc::new(tokio::sync::RwLock::new(map));

        let rows: Vec<AdminSourceRow> =
            build_rows(vec![sample_source(known), sample_source(unknown)], &cache).await;

        assert_eq!(rows.len(), 2);
        assert_eq!(rows[0].budget_badge_class, "budget-direct");
        assert_eq!(rows[0].budget_badge_char, "⚡");
        assert_eq!(rows[1].budget_badge_class, "budget-unknown");
        assert_eq!(rows[1].budget_badge_char, "");
    }

    #[tokio::test]
    async fn build_row_fills_single_row() {
        let url = "https://cdn.example.com/live/stream.m3u8";
        let mut map = HashMap::new();
        map.insert(extract_manifest_host(url), false);
        let cache: crate::CorsCache = std::sync::Arc::new(tokio::sync::RwLock::new(map));

        let row: AdminSourceRow = build_row(sample_source(url), &cache).await;
        assert_eq!(row.budget_badge_class, "budget-proxied");
        assert_eq!(row.budget_badge_char, "☁");
    }

    #[test]
    fn test_check_basic_auth_valid_credentials() {
        assert!(check_basic_auth("Basic dXNlcjpzZWNyZXQ=", "secret"));
    }

    #[test]
    fn test_check_basic_auth_wrong_password() {
        assert!(!check_basic_auth("Basic dXNlcjp3cm9uZw==", "secret"));
    }

    #[test]
    fn test_check_basic_auth_malformed_no_basic_prefix() {
        assert!(!check_basic_auth("Bearer sometoken", "secret"));
    }

    #[test]
    fn test_check_basic_auth_empty_header() {
        assert!(!check_basic_auth("", "secret"));
    }

    #[test]
    fn test_check_basic_auth_no_colon_in_credentials() {
        assert!(!check_basic_auth("Basic cGFzc3dvcmRvbmx5", "passwordonly"));
    }

    #[test]
    fn test_check_basic_auth_password_containing_colon() {
        assert!(check_basic_auth("Basic dXNlcjpwYXNzOndvcmQ=", "pass:word"));
    }
}
