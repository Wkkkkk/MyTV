use anyhow::Result;
use mytv::{build_router, config, db, health, AppState, CorsCache};
use std::collections::HashMap;
use std::sync::Arc;
use tokio::sync::RwLock;
use tracing_subscriber::EnvFilter;

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

    let proxy_client = reqwest::Client::builder()
        .redirect(reqwest::redirect::Policy::none())
        .timeout(std::time::Duration::from_secs(10))
        .build()?;

    let cors_cache: CorsCache = Arc::new(RwLock::new(HashMap::new()));

    let state = AppState {
        pool,
        config: config.clone(),
        http_client,
        proxy_client,
        cors_cache,
    };

    health::start(
        state.pool.clone(),
        state.http_client.clone(),
        state.cors_cache.clone(),
    );

    let app = build_router(state);
    let addr = format!("0.0.0.0:{}", config.port);
    tracing::info!("listening on {addr}");
    let listener = tokio::net::TcpListener::bind(&addr).await?;
    axum::serve(listener, app).await?;

    Ok(())
}
