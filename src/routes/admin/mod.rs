pub mod channels;
pub mod discover;
pub mod playlist;
pub mod sources;

// Re-export all handlers so main.rs route wiring stays unchanged
pub use channels::{
    admin_index, channel_create, channel_delete, channel_detail, channel_edit_form, channel_list,
    channel_new_form, channel_update,
};
pub use discover::{
    discover_add, discover_add_form, discover_m3u_search, discover_manual_resolve, discover_page,
    discover_youtube_search,
};
pub use playlist::{playlist_item_create, playlist_item_delete};
pub use sources::{source_create, source_delete, source_test, source_toggle};

use axum::{
    extract::{Request, State},
    http::{header, StatusCode},
    middleware::Next,
    response::{IntoResponse, Response},
};
use base64::{engine::general_purpose, Engine as _};

use crate::{
    model::{channel, playlist_item, source},
    AppState,
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
}

pub struct AdminPlaylistItemRow {
    pub id: i64,
    pub title: String,
    pub url: String,
    pub duration_secs: i64,
    pub sort_order: i64,
}

// ── From impls ─────────────────────────────────────────────────────────────

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

impl From<source::Source> for AdminSourceRow {
    fn from(s: source::Source) -> Self {
        Self {
            id: s.id,
            kind: s.kind,
            url: s.url,
            priority: s.priority,
            is_active: s.is_active,
        }
    }
}

impl From<playlist_item::PlaylistItem> for AdminPlaylistItemRow {
    fn from(i: playlist_item::PlaylistItem) -> Self {
        Self {
            id: i.id,
            title: i.title,
            url: i.url,
            duration_secs: i.duration_secs,
            sort_order: i.sort_order,
        }
    }
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
