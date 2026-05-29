use std::sync::Arc;

use axum::body::Body;
use axum::http::{Request, StatusCode};
use mytv::{build_router, config::Config, db, AppState};
use tower::ServiceExt;

async fn app() -> axum::Router {
    let pool = db::connect("sqlite::memory:").await.unwrap();
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
        http_client: reqwest::Client::new(),
    };
    build_router(state)
}

fn req(uri: &str) -> Request<Body> {
    Request::builder().uri(uri).body(Body::empty()).unwrap()
}

fn authed(uri: &str) -> Request<Body> {
    // "user:test" → base64 → "dXNlcjp0ZXN0"
    Request::builder()
        .uri(uri)
        .header("Authorization", "Basic dXNlcjp0ZXN0")
        .body(Body::empty())
        .unwrap()
}

// ── Auth middleware ───────────────────────────────────────────────────────────

#[tokio::test]
async fn test_admin_no_credentials_returns_401() {
    let response = app().await.oneshot(req("/admin")).await.unwrap();
    assert_eq!(response.status(), StatusCode::UNAUTHORIZED);
}

#[tokio::test]
async fn test_admin_wrong_password_returns_401() {
    // "user:wrong" → base64 → "dXNlcjp3cm9uZw=="
    let r = Request::builder()
        .uri("/admin")
        .header("Authorization", "Basic dXNlcjp3cm9uZw==")
        .body(Body::empty())
        .unwrap();
    let response = app().await.oneshot(r).await.unwrap();
    assert_eq!(response.status(), StatusCode::UNAUTHORIZED);
}

#[tokio::test]
async fn test_admin_correct_password_returns_200() {
    // /admin/channels returns 200 for an authenticated request
    let response = app()
        .await
        .oneshot(authed("/admin/channels"))
        .await
        .unwrap();
    assert_eq!(response.status(), StatusCode::OK);
}

// ── Smoke tests ───────────────────────────────────────────────────────────────

#[tokio::test]
async fn test_health_returns_200() {
    let response = app().await.oneshot(req("/health")).await.unwrap();
    assert_eq!(response.status(), StatusCode::OK);
}

#[tokio::test]
async fn test_root_redirects_to_guide() {
    let response = app().await.oneshot(req("/")).await.unwrap();
    assert_eq!(response.status(), StatusCode::PERMANENT_REDIRECT);
    assert_eq!(response.headers().get("location").unwrap(), "/guide");
}

#[tokio::test]
async fn test_guide_returns_200() {
    let response = app().await.oneshot(req("/guide")).await.unwrap();
    assert_eq!(response.status(), StatusCode::OK);
}

#[tokio::test]
async fn test_guide_partial_returns_200() {
    let response = app().await.oneshot(req("/guide/partial")).await.unwrap();
    assert_eq!(response.status(), StatusCode::OK);
}

#[tokio::test]
async fn test_admin_channels_authed_returns_200() {
    let response = app()
        .await
        .oneshot(authed("/admin/channels"))
        .await
        .unwrap();
    assert_eq!(response.status(), StatusCode::OK);
}

// ── Redirect middleware ───────────────────────────────────────────────────────

#[tokio::test]
async fn test_trailing_slash_redirects() {
    // Redirect middleware fires before auth, so no credentials needed.
    let response = app().await.oneshot(req("/admin/channels/")).await.unwrap();
    assert_eq!(response.status(), StatusCode::PERMANENT_REDIRECT);
    assert_eq!(
        response.headers().get("location").unwrap(),
        "/admin/channels"
    );
}
