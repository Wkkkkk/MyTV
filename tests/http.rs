use std::sync::Arc;

use axum::body::Body;
use axum::http::{Request, StatusCode};
use mytv::{build_router, config::Config, db, AppState};
use tower::ServiceExt;

// A bounded-timeout client so any test that triggers an outbound request (e.g.
// the source Test endpoint hitting an unreachable seed URL) fails fast instead
// of stalling on the production 5s health-check ceiling in restrictive CI.
fn test_client() -> reqwest::Client {
    reqwest::Client::builder()
        .timeout(std::time::Duration::from_millis(500))
        .build()
        .unwrap()
}

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
        http_client: test_client(),
        cors_cache: Arc::new(tokio::sync::RwLock::new(std::collections::HashMap::new())),
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

async fn body_json(response: axum::response::Response) -> serde_json::Value {
    use http_body_util::BodyExt;
    let bytes = response.into_body().collect().await.unwrap().to_bytes();
    serde_json::from_slice(&bytes).unwrap()
}

fn authed_post(uri: &str) -> Request<Body> {
    Request::builder()
        .method("POST")
        .uri(uri)
        .header("Authorization", "Basic dXNlcjp0ZXN0")
        .body(Body::empty())
        .unwrap()
}

async fn body_text(response: axum::response::Response) -> String {
    use http_body_util::BodyExt;
    let bytes = response.into_body().collect().await.unwrap().to_bytes();
    String::from_utf8_lossy(&bytes).to_string()
}

async fn app_with_cors(host: &str, direct: bool) -> axum::Router {
    let pool = db::connect("sqlite::memory:").await.unwrap();
    sqlx::query(include_str!("fixtures/seed.sql"))
        .execute(&pool)
        .await
        .unwrap();
    let cors_cache = Arc::new(tokio::sync::RwLock::new(std::collections::HashMap::new()));
    cors_cache.write().await.insert(host.to_string(), direct);
    let state = AppState {
        pool,
        config: Arc::new(Config {
            database_url: "sqlite::memory:".to_string(),
            admin_password: "test".to_string(),
            youtube_api_key: None,
            port: 0,
        }),
        http_client: test_client(),
        cors_cache,
    };
    build_router(state)
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

// ── Player contract tests ─────────────────────────────────────────────────────

#[tokio::test]
async fn test_tune_live_ok_returns_stream_url() {
    let response = app().await.oneshot(req("/channel/1/tune")).await.unwrap();
    assert_eq!(response.status(), StatusCode::OK);
    let json = body_json(response).await;
    assert_eq!(
        json["url"].as_str().unwrap(),
        "https://stream.example.com/live.m3u8"
    );
}

#[tokio::test]
async fn test_tune_live_all_sources_down_returns_503() {
    let response = app().await.oneshot(req("/channel/2/tune")).await.unwrap();
    assert_eq!(response.status(), StatusCode::SERVICE_UNAVAILABLE);
}

#[tokio::test]
async fn test_next_live_returns_backup_when_no_failed_url() {
    let response = app().await.oneshot(req("/channel/3/next")).await.unwrap();
    assert_eq!(response.status(), StatusCode::OK);
    let json = body_json(response).await;
    assert_eq!(
        json["url"].as_str().unwrap(),
        "https://stream.example.com/backup.m3u8"
    );
}

#[tokio::test]
async fn test_next_live_all_sources_failed_returns_503() {
    // backup is the only active source; passing it as failed_url leaves nothing
    let response = app()
        .await
        .oneshot(req(
            "/channel/3/next?failed_url=https%3A%2F%2Fstream.example.com%2Fbackup.m3u8",
        ))
        .await
        .unwrap();
    assert_eq!(response.status(), StatusCode::SERVICE_UNAVAILABLE);
}

#[tokio::test]
async fn test_tune_vod_with_playlist_returns_stream_url() {
    let response = app().await.oneshot(req("/channel/4/tune")).await.unwrap();
    assert_eq!(response.status(), StatusCode::OK);
    let json = body_json(response).await;
    // exact episode depends on Utc::now() — assert URL is from the playlist
    assert!(json["url"].as_str().unwrap().contains("vod.example.com/ep"));
}

#[tokio::test]
async fn test_tune_vod_empty_playlist_returns_503() {
    let response = app().await.oneshot(req("/channel/5/tune")).await.unwrap();
    assert_eq!(response.status(), StatusCode::SERVICE_UNAVAILABLE);
}

#[tokio::test]
async fn tune_response_includes_channel_metadata() {
    let app = app().await;
    let resp = app
        .oneshot(
            Request::builder()
                .uri("/channel/1/tune")
                .body(Body::empty())
                .unwrap(),
        )
        .await
        .unwrap();
    assert_eq!(resp.status(), StatusCode::OK);
    let body = axum::body::to_bytes(resp.into_body(), usize::MAX)
        .await
        .unwrap();
    let json: serde_json::Value = serde_json::from_slice(&body).unwrap();
    assert_eq!(json["name"], "Live OK");
    assert_eq!(json["channel_type"], "live");
    assert!(json["url"].as_str().unwrap().contains("live.m3u8"));
    assert!(
        json["logo_url"].is_null(),
        "logo_url should be null for seed channel 1 which has no logo"
    );
    assert!(json["category"].is_string());
}

#[tokio::test]
async fn guide_embeds_epg_channels_json() {
    let app = app().await;
    let resp = app
        .oneshot(
            Request::builder()
                .uri("/guide")
                .body(Body::empty())
                .unwrap(),
        )
        .await
        .unwrap();
    assert_eq!(resp.status(), StatusCode::OK);
    let body = axum::body::to_bytes(resp.into_body(), usize::MAX)
        .await
        .unwrap();
    let html = std::str::from_utf8(&body).unwrap();
    assert!(
        html.contains("window.epgChannels"),
        "missing epgChannels script"
    );
    assert!(
        html.contains("\"Live OK\""),
        "missing channel name in epgChannels"
    );
}

// ── Static file routes ────────────────────────────────────────────────────────

#[tokio::test]
async fn test_favicon_svg() {
    let app = app().await;
    let response = app.oneshot(req("/favicon.svg")).await.unwrap();
    assert_eq!(response.status(), StatusCode::OK);
    assert_eq!(
        response.headers().get("content-type").unwrap(),
        "image/svg+xml",
    );
}

#[tokio::test]
async fn test_manifest_json() {
    let app = app().await;
    let response = app.oneshot(req("/manifest.json")).await.unwrap();
    assert_eq!(response.status(), StatusCode::OK);
    assert_eq!(
        response.headers().get("content-type").unwrap(),
        "application/manifest+json",
    );
}

#[tokio::test]
async fn test_favicon_ico_redirect() {
    let app = app().await;
    let response = app.oneshot(req("/favicon.ico")).await.unwrap();
    assert_eq!(response.status(), StatusCode::PERMANENT_REDIRECT);
    assert_eq!(response.headers().get("location").unwrap(), "/favicon.svg",);
}

// ── Admin Test button / guide budget badge ────────────────────────────────

#[tokio::test]
async fn test_source_test_returns_row_partial_not_ok_badge() {
    // Source 1 (https, unreachable in tests) -> check writes an "error" status.
    let response = app()
        .await
        .oneshot(authed_post("/admin/sources/1/test"))
        .await
        .unwrap();
    assert_eq!(response.status(), StatusCode::OK);
    let body = body_text(response).await;
    assert!(
        body.contains("src-row-1"),
        "response should be the row partial"
    );
    assert!(
        body.contains("\u{25CF}"),
        "row should render a health dot (\u{25CF})"
    );
    assert!(
        !body.contains(">OK<"),
        "old OK badge text must be gone, got: {body}"
    );
}

#[tokio::test]
async fn test_guide_renders_direct_budget_badge_from_cache() {
    // Channel 1's first active source host is https://stream.example.com.
    let response = app_with_cors("https://stream.example.com", true)
        .await
        .oneshot(req("/guide"))
        .await
        .unwrap();
    assert_eq!(response.status(), StatusCode::OK);
    let body = body_text(response).await;
    assert!(
        body.contains("\u{26A1}"),
        "guide should show the direct budget badge (lightning)"
    );
}

#[tokio::test]
async fn test_guide_renders_vod_budget_badge_from_cache() {
    // Channel 4 (VOD) plays items hosted on https://vod.example.com. Only that
    // host is seeded into the cache, so the lightning badge must come from VOD.
    let response = app_with_cors("https://vod.example.com", true)
        .await
        .oneshot(req("/guide"))
        .await
        .unwrap();
    assert_eq!(response.status(), StatusCode::OK);
    let body = body_text(response).await;
    assert!(
        body.contains("\u{26A1}"),
        "guide should show the direct budget badge (lightning) for the VOD channel"
    );
}
