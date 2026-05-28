mod config;
mod db;
mod epg;
mod media;
mod model;
mod routes;

use anyhow::Result;
use axum::{
    extract::Request,
    middleware::{self, Next},
    response::{IntoResponse, Redirect},
    routing::{get, post},
    Router,
};
use sqlx::SqlitePool;
use std::sync::Arc;
use tracing_subscriber::EnvFilter;

#[derive(Clone)]
pub struct AppState {
    pub pool: SqlitePool,
    pub config: Arc<config::Config>,
    pub http_client: reqwest::Client,
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

#[tokio::main]
async fn main() -> Result<()> {
    dotenvy::dotenv().ok();
    tracing_subscriber::fmt()
        .with_env_filter(EnvFilter::from_default_env())
        .init();

    let config = Arc::new(config::Config::from_env()?);
    let pool = db::connect(&config.database_url).await?;
    let http_client = reqwest::Client::builder()
        .timeout(std::time::Duration::from_secs(10))
        .build()?;

    let state = AppState {
        pool,
        config: config.clone(),
        http_client,
    };

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

    let app = Router::new()
        .route("/health", get(routes::health::health_check))
        .route("/guide", get(routes::guide::guide_page))
        .route("/guide/partial", get(routes::guide::guide_partial))
        .route("/channel/:id/tune", get(routes::player::tune))
        .route("/channel/:id/next", get(routes::player::next))
        .route("/stream-proxy", get(routes::player::stream_proxy))
        .nest("/admin", admin_router)
        .layer(middleware::from_fn(redirect_trailing_slash))
        .with_state(state);

    let addr = format!("0.0.0.0:{}", config.port);
    tracing::info!("listening on {addr}");
    let listener = tokio::net::TcpListener::bind(&addr).await?;
    axum::serve(listener, app).await?;

    Ok(())
}
