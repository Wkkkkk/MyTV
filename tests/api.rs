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

#[tokio::test]
async fn source_crud_and_kind_autodetect() {
    let app = app().await;

    let r = app
        .clone()
        .oneshot(authed_json(
            "POST",
            "/api/admin/channels/1/sources",
            serde_json::json!({"url": "https://www.youtube.com/watch?v=abc"}),
        ))
        .await
        .unwrap();
    assert_eq!(r.status(), StatusCode::CREATED);
    let created = body_json(r).await;
    let id = created["id"].as_i64().unwrap();
    assert_eq!(created["kind"], "youtube_live"); // SourceKind::detect → YoutubeLive
    assert_eq!(created["channel_id"], 1);

    let r = app
        .clone()
        .oneshot(authed_get("/api/admin/channels/1/sources"))
        .await
        .unwrap();
    assert_eq!(r.status(), StatusCode::OK);
    assert!(body_json(r)
        .await
        .as_array()
        .unwrap()
        .iter()
        .any(|s| s["id"] == id));

    let r = app
        .clone()
        .oneshot(authed_json(
            "PATCH",
            &format!("/api/admin/sources/{id}"),
            serde_json::json!({"url":"https://x.example.com/y.m3u8","priority":5}),
        ))
        .await
        .unwrap();
    assert_eq!(r.status(), StatusCode::OK);
    let updated = body_json(r).await;
    assert_eq!(updated["url"], "https://x.example.com/y.m3u8");
    assert_eq!(updated["priority"], 5);

    let r = app
        .clone()
        .oneshot(authed_json(
            "POST",
            &format!("/api/admin/sources/{id}/toggle"),
            serde_json::json!({"active": false}),
        ))
        .await
        .unwrap();
    assert_eq!(r.status(), StatusCode::OK);
    assert_eq!(body_json(r).await["is_active"], false);

    let r = app
        .clone()
        .oneshot(
            Request::builder()
                .method("DELETE")
                .uri(format!("/api/admin/sources/{id}"))
                .header("Authorization", AUTH)
                .body(Body::empty())
                .unwrap(),
        )
        .await
        .unwrap();
    assert_eq!(r.status(), StatusCode::NO_CONTENT);
}

#[tokio::test]
async fn source_create_empty_url_is_422() {
    let r = app()
        .await
        .oneshot(authed_json(
            "POST",
            "/api/admin/channels/1/sources",
            serde_json::json!({"url": "   "}),
        ))
        .await
        .unwrap();
    assert_eq!(r.status(), StatusCode::UNPROCESSABLE_ENTITY);
}

#[tokio::test]
async fn source_test_populates_last_checked() {
    // Seed source 1 (https://stream.example.com, unreachable in tests) → probe
    // fast-fails and still records last_checked_at. Mirrors the non-ignored
    // test_source_test_returns_row_partial_not_ok_badge in http.rs.
    let r = app()
        .await
        .oneshot(
            Request::builder()
                .method("POST")
                .uri("/api/admin/sources/1/test")
                .header("Authorization", AUTH)
                .body(Body::empty())
                .unwrap(),
        )
        .await
        .unwrap();
    assert_eq!(r.status(), StatusCode::OK);
    let json = body_json(r).await;
    assert_eq!(json["id"], 1);
    assert!(!json["last_checked_at"].is_null());
}

#[tokio::test]
async fn source_update_unknown_is_404() {
    let r = app()
        .await
        .oneshot(authed_json(
            "PATCH",
            "/api/admin/sources/999999",
            serde_json::json!({"url":"https://x","priority":1}),
        ))
        .await
        .unwrap();
    assert_eq!(r.status(), StatusCode::NOT_FOUND);
}

#[tokio::test]
async fn playlist_crud_round_trip() {
    let app = app().await;

    let r = app.clone().oneshot(authed_json("POST", "/api/admin/channels/4/playlist",
        serde_json::json!({"title":"API Ep","url":"https://vod.example.com/api.mp4","duration_secs":600,"sort_order":10}))).await.unwrap();
    assert_eq!(r.status(), StatusCode::CREATED);
    let created = body_json(r).await;
    let id = created["id"].as_i64().unwrap();
    assert_eq!(created["title"], "API Ep");
    assert_eq!(created["channel_id"], 4);

    let r = app
        .clone()
        .oneshot(authed_get("/api/admin/channels/4/playlist"))
        .await
        .unwrap();
    assert_eq!(r.status(), StatusCode::OK);
    assert!(body_json(r)
        .await
        .as_array()
        .unwrap()
        .iter()
        .any(|i| i["id"] == id));

    let r = app.clone().oneshot(authed_json("PATCH", &format!("/api/admin/playlist/{id}"),
        serde_json::json!({"title":"API Ep 2","url":"https://vod.example.com/api2.mp4","duration_secs":700,"sort_order":11}))).await.unwrap();
    assert_eq!(r.status(), StatusCode::OK);
    assert_eq!(body_json(r).await["title"], "API Ep 2");

    let r = app
        .clone()
        .oneshot(authed_json(
            "POST",
            &format!("/api/admin/playlist/{id}/toggle"),
            serde_json::json!({"active": false}),
        ))
        .await
        .unwrap();
    assert_eq!(r.status(), StatusCode::OK);
    assert_eq!(body_json(r).await["is_active"], false);

    let r = app
        .clone()
        .oneshot(
            Request::builder()
                .method("DELETE")
                .uri(format!("/api/admin/playlist/{id}"))
                .header("Authorization", AUTH)
                .body(Body::empty())
                .unwrap(),
        )
        .await
        .unwrap();
    assert_eq!(r.status(), StatusCode::NO_CONTENT);
}

#[tokio::test]
async fn playlist_get_unknown_is_404() {
    let r = app()
        .await
        .oneshot(authed_get("/api/admin/playlist/999999"))
        .await
        .unwrap();
    assert_eq!(r.status(), StatusCode::NOT_FOUND);
}

#[tokio::test]
async fn playlist_create_zero_duration_is_422() {
    let r = app().await.oneshot(authed_json("POST", "/api/admin/channels/4/playlist",
        serde_json::json!({"title":"X","url":"https://vod.example.com/x.mp4","duration_secs":0}))).await.unwrap();
    assert_eq!(r.status(), StatusCode::UNPROCESSABLE_ENTITY);
}

#[tokio::test]
async fn playlist_test_endpoint_returns_item() {
    // Seed channel 4 has playlist item id 1 ("Episode 1", https://vod.example.com/ep1.mp4,
    // unreachable in tests) → probe fast-fails but the handler still returns the re-fetched item.
    let r = app()
        .await
        .oneshot(
            Request::builder()
                .method("POST")
                .uri("/api/admin/playlist/1/test")
                .header("Authorization", AUTH)
                .body(Body::empty())
                .unwrap(),
        )
        .await
        .unwrap();
    assert_eq!(r.status(), StatusCode::OK);
    let json = body_json(r).await;
    assert_eq!(json["id"], 1);
    assert!(!json["last_checked_at"].is_null());
}
