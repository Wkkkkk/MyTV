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

#[tokio::test]
async fn channel_crud_round_trip() {
    let app = app().await;

    let r = app
        .clone()
        .oneshot(authed_json(
            "POST",
            "/api/admin/channels",
            serde_json::json!({"name":"API Made","category":"test","type":"live","sort_order":99}),
        ))
        .await
        .unwrap();
    assert_eq!(r.status(), StatusCode::CREATED);
    let created = body_json(r).await;
    let id = created["id"].as_i64().unwrap();
    assert_eq!(created["name"], "API Made");
    assert_eq!(created["type"], "live");

    let r = app
        .clone()
        .oneshot(authed_get(&format!("/api/admin/channels/{id}")))
        .await
        .unwrap();
    assert_eq!(r.status(), StatusCode::OK);
    assert_eq!(body_json(r).await["name"], "API Made");

    let r = app.clone().oneshot(authed_json("PATCH", &format!("/api/admin/channels/{id}"),
        serde_json::json!({"name":"API Renamed","category":"test","type":"live","sort_order":1}))).await.unwrap();
    assert_eq!(r.status(), StatusCode::OK);
    assert_eq!(body_json(r).await["name"], "API Renamed");

    let r = app
        .clone()
        .oneshot(
            Request::builder()
                .method("DELETE")
                .uri(format!("/api/admin/channels/{id}"))
                .header("Authorization", AUTH)
                .body(Body::empty())
                .unwrap(),
        )
        .await
        .unwrap();
    assert_eq!(r.status(), StatusCode::NO_CONTENT);

    let r = app
        .oneshot(authed_get(&format!("/api/admin/channels/{id}")))
        .await
        .unwrap();
    assert_eq!(r.status(), StatusCode::NOT_FOUND);
}

#[tokio::test]
async fn channel_create_bad_type_is_422() {
    let r = app()
        .await
        .oneshot(authed_json(
            "POST",
            "/api/admin/channels",
            serde_json::json!({"name":"x","category":"y","type":"nonsense","sort_order":0}),
        ))
        .await
        .unwrap();
    assert_eq!(r.status(), StatusCode::UNPROCESSABLE_ENTITY);
}

#[tokio::test]
async fn channel_get_unknown_is_404() {
    let r = app()
        .await
        .oneshot(authed_get("/api/admin/channels/999999"))
        .await
        .unwrap();
    assert_eq!(r.status(), StatusCode::NOT_FOUND);
}

#[tokio::test]
async fn channel_create_empty_name_is_422() {
    let r = app()
        .await
        .oneshot(authed_json(
            "POST",
            "/api/admin/channels",
            serde_json::json!({"name":"   ","category":"y","type":"live","sort_order":0}),
        ))
        .await
        .unwrap();
    assert_eq!(r.status(), StatusCode::UNPROCESSABLE_ENTITY);
}

#[tokio::test]
async fn channel_update_preserves_existing_loop_anchor() {
    let app = app().await;
    // create a vod_loop channel with an explicit anchor
    let r = app.clone().oneshot(authed_json("POST", "/api/admin/channels",
        serde_json::json!({"name":"VOD A","category":"test","type":"vod_loop","sort_order":0,"loop_anchor":"2021-05-05T10:00"}))).await.unwrap();
    assert_eq!(r.status(), StatusCode::CREATED);
    let created = body_json(r).await;
    let id = created["id"].as_i64().unwrap();
    let anchor = created["loop_anchor"].clone();
    assert!(!anchor.is_null());

    // PATCH changing only the name, omitting loop_anchor — anchor must be preserved
    let r = app
        .oneshot(authed_json(
            "PATCH",
            &format!("/api/admin/channels/{id}"),
            serde_json::json!({"name":"VOD B","category":"test","type":"vod_loop","sort_order":0}),
        ))
        .await
        .unwrap();
    assert_eq!(r.status(), StatusCode::OK);
    let updated = body_json(r).await;
    assert_eq!(updated["name"], "VOD B");
    assert_eq!(
        updated["loop_anchor"], anchor,
        "loop_anchor must be preserved when omitted on update"
    );
}
