pub mod admin;
pub mod guide;
pub mod health;
pub mod player;

use axum::{http::StatusCode, response::Html};

pub fn render<T: askama::Template>(t: T) -> Result<Html<String>, StatusCode> {
    t.render().map(Html).map_err(|e| {
        tracing::error!("template render error: {e}");
        StatusCode::INTERNAL_SERVER_ERROR
    })
}

pub fn internal_error<E: std::fmt::Display>(e: E) -> StatusCode {
    tracing::error!("{e}");
    StatusCode::INTERNAL_SERVER_ERROR
}
