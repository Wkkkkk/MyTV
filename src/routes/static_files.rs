use axum::{
    http::header,
    response::{IntoResponse, Redirect, Response},
};

const SVG: &str = include_str!("../../static/favicon.svg");

const MANIFEST: &str = r##"{"name":"MyTV","short_name":"MyTV","start_url":"/guide","display":"standalone","background_color":"#0f0f0f","theme_color":"#e94560","icons":[{"src":"/favicon.svg","sizes":"any","type":"image/svg+xml"}]}"##;

pub async fn favicon_svg() -> Response {
    ([(header::CONTENT_TYPE, "image/svg+xml")], SVG).into_response()
}

pub async fn manifest_json() -> Response {
    (
        [(header::CONTENT_TYPE, "application/manifest+json")],
        MANIFEST,
    )
        .into_response()
}

pub async fn favicon_ico() -> Redirect {
    Redirect::permanent("/favicon.svg")
}
