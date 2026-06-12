//! JSON admin API under /api/admin. Thin handlers over the model layer,
//! sharing the form admin's basic_auth. Responses serialize model structs;
//! requests use the DTOs defined per submodule.

mod channels;
mod discover;
mod playlist;
mod sources;

use axum::{
    http::StatusCode,
    response::{IntoResponse, Json, Response},
    routing::{get, post},
    Router,
};
use serde::{Deserialize, Serialize};

use crate::AppState;

/// Unified JSON error: renders `{ "error": "<msg>" }` with a status code.
pub enum ApiError {
    NotFound,
    Validation(String),
    Unavailable(String),
    Internal,
}

impl IntoResponse for ApiError {
    fn into_response(self) -> Response {
        let (status, msg) = match self {
            ApiError::NotFound => (StatusCode::NOT_FOUND, "not found".to_string()),
            ApiError::Validation(m) => (StatusCode::UNPROCESSABLE_ENTITY, m),
            ApiError::Unavailable(m) => (StatusCode::SERVICE_UNAVAILABLE, m),
            ApiError::Internal => (
                StatusCode::INTERNAL_SERVER_ERROR,
                "internal error".to_string(),
            ),
        };
        #[derive(Serialize)]
        struct ErrorBody {
            error: String,
        }
        (status, Json(ErrorBody { error: msg })).into_response()
    }
}

/// Map any model/db error to a logged 500.
pub(crate) fn internal<E: std::fmt::Display>(e: E) -> ApiError {
    tracing::error!("api error: {e}");
    ApiError::Internal
}

/// Shared payload for the `/toggle` endpoints: set is_active on a source or item.
#[derive(Deserialize)]
pub struct ToggleRequest {
    pub active: bool,
}

pub fn api_router() -> Router<AppState> {
    Router::new()
        .route("/channels", get(channels::list).post(channels::create))
        .route(
            "/channels/:id",
            get(channels::get_one)
                .patch(channels::update)
                .delete(channels::remove),
        )
        .route(
            "/channels/:id/sources",
            get(sources::list_for_channel).post(sources::create),
        )
        .route(
            "/sources/:id",
            get(sources::get_one)
                .patch(sources::update)
                .delete(sources::remove),
        )
        .route("/sources/:id/toggle", post(sources::toggle))
        .route("/sources/:id/test", post(sources::test))
        .route(
            "/channels/:id/playlist",
            get(playlist::list_for_channel).post(playlist::create),
        )
        .route(
            "/playlist/:id",
            get(playlist::get_one)
                .patch(playlist::update)
                .delete(playlist::remove),
        )
        .route("/playlist/:id/toggle", post(playlist::toggle))
        .route("/playlist/:id/test", post(playlist::test))
        .route("/discover/resolve", post(discover::resolve))
        .route("/discover/channel", post(discover::channel))
        .route("/discover/m3u", get(discover::m3u))
        .route("/discover/youtube", get(discover::youtube))
}
