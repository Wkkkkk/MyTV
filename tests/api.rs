use http_body_util::BodyExt;
use mytv::{build_router, config::Config, metrics, AppState};
use std::sync::Arc;
use tower::ServiceExt;

use axum::{
    body::Body,
    http::{Request, StatusCode},
};

async fn app() -> axum::Router {
    let pool = mytv::db::connect("sqlite::memory:").await.unwrap();
    sqlx::query(include_str!("fixtures/seed.sql"))
        .execute(&pool)
        .await
        .unwrap();
    let state = AppState {
        pool,
        config: Arc::new(Config {
            database_url: "sqlite::memory:".to_string(),
            admin_password: "test".to_string(),
            youtube_api_key: None,
            port: 0,
        }),
        http_client: reqwest::Client::builder()
            .connect_timeout(std::time::Duration::from_secs(1))
            .timeout(std::time::Duration::from_secs(2))
            .build()
            .unwrap(),
        proxy_client: reqwest::Client::builder()
            .redirect(reqwest::redirect::Policy::none())
            .build()
            .unwrap(),
        cors_cache: Arc::new(tokio::sync::RwLock::new(std::collections::HashMap::new())),
        ssrf_cache: Arc::new(tokio::sync::RwLock::new(std::collections::HashMap::new())),
        metrics: Arc::new(metrics::Metrics::new()),
        live_cache: Arc::new(tokio::sync::RwLock::new(std::collections::HashMap::new())),
    };
    build_router(state)
}

const AUTH: &str = "Basic dXNlcjp0ZXN0"; // user:test

fn get(uri: &str) -> Request<Body> {
    Request::builder().uri(uri).body(Body::empty()).unwrap()
}

fn authed_get(uri: &str) -> Request<Body> {
    Request::builder()
        .uri(uri)
        .header("Authorization", AUTH)
        .body(Body::empty())
        .unwrap()
}

#[allow(dead_code)]
fn authed_json(method: &str, uri: &str, body: serde_json::Value) -> Request<Body> {
    Request::builder()
        .method(method)
        .uri(uri)
        .header("Authorization", AUTH)
        .header("content-type", "application/json")
        .body(Body::from(body.to_string()))
        .unwrap()
}

async fn body_json(response: axum::response::Response) -> serde_json::Value {
    let bytes = response.into_body().collect().await.unwrap().to_bytes();
    serde_json::from_slice(&bytes).unwrap()
}

#[tokio::test]
async fn channels_list_requires_auth() {
    let r = app()
        .await
        .oneshot(get("/api/admin/channels"))
        .await
        .unwrap();
    assert_eq!(r.status(), StatusCode::UNAUTHORIZED);
}

#[tokio::test]
async fn channels_list_returns_seeded_channels() {
    let r = app()
        .await
        .oneshot(authed_get("/api/admin/channels"))
        .await
        .unwrap();
    assert_eq!(r.status(), StatusCode::OK);
    let json = body_json(r).await;
    let arr = json.as_array().expect("array");
    assert!(arr.len() >= 5);
    assert!(arr.iter().any(|c| c["name"] == "Live OK"));
}
