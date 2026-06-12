//! JSON admin API under /api/admin. Thin handlers over the model layer,
//! sharing the form admin's basic_auth. Responses serialize model structs;
//! requests use the DTOs defined per submodule.

mod channels;

use axum::{
    http::StatusCode,
    response::{IntoResponse, Json, Response},
    routing::get,
    Router,
};
use serde::Serialize;

use crate::AppState;

/// Unified JSON error: renders `{ "error": "<msg>" }` with a status code.
pub enum ApiError {
    NotFound,
    Validation(String),
    Internal,
}

impl IntoResponse for ApiError {
    fn into_response(self) -> Response {
        let (status, msg) = match self {
            ApiError::NotFound => (StatusCode::NOT_FOUND, "not found".to_string()),
            ApiError::Validation(m) => (StatusCode::UNPROCESSABLE_ENTITY, m),
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

pub fn api_router() -> Router<AppState> {
    Router::new()
        .route("/channels", get(channels::list).post(channels::create))
        .route(
            "/channels/:id",
            get(channels::get_one)
                .patch(channels::update)
                .delete(channels::remove),
        )
}
