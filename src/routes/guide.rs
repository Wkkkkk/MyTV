use askama::Template;
use axum::{
    extract::{Query, State},
    http::StatusCode,
    response::Html,
};
use serde::Deserialize;

use crate::AppState;

#[derive(Template)]
#[template(path = "guide.html")]
struct GuidePageTemplate {}

#[derive(Debug, Deserialize)]
pub struct GuideQuery {
    pub category: Option<String>,
    pub offset: Option<i64>,
}

pub async fn guide_page(
    State(_state): State<AppState>,
    Query(_params): Query<GuideQuery>,
) -> Result<Html<String>, StatusCode> {
    let html = GuidePageTemplate {}
        .render()
        .map_err(|_| StatusCode::INTERNAL_SERVER_ERROR)?;
    Ok(Html(html))
}

pub async fn guide_partial(
    State(_state): State<AppState>,
    Query(_params): Query<GuideQuery>,
) -> Result<Html<String>, StatusCode> {
    Ok(Html("<p>partial placeholder</p>".into()))
}
