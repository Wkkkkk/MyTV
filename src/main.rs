mod channel;
mod config;
mod db;
mod epg;
mod playlist_item;
mod resolver;
mod routes;
mod source;

use anyhow::Result;
use axum::{middleware, routing::{get, post}, Router};
use sqlx::SqlitePool;
use std::sync::Arc;
use tracing_subscriber::EnvFilter;

#[derive(Clone)]
pub struct AppState {
    pub pool: SqlitePool,
    pub config: Arc<config::Config>,
}

#[tokio::main]
async fn main() -> Result<()> {
    dotenvy::dotenv().ok();
    tracing_subscriber::fmt()
        .with_env_filter(EnvFilter::from_default_env())
        .init();

    let config = Arc::new(config::Config::from_env()?);
    let pool = db::connect(&config.database_url).await?;

    let state = AppState {
        pool,
        config: config.clone(),
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
        .route("/channels/:id/playlist", post(routes::admin::playlist_item_create))
        .route("/playlist/:id/delete", post(routes::admin::playlist_item_delete))
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
        .nest("/admin", admin_router)
        .with_state(state);

    let addr = format!("0.0.0.0:{}", config.port);
    tracing::info!("listening on {addr}");
    let listener = tokio::net::TcpListener::bind(&addr).await?;
    axum::serve(listener, app).await?;

    Ok(())
}
