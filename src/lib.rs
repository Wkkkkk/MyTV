mod budget;
pub mod config;
pub mod db;
mod epg;
pub mod health;
mod media;
mod model;
mod routes;
pub mod ssrf;

use axum::{
    extract::Request,
    middleware::{self, Next},
    response::{IntoResponse, Redirect},
    routing::{get, post},
    Router,
};
use sqlx::SqlitePool;
use std::collections::HashMap;
use std::sync::Arc;
use tokio::sync::RwLock;

pub type CorsCache = Arc<RwLock<HashMap<String, bool>>>;

#[derive(Clone)]
pub struct AppState {
    pub pool: SqlitePool,
    pub config: Arc<config::Config>,
    pub http_client: reqwest::Client,
    pub proxy_client: reqwest::Client,
    pub cors_cache: CorsCache,
}

async fn redirect_trailing_slash(req: Request, next: Next) -> axum::response::Response {
    let path = req.uri().path();
    if path != "/" && path.ends_with('/') {
        let new_path = path.trim_end_matches('/');
        let location = match req.uri().query() {
            Some(q) => format!("{}?{}", new_path, q),
            None => new_path.to_string(),
        };
        return Redirect::permanent(&location).into_response();
    }
    next.run(req).await
}

pub fn build_router(state: AppState) -> Router {
    let admin_router: Router<AppState> = Router::new()
        .route("/", get(routes::admin::admin_index))
        .route(
            "/channels",
            get(routes::admin::channel_list).post(routes::admin::channel_create),
        )
        .route("/channels/new", get(routes::admin::channel_new_form))
        .route(
            "/channels/:id",
            get(routes::admin::channel_detail).post(routes::admin::channel_update),
        )
        .route("/channels/:id/edit", get(routes::admin::channel_edit_form))
        .route("/channels/:id/delete", post(routes::admin::channel_delete))
        .route("/channels/:id/sources", post(routes::admin::source_create))
        .route("/sources/:id/delete", post(routes::admin::source_delete))
        .route("/sources/:id/toggle", post(routes::admin::source_toggle))
        .route("/sources/:id/test", post(routes::admin::source_test))
        .route(
            "/channels/:id/playlist",
            post(routes::admin::playlist_item_create),
        )
        .route(
            "/playlist/:id/delete",
            post(routes::admin::playlist_item_delete),
        )
        .route(
            "/playlist/:id/test",
            post(routes::admin::playlist_item_test),
        )
        .route("/discover", get(routes::admin::discover_page))
        .route("/discover/add-form", post(routes::admin::discover_add_form))
        .route("/discover/add", post(routes::admin::discover_add))
        .route(
            "/discover/m3u/search",
            post(routes::admin::discover_m3u_search),
        )
        .route(
            "/discover/youtube/search",
            post(routes::admin::discover_youtube_search),
        )
        .route(
            "/discover/manual/resolve",
            post(routes::admin::discover_manual_resolve),
        )
        .route_layer(middleware::from_fn_with_state(
            state.clone(),
            routes::admin::basic_auth,
        ));

    Router::new()
        .route("/", get(|| async { Redirect::permanent("/guide") }))
        .route("/health", get(routes::health::health_check))
        .route("/guide", get(routes::guide::guide_page))
        .route("/guide/partial", get(routes::guide::guide_partial))
        .route("/channel/:id/tune", get(routes::player::tune))
        .route("/channel/:id/next", get(routes::player::next))
        .route("/stream-proxy", get(routes::player::stream_proxy))
        .route("/favicon.svg", get(routes::static_files::favicon_svg))
        .route("/manifest.json", get(routes::static_files::manifest_json))
        .route("/favicon.ico", get(routes::static_files::favicon_ico))
        .nest("/admin", admin_router)
        .layer(middleware::from_fn(redirect_trailing_slash))
        .with_state(state)
}
