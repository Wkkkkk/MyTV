use std::sync::Arc;

use axum::body::Body;
use axum::http::{Request, StatusCode};
use mytv::{build_router, config::Config, db, metrics, AppState};
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

async fn app_with_pool() -> (axum::Router, sqlx::SqlitePool) {
    let pool = db::connect("sqlite::memory:").await.unwrap();
    sqlx::query(include_str!("fixtures/seed.sql"))
        .execute(&pool)
        .await
        .unwrap();
    let state = AppState {
        pool: pool.clone(),
        config: Arc::new(Config {
            database_url: "sqlite::memory:".to_string(),
            admin_password: "test".to_string(),
            youtube_api_key: None,
            port: 0,
        }),
        http_client: test_client(),
        proxy_client: reqwest::Client::builder()
            .redirect(reqwest::redirect::Policy::none())
            .connect_timeout(std::time::Duration::from_millis(500))
            .timeout(std::time::Duration::from_millis(500))
            .build()
            .unwrap(),
        cors_cache: Arc::new(tokio::sync::RwLock::new(std::collections::HashMap::new())),
        ssrf_cache: Arc::new(tokio::sync::RwLock::new(std::collections::HashMap::new())),
        metrics: Arc::new(metrics::Metrics::new()),
        live_cache: Arc::new(tokio::sync::RwLock::new(std::collections::HashMap::new())),
    };
    (build_router(state), pool)
}

async fn app() -> axum::Router {
    app_with_pool().await.0
}

async fn app_for_network() -> axum::Router {
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
        proxy_client: reqwest::Client::builder()
            .redirect(reqwest::redirect::Policy::none())
            .connect_timeout(std::time::Duration::from_secs(5))
            .timeout(std::time::Duration::from_secs(30))
            .build()
            .unwrap(),
        cors_cache: Arc::new(tokio::sync::RwLock::new(std::collections::HashMap::new())),
        ssrf_cache: Arc::new(tokio::sync::RwLock::new(std::collections::HashMap::new())),
        metrics: Arc::new(metrics::Metrics::new()),
        live_cache: Arc::new(tokio::sync::RwLock::new(std::collections::HashMap::new())),
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

fn authed_form_post(uri: &str, body: &str) -> Request<Body> {
    Request::builder()
        .method("POST")
        .uri(uri)
        .header("Authorization", "Basic dXNlcjp0ZXN0")
        .header("content-type", "application/x-www-form-urlencoded")
        .body(Body::from(body.to_string()))
        .unwrap()
}

async fn body_text(response: axum::response::Response) -> String {
    use http_body_util::BodyExt;
    let bytes = response.into_body().collect().await.unwrap().to_bytes();
    String::from_utf8_lossy(&bytes).to_string()
}

async fn app_with_ssrf_bypass(host: &str) -> axum::Router {
    let pool = db::connect("sqlite::memory:").await.unwrap();
    sqlx::query(include_str!("fixtures/seed.sql"))
        .execute(&pool)
        .await
        .unwrap();
    let ssrf_cache = Arc::new(tokio::sync::RwLock::new(std::collections::HashMap::new()));
    ssrf_cache
        .write()
        .await
        .insert(host.to_string(), std::time::Instant::now());
    let state = AppState {
        pool,
        config: Arc::new(Config {
            database_url: "sqlite::memory:".to_string(),
            admin_password: "test".to_string(),
            youtube_api_key: None,
            port: 0,
        }),
        http_client: test_client(),
        proxy_client: reqwest::Client::builder()
            .redirect(reqwest::redirect::Policy::none())
            .connect_timeout(std::time::Duration::from_millis(500))
            .timeout(std::time::Duration::from_millis(500))
            .build()
            .unwrap(),
        cors_cache: Arc::new(tokio::sync::RwLock::new(std::collections::HashMap::new())),
        ssrf_cache,
        metrics: Arc::new(metrics::Metrics::new()),
        live_cache: Arc::new(tokio::sync::RwLock::new(std::collections::HashMap::new())),
    };
    build_router(state)
}

async fn app_with_live_status(
    url: &str,
    status: mytv::media::resolver::LiveStatus,
) -> axum::Router {
    let pool = db::connect("sqlite::memory:").await.unwrap();
    sqlx::query(include_str!("fixtures/seed.sql"))
        .execute(&pool)
        .await
        .unwrap();
    let live_cache: mytv::LiveStatusCache =
        Arc::new(tokio::sync::RwLock::new(std::collections::HashMap::new()));
    live_cache
        .write()
        .await
        .insert(url.to_string(), (status, std::time::Instant::now()));
    let state = AppState {
        pool,
        config: Arc::new(Config {
            database_url: "sqlite::memory:".to_string(),
            admin_password: "test".to_string(),
            youtube_api_key: None,
            port: 0,
        }),
        http_client: test_client(),
        proxy_client: reqwest::Client::builder()
            .redirect(reqwest::redirect::Policy::none())
            .connect_timeout(std::time::Duration::from_millis(500))
            .timeout(std::time::Duration::from_millis(500))
            .build()
            .unwrap(),
        cors_cache: Arc::new(tokio::sync::RwLock::new(std::collections::HashMap::new())),
        ssrf_cache: Arc::new(tokio::sync::RwLock::new(std::collections::HashMap::new())),
        metrics: Arc::new(metrics::Metrics::new()),
        live_cache,
    };
    build_router(state)
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
        proxy_client: reqwest::Client::builder()
            .redirect(reqwest::redirect::Policy::none())
            .connect_timeout(std::time::Duration::from_millis(500))
            .timeout(std::time::Duration::from_millis(500))
            .build()
            .unwrap(),
        cors_cache,
        ssrf_cache: Arc::new(tokio::sync::RwLock::new(std::collections::HashMap::new())),
        metrics: Arc::new(metrics::Metrics::new()),
        live_cache: Arc::new(tokio::sync::RwLock::new(std::collections::HashMap::new())),
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
async fn test_tune_vod_skips_disabled_item() {
    let app = app().await;

    // Seed has channel 4 with items id=1 (ep1) and id=2 (ep2).
    // Disable ep1 via the new toggle endpoint.
    let toggle = app
        .clone()
        .oneshot(authed_post("/admin/playlist/1/toggle"))
        .await
        .unwrap();
    assert_eq!(toggle.status(), StatusCode::SEE_OTHER);

    // Channel 4 now only has ep2 active — tune must return its URL.
    let tune = app.oneshot(req("/channel/4/tune")).await.unwrap();
    assert_eq!(tune.status(), StatusCode::OK);
    let json = body_json(tune).await;
    assert!(
        json["url"].as_str().unwrap().contains("ep2"),
        "disabled ep1 should be skipped; ep2 expected"
    );
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
        body.contains("\u{2715}"),
        "an errored source should render the Down status glyph (\u{2715}), got: {body}"
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

#[tokio::test]
async fn test_guide_excludes_inactive_playlist_items() {
    // Channel 4 (VOD Has Items) has two active items and one inactive YouTube item
    // (seed.sql: "YT Episode", is_active=0). The guide schedule must never include
    // the inactive item regardless of wall-clock time.
    let response = app().await.oneshot(req("/guide")).await.unwrap();
    assert_eq!(response.status(), StatusCode::OK);
    let body = body_text(response).await;
    assert!(
        !body.contains("YT Episode"),
        "guide must not render the inactive playlist item title"
    );
    assert!(
        !body.contains("dQw4w9WgXcQ"),
        "guide must not render the inactive playlist item URL marker"
    );
}

#[tokio::test]
async fn test_source_test_youtube_live_routes_through_resolution() {
    // Source 5 (seed) is a YouTube-live URL -> needs_resolution() is true, so the
    // handler takes the resolve+probe branch. Its bogus video id makes yt-dlp fail
    // fast (or yt-dlp is simply absent), so the probe is a no-op and the badge stays
    // blank, but the handler must still return 200 and re-render the source row partial.
    let response = app()
        .await
        .oneshot(authed_post("/admin/sources/5/test"))
        .await
        .unwrap();
    assert_eq!(response.status(), StatusCode::OK);
    let body = body_text(response).await;
    assert!(
        body.contains("src-row-5"),
        "response should be the row partial for source 5"
    );
    assert!(
        !body.contains('\u{26A1}') && !body.contains('\u{2601}'),
        "badge stays blank when resolution of the bogus id yields no probe result"
    );
}

#[tokio::test]
async fn test_playlist_item_test_returns_row_partial() {
    // Playlist item 1 belongs to VOD channel 4 (https, unreachable in tests).
    let response = app()
        .await
        .oneshot(authed_post("/admin/playlist/1/test"))
        .await
        .unwrap();
    assert_eq!(response.status(), StatusCode::OK);
    let body = body_text(response).await;
    assert!(
        body.contains("pl-row-1"),
        "response should be the playlist row partial"
    );
}

#[tokio::test]
async fn stream_proxy_blocks_loopback() {
    let response = app()
        .await
        .oneshot(req("/stream-proxy?url=http://127.0.0.1/foo"))
        .await
        .unwrap();
    assert_eq!(response.status(), StatusCode::UNPROCESSABLE_ENTITY);
}

#[tokio::test]
async fn stream_proxy_blocks_link_local_metadata() {
    let response = app()
        .await
        .oneshot(req(
            "/stream-proxy?url=http://169.254.169.254/latest/meta-data/",
        ))
        .await
        .unwrap();
    assert_eq!(response.status(), StatusCode::UNPROCESSABLE_ENTITY);
}

#[tokio::test]
async fn stream_proxy_blocks_private_rfc1918() {
    let response = app()
        .await
        .oneshot(req("/stream-proxy?url=http://10.0.0.1/foo"))
        .await
        .unwrap();
    assert_eq!(response.status(), StatusCode::UNPROCESSABLE_ENTITY);
}

#[tokio::test]
async fn stream_proxy_rejects_non_http_scheme() {
    let response = app()
        .await
        .oneshot(req("/stream-proxy?url=file:///etc/passwd"))
        .await
        .unwrap();
    assert_eq!(response.status(), StatusCode::BAD_REQUEST);
}

#[tokio::test]
async fn stream_proxy_strips_hop_by_hop_headers() {
    use tokio::io::{AsyncReadExt, AsyncWriteExt};

    // Upstream that includes hop-by-hop headers in its response.
    let listener = tokio::net::TcpListener::bind("127.0.0.1:0").await.unwrap();
    let port = listener.local_addr().unwrap().port();

    tokio::spawn(async move {
        let (mut conn, _) = listener.accept().await.unwrap();
        let mut buf = [0u8; 512];
        let _ = conn.read(&mut buf).await;
        conn.write_all(
            b"HTTP/1.1 200 OK\r\n\
              Content-Type: application/octet-stream\r\n\
              Transfer-Encoding: chunked\r\n\
              Connection: keep-alive\r\n\
              \r\n\
              5\r\nhello\r\n0\r\n\r\n",
        )
        .await
        .unwrap();
    });

    // Pre-seed ssrf_cache so 127.0.0.1 bypasses the SSRF block.
    let app = app_with_ssrf_bypass("127.0.0.1").await;
    let encoded = format!("http%3A%2F%2F127.0.0.1%3A{}%2F", port);
    let response = app
        .oneshot(req(&format!("/stream-proxy?url={}", encoded)))
        .await
        .unwrap();

    assert_eq!(response.status(), StatusCode::OK);
    assert!(
        response.headers().get("transfer-encoding").is_none(),
        "Transfer-Encoding must be stripped from proxy response"
    );
    assert!(
        response.headers().get("connection").is_none(),
        "Connection must be stripped from proxy response"
    );
}

// ── Admin mutations ──────────────────────────────────────────────────────────

// Channel create

#[tokio::test]
async fn channel_create_redirects_on_success() {
    let response = app()
        .await
        .oneshot(authed_form_post(
            "/admin/channels",
            "name=Test+Channel&category=test&channel_type=live&sort_order=0&logo_url=&loop_anchor=",
        ))
        .await
        .unwrap();
    assert_eq!(response.status(), StatusCode::SEE_OTHER);
    assert_eq!(
        response.headers().get("location").unwrap(),
        "/admin/channels"
    );
}

#[tokio::test]
async fn channel_create_rejects_invalid_type() {
    let response = app()
        .await
        .oneshot(authed_form_post(
            "/admin/channels",
            "name=Test&category=test&channel_type=invalid&sort_order=0&logo_url=&loop_anchor=",
        ))
        .await
        .unwrap();
    assert_eq!(response.status(), StatusCode::UNPROCESSABLE_ENTITY);
}

#[tokio::test]
async fn channel_create_requires_auth() {
    let response = app()
        .await
        .oneshot(
            Request::builder()
                .method("POST")
                .uri("/admin/channels")
                .header("content-type", "application/x-www-form-urlencoded")
                .body(Body::from(
                    "name=Test&category=test&channel_type=live&sort_order=0&logo_url=&loop_anchor=",
                ))
                .unwrap(),
        )
        .await
        .unwrap();
    assert_eq!(response.status(), StatusCode::UNAUTHORIZED);
}

// Channel update

#[tokio::test]
async fn channel_update_redirects_on_success() {
    // Channel 1 ("Live OK") exists in seed data
    let response = app()
        .await
        .oneshot(authed_form_post(
            "/admin/channels/1",
            "name=Updated+Channel&category=test&channel_type=live&sort_order=1&logo_url=&loop_anchor=",
        ))
        .await
        .unwrap();
    assert_eq!(response.status(), StatusCode::SEE_OTHER);
}

#[tokio::test]
async fn channel_update_rejects_invalid_type() {
    let response = app()
        .await
        .oneshot(authed_form_post(
            "/admin/channels/1",
            "name=Updated+Channel&category=test&channel_type=invalid&sort_order=1&logo_url=&loop_anchor=",
        ))
        .await
        .unwrap();
    assert_eq!(response.status(), StatusCode::UNPROCESSABLE_ENTITY);
}

#[tokio::test]
async fn channel_update_returns_404_for_missing_channel() {
    let response = app()
        .await
        .oneshot(authed_form_post(
            "/admin/channels/9999",
            "name=Ghost&category=test&channel_type=live&sort_order=0&logo_url=&loop_anchor=",
        ))
        .await
        .unwrap();
    assert_eq!(response.status(), StatusCode::NOT_FOUND);
}

// Channel delete

#[tokio::test]
async fn channel_delete_redirects_on_success() {
    // Channel 1 exists in seed data
    let response = app()
        .await
        .oneshot(authed_post("/admin/channels/1/delete"))
        .await
        .unwrap();
    assert_eq!(response.status(), StatusCode::SEE_OTHER);
}

#[tokio::test]
async fn channel_delete_returns_404_for_missing_channel() {
    let response = app()
        .await
        .oneshot(authed_post("/admin/channels/9999/delete"))
        .await
        .unwrap();
    assert_eq!(response.status(), StatusCode::NOT_FOUND);
}

// Source create

#[tokio::test]
async fn source_create_redirects_on_success() {
    // Channel 1 exists in seed data
    let response = app()
        .await
        .oneshot(authed_form_post(
            "/admin/channels/1/sources",
            "kind=hls&url=https%3A%2F%2Fexample.com%2Ftest.m3u8&priority=5",
        ))
        .await
        .unwrap();
    assert_eq!(response.status(), StatusCode::SEE_OTHER);
    assert_eq!(
        response.headers().get("location").unwrap(),
        "/admin/channels/1"
    );
}

#[tokio::test]
async fn source_create_rejects_invalid_kind() {
    let response = app()
        .await
        .oneshot(authed_form_post(
            "/admin/channels/1/sources",
            "kind=rtmp&url=https%3A%2F%2Fexample.com%2Fstream&priority=1",
        ))
        .await
        .unwrap();
    assert_eq!(response.status(), StatusCode::UNPROCESSABLE_ENTITY);
}

#[tokio::test]
async fn source_create_rejects_empty_url() {
    let response = app()
        .await
        .oneshot(authed_form_post(
            "/admin/channels/1/sources",
            "kind=hls&url=&priority=1",
        ))
        .await
        .unwrap();
    assert_eq!(response.status(), StatusCode::UNPROCESSABLE_ENTITY);
}

// Source delete

#[tokio::test]
async fn source_delete_redirects_on_success() {
    // Source 1 exists in seed data
    let response = app()
        .await
        .oneshot(authed_post("/admin/sources/1/delete"))
        .await
        .unwrap();
    assert_eq!(response.status(), StatusCode::SEE_OTHER);
}

#[tokio::test]
async fn source_delete_returns_404_for_missing_source() {
    let response = app()
        .await
        .oneshot(authed_post("/admin/sources/9999/delete"))
        .await
        .unwrap();
    assert_eq!(response.status(), StatusCode::NOT_FOUND);
}

// Source toggle

#[tokio::test]
async fn source_toggle_redirects_on_success() {
    // Source 1 exists in seed data
    let response = app()
        .await
        .oneshot(authed_post("/admin/sources/1/toggle"))
        .await
        .unwrap();
    assert_eq!(response.status(), StatusCode::SEE_OTHER);
}

#[tokio::test]
async fn source_toggle_returns_404_for_missing_source() {
    let response = app()
        .await
        .oneshot(authed_post("/admin/sources/9999/toggle"))
        .await
        .unwrap();
    assert_eq!(response.status(), StatusCode::NOT_FOUND);
}

// Playlist item toggle

#[tokio::test]
async fn playlist_item_toggle_redirects_on_success() {
    // Item id=1 (ep1) belongs to channel 4 in seed data.
    let response = app()
        .await
        .oneshot(authed_post("/admin/playlist/1/toggle"))
        .await
        .unwrap();
    assert_eq!(response.status(), StatusCode::SEE_OTHER);
    assert_eq!(
        response
            .headers()
            .get("location")
            .and_then(|v| v.to_str().ok()),
        Some("/admin/channels/4")
    );
}

#[tokio::test]
async fn playlist_item_toggle_returns_404_for_missing_item() {
    let response = app()
        .await
        .oneshot(authed_post("/admin/playlist/9999/toggle"))
        .await
        .unwrap();
    assert_eq!(response.status(), StatusCode::NOT_FOUND);
}

// Channel edit form

#[tokio::test]
async fn channel_edit_form_returns_200() {
    // Channel 1 exists in seed
    let response = app()
        .await
        .oneshot(authed("/admin/channels/1/edit"))
        .await
        .unwrap();
    assert_eq!(response.status(), StatusCode::OK);
}

#[tokio::test]
async fn channel_edit_form_returns_404_for_missing_channel() {
    let response = app()
        .await
        .oneshot(authed("/admin/channels/9999/edit"))
        .await
        .unwrap();
    assert_eq!(response.status(), StatusCode::NOT_FOUND);
}

#[tokio::test]
async fn channel_detail_returns_200_with_budget_badge() {
    // Test with empty CORS cache (Unknown status)
    let response = app()
        .await
        .oneshot(authed("/admin/channels/1"))
        .await
        .unwrap();
    assert_eq!(response.status(), StatusCode::OK);
    let body = body_text(response).await;
    // When CORS cache is empty, budget is Unknown, which renders a dot but no CSS class.
    assert!(
        body.contains("Network budget not yet probed"),
        "channel detail with empty CORS cache should show 'Network budget not yet probed'"
    );

    // Test with CORS cache populated (Direct budget)
    let response = app_with_cors("https://stream.example.com", true)
        .await
        .oneshot(authed("/admin/channels/1"))
        .await
        .unwrap();
    assert_eq!(response.status(), StatusCode::OK);
    let body = body_text(response).await;
    // Channel 1's source host is https://stream.example.com with Direct=true,
    // so budget_badge_class should be "budget-direct"
    assert!(
        body.contains("budget-direct"),
        "channel detail with cached Direct status should show 'budget-direct' class"
    );
}

#[tokio::test]
async fn channel_detail_renders_status_column() {
    let response = app()
        .await
        .oneshot(authed("/admin/channels/1"))
        .await
        .unwrap();
    assert_eq!(response.status(), StatusCode::OK);
    let body = body_text(response).await;
    assert!(
        body.contains("<th>Status</th>"),
        "source table has a Status header"
    );
    assert!(!body.contains("<th>Live</th>"), "old Live header removed");
}

#[tokio::test]
async fn channel_detail_vod_renders_playlist_status_column() {
    let response = app()
        .await
        .oneshot(authed("/admin/channels/4"))
        .await
        .unwrap();
    assert_eq!(response.status(), StatusCode::OK);
    let body = body_text(response).await;
    assert!(body.contains("<th>Status</th>"));
    assert!(
        !body.contains("<th>Active</th>"),
        "old Active header removed"
    );
}

// Discover page

#[tokio::test]
async fn admin_discover_page_returns_200() {
    let response = app()
        .await
        .oneshot(authed("/admin/discover"))
        .await
        .unwrap();
    assert_eq!(response.status(), StatusCode::OK);
}

#[tokio::test]
async fn admin_discover_page_requires_auth() {
    let response = app().await.oneshot(req("/admin/discover")).await.unwrap();
    assert_eq!(response.status(), StatusCode::UNAUTHORIZED);
}

#[tokio::test]
async fn stream_proxy_follows_relative_redirect() {
    use tokio::io::{AsyncReadExt, AsyncWriteExt};

    let listener = tokio::net::TcpListener::bind("127.0.0.1:0").await.unwrap();
    let port = listener.local_addr().unwrap().port();

    tokio::spawn(async move {
        let mut buf = [0u8; 512];

        // First connection: 302 with a relative Location header.
        let (mut conn, _) = listener.accept().await.unwrap();
        let _ = conn.read(&mut buf).await;
        conn.write_all(
            b"HTTP/1.1 302 Found\r\n\
              Location: /redirected.m3u8\r\n\
              Content-Length: 0\r\n\
              \r\n",
        )
        .await
        .unwrap();
        drop(conn);

        // Second connection: the resolved redirect target returns HLS content.
        let (mut conn, _) = listener.accept().await.unwrap();
        let n = conn.read(&mut buf).await.unwrap();
        let request = String::from_utf8_lossy(&buf[..n]);
        assert!(
            request.contains("/redirected.m3u8"),
            "stream_proxy must request the resolved path, got: {request}"
        );
        conn.write_all(
            b"HTTP/1.1 200 OK\r\n\
              Content-Type: application/vnd.apple.mpegurl\r\n\
              Content-Length: 7\r\n\
              \r\n\
              #EXTM3U",
        )
        .await
        .unwrap();
    });

    // app_with_ssrf_bypass pre-seeds 127.0.0.1 in the ssrf_cache so the SSRF
    // check passes for localhost (same pattern as stream_proxy_strips_hop_by_hop_headers).
    let app = app_with_ssrf_bypass("127.0.0.1").await;
    let url_param = format!("http%3A%2F%2F127.0.0.1%3A{}%2Foriginal.m3u8", port);
    let response = app
        .oneshot(req(&format!("/stream-proxy?url={}", url_param)))
        .await
        .unwrap();

    assert_eq!(
        response.status(),
        StatusCode::OK,
        "stream_proxy must follow a relative Location redirect (got {} instead)",
        response.status()
    );
}

#[tokio::test]
async fn test_playlist_item_test_marks_youtube_as_direct_budget() {
    use http_body_util::BodyExt;
    // seed.sql: playlist items ids 1 (ep1) and 2 (ep2) for channel 4,
    // plus YouTube item as id 3 (is_active=0, excluded from VOD loop).
    let resp = app()
        .await
        .oneshot(authed_post("/admin/playlist/3/test"))
        .await
        .unwrap();
    assert_eq!(resp.status(), StatusCode::OK);
    let bytes = resp.into_body().collect().await.unwrap().to_bytes();
    let html = String::from_utf8_lossy(&bytes);
    assert!(
        html.contains('⚡'),
        "YouTube VOD item must show Direct (⚡) budget after Test"
    );
}

// ── Metrics endpoint ──────────────────────────────────────────────────────────

#[tokio::test]
async fn test_metrics_requires_auth() {
    let response = app().await.oneshot(req("/admin/metrics")).await.unwrap();
    assert_eq!(response.status(), StatusCode::UNAUTHORIZED);
}

#[tokio::test]
async fn test_metrics_returns_expected_shape() {
    let response = app().await.oneshot(authed("/admin/metrics")).await.unwrap();
    assert_eq!(response.status(), StatusCode::OK);
    let json = body_json(response).await;
    assert!(json.as_object().unwrap().contains_key("rss_bytes"));
    assert!(json["routes"].is_object());
    assert!(json["proxy"]["bytes_proxied"].is_u64());
    assert!(json["proxy"]["active_streams"].is_u64());
    assert!(json["caches"]["ssrf_entries"].is_u64());
    assert!(json["caches"]["cors_entries"].is_u64());
}

#[tokio::test]
async fn test_metrics_route_counter_increments() {
    // Router clones share AppState (Arc fields), so the /guide hit is visible
    // to the subsequent /admin/metrics request.
    let app = app().await;
    let r = app.clone().oneshot(req("/guide")).await.unwrap();
    assert_eq!(r.status(), StatusCode::OK);
    let response = app.oneshot(authed("/admin/metrics")).await.unwrap();
    let json = body_json(response).await;
    assert_eq!(json["routes"]["/guide"]["count"], 1);
}

#[tokio::test]
async fn test_metrics_counts_proxied_bytes_and_resets_gauge() {
    use tokio::io::{AsyncReadExt, AsyncWriteExt};

    let listener = tokio::net::TcpListener::bind("127.0.0.1:0").await.unwrap();
    let port = listener.local_addr().unwrap().port();

    tokio::spawn(async move {
        let (mut conn, _) = listener.accept().await.unwrap();
        let mut buf = [0u8; 512];
        let _ = conn.read(&mut buf).await;
        conn.write_all(
            b"HTTP/1.1 200 OK\r\n\
              Content-Type: video/mp2t\r\n\
              Content-Length: 10\r\n\
              \r\n\
              0123456789",
        )
        .await
        .unwrap();
    });

    let app = app_with_ssrf_bypass("127.0.0.1").await;
    // .ts path + non-mpegurl content type → non-playlist streaming branch.
    let url_param = format!("http%3A%2F%2F127.0.0.1%3A{}%2Fseg.ts", port);
    let response = app
        .clone()
        .oneshot(req(&format!("/stream-proxy?url={}", url_param)))
        .await
        .unwrap();
    assert_eq!(response.status(), StatusCode::OK);
    let body = body_text(response).await; // fully consume → stream (and gauge guard) dropped
    assert_eq!(body, "0123456789");

    let metrics = body_json(app.oneshot(authed("/admin/metrics")).await.unwrap()).await;
    assert_eq!(metrics["proxy"]["bytes_proxied"], 10);
    assert_eq!(metrics["proxy"]["active_streams"], 0);
}

#[tokio::test]
async fn playlist_item_create_sort_order_skips_gap_after_delete() {
    // After deleting ep1, list_for_channel returns [ep2(sort=2)], len=1.
    // Bug: sort_order = len = 1 < ep2 sort_order=2 → ep3 renders before ep2.
    // Fix: sort_order = max(2)+1 = 3 → ep3 renders after ep2.
    let app = app().await;

    let del = app
        .clone()
        .oneshot(authed_post("/admin/playlist/1/delete"))
        .await
        .unwrap();
    assert_eq!(del.status(), StatusCode::SEE_OTHER);

    let create = app
        .clone()
        .oneshot(authed_form_post(
            "/admin/channels/4/playlist",
            "title=Episode+3&url=https%3A%2F%2Fvod.example.com%2Fep3.mp4&duration_secs=1800",
        ))
        .await
        .unwrap();
    assert_eq!(create.status(), StatusCode::SEE_OTHER);

    let detail = app.oneshot(authed("/admin/channels/4")).await.unwrap();
    let html = body_text(detail).await;

    let pos_ep2 = html
        .find("ep2.mp4")
        .expect("ep2 must appear in the playlist");
    let pos_ep3 = html
        .find("ep3.mp4")
        .expect("ep3 must appear in the playlist");
    assert!(
        pos_ep2 < pos_ep3,
        "ep2 (sort_order=2) must render before ep3 (sort_order=3 with fix, 1 with bug)"
    );
}

#[tokio::test]
async fn test_tune_skip_proxy_false_for_plain_hls() {
    let response = app().await.oneshot(req("/channel/1/tune")).await.unwrap();
    assert_eq!(response.status(), StatusCode::OK);
    let json = body_json(response).await;
    assert_eq!(json["skip_proxy"], false);
}

#[tokio::test]
async fn test_tune_finished_live_returns_ended_and_no_url() {
    let (router, pool) = app_with_pool().await;
    // A resolved URL containing force_finished/1 marks an ended YouTube live.
    // Seed it as a plain HLS source so resolve_url passes it through unchanged
    // (no yt-dlp needed), exercising the ended-detection wiring deterministically.
    // priority 0 so it is tried before channel 1's existing live source.
    sqlx::query(
        "INSERT INTO sources (channel_id, kind, url, priority, is_active) \
         VALUES (1, 'hls', 'https://stream.example.com/ended.m3u8?force_finished/1', 0, 1)",
    )
    .execute(&pool)
    .await
    .unwrap();

    let response = router.oneshot(req("/channel/1/tune")).await.unwrap();
    assert_eq!(response.status(), StatusCode::OK);
    let json = body_json(response).await;
    assert_eq!(json["ended"], serde_json::json!(true));
    assert_eq!(json["url"], serde_json::json!(""));
}

#[tokio::test]
#[ignore = "requires network access — run manually"]
async fn test_stream_proxy_rewrites_dash_bbb_manifest() {
    use http_body_util::BodyExt;
    let app = app_for_network().await;
    let encoded_url = "https%3A%2F%2Fdash.akamaized.net%2Fakamai%2Fbbb_30fps%2Fbbb_30fps.mpd";
    let response = app
        .oneshot(req(&format!("/stream-proxy?url={encoded_url}")))
        .await
        .unwrap();
    assert_eq!(response.status(), StatusCode::OK);
    let ct = response
        .headers()
        .get("content-type")
        .and_then(|v| v.to_str().ok())
        .unwrap_or("");
    assert!(
        ct.contains("dash+xml"),
        "expected dash+xml content-type, got: {ct}"
    );
    let bytes = response.into_body().collect().await.unwrap().to_bytes();
    let body = std::str::from_utf8(&bytes).unwrap();
    // BaseURL "./" resolved to absolute CDN path
    assert!(
        body.contains("https://dash.akamaized.net/akamai/bbb_30fps/"),
        "expected resolved absolute BaseURL in body"
    );
    // Valid DASH XML
    assert!(body.contains("<MPD"), "expected MPD root element");
}

#[tokio::test]
async fn admin_discover_channel_resolve_normalizes_handle() {
    let response = app()
        .await
        .oneshot(authed_form_post(
            "/admin/discover/channel/resolve",
            "url=%40NASA",
        ))
        .await
        .unwrap();
    assert_eq!(response.status(), StatusCode::OK);
    let body = body_text(response).await;
    assert!(body.contains("https://www.youtube.com/@NASA/live"));
}

#[tokio::test]
async fn admin_discover_channel_resolve_rejects_non_youtube() {
    let response = app()
        .await
        .oneshot(authed_form_post(
            "/admin/discover/channel/resolve",
            "url=https%3A%2F%2Fexample.com%2Ffoo",
        ))
        .await
        .unwrap();
    assert_eq!(response.status(), StatusCode::UNPROCESSABLE_ENTITY);
}

#[tokio::test]
async fn admin_live_status_non_youtube_is_neutral() {
    let response = app()
        .await
        .oneshot(authed(
            "/admin/live-status?url=https%3A%2F%2Fexample.com%2Ffoo",
        ))
        .await
        .unwrap();
    assert_eq!(response.status(), StatusCode::OK);
    let body = body_text(response).await;
    assert!(body.contains("Not yet checked"));
    assert!(!body.contains("Currently live"));
}

#[tokio::test]
async fn admin_live_status_requires_auth() {
    let response = app()
        .await
        .oneshot(req(
            "/admin/live-status?url=https%3A%2F%2Fexample.com%2Ffoo",
        ))
        .await
        .unwrap();
    assert_eq!(response.status(), StatusCode::UNAUTHORIZED);
}

#[tokio::test]
async fn admin_live_status_youtube_returns_cached_status() {
    use mytv::media::resolver::LiveStatus;
    let url = "https://www.youtube.com/@LofiGirl/live";
    let app = app_with_live_status(url, LiveStatus::Live).await;
    let encoded = "https%3A%2F%2Fwww.youtube.com%2F%40LofiGirl%2Flive";
    let response = app
        .oneshot(authed(&format!("/admin/live-status?url={encoded}")))
        .await
        .unwrap();
    assert_eq!(response.status(), StatusCode::OK);
    let body = body_text(response).await;
    // The YouTube URL passes needs_resolution, so cached_live_status is consulted
    // and returns the pre-seeded Live status — no yt-dlp invocation.
    assert!(body.contains("Currently live"));
}

#[tokio::test]
async fn admin_channel_resolve_includes_live_badge() {
    let response = app()
        .await
        .oneshot(authed_form_post(
            "/admin/discover/channel/resolve",
            "url=%40NASA",
        ))
        .await
        .unwrap();
    assert_eq!(response.status(), StatusCode::OK);
    let body = body_text(response).await;
    assert!(body.contains("hx-get=\"/admin/live-status?url="));
    assert!(body.contains("youtube.com/%40NASA/live"));
}

async fn app_with_youtube_live_source() -> axum::Router {
    let pool = db::connect("sqlite::memory:").await.unwrap();
    sqlx::query(include_str!("fixtures/seed.sql"))
        .execute(&pool)
        .await
        .unwrap();
    sqlx::query(
        "INSERT INTO sources (channel_id, kind, url, priority, is_active, consecutive_failures) \
         VALUES (1, 'youtube_live', 'https://www.youtube.com/@NASA/live', 9, 1, 0)",
    )
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
        proxy_client: reqwest::Client::builder()
            .redirect(reqwest::redirect::Policy::none())
            .connect_timeout(std::time::Duration::from_millis(500))
            .timeout(std::time::Duration::from_millis(500))
            .build()
            .unwrap(),
        cors_cache: Arc::new(tokio::sync::RwLock::new(std::collections::HashMap::new())),
        ssrf_cache: Arc::new(tokio::sync::RwLock::new(std::collections::HashMap::new())),
        metrics: Arc::new(metrics::Metrics::new()),
        live_cache: Arc::new(tokio::sync::RwLock::new(std::collections::HashMap::new())),
    };
    build_router(state)
}

#[tokio::test]
async fn admin_channel_detail_shows_live_badge_for_youtube_source() {
    let app = app_with_youtube_live_source().await;
    let response = app.oneshot(authed("/admin/channels/1")).await.unwrap();
    assert_eq!(response.status(), StatusCode::OK);
    let body = body_text(response).await;
    assert!(body.contains("hx-get=\"/admin/live-status?url="));
    assert!(body.contains("youtube.com/%40NASA/live"));
}

#[tokio::test]
async fn test_tune_live_includes_source_identity() {
    let response = app().await.oneshot(req("/channel/1/tune")).await.unwrap();
    assert_eq!(response.status(), StatusCode::OK);
    let json = body_json(response).await;
    // Channel 1's only active source is seed source id 1 / live.m3u8.
    assert_eq!(json["source_id"].as_i64().unwrap(), 1);
    assert_eq!(
        json["source_url"].as_str().unwrap(),
        "https://stream.example.com/live.m3u8"
    );
    // A live tune has no playlist item.
    assert!(json["playlist_item_id"].is_null());
}

#[tokio::test]
async fn test_tune_vod_includes_playlist_item_id() {
    let response = app().await.oneshot(req("/channel/4/tune")).await.unwrap();
    assert_eq!(response.status(), StatusCode::OK);
    let json = body_json(response).await;
    // VOD playback comes from a playlist item, not a source.
    assert!(!json["playlist_item_id"].is_null());
    assert!(json["source_id"].is_null());
    assert!(json["source_url"].is_null());
}

#[tokio::test]
async fn test_watch_known_channel_injects_auto_tune() {
    let response = app().await.oneshot(req("/watch/1")).await.unwrap();
    assert_eq!(response.status(), StatusCode::OK);
    let body = body_text(response).await;
    assert!(body.contains("window.__autoTuneChannelId = 1;"));
}

#[tokio::test]
async fn test_watch_unknown_channel_falls_back_to_guide() {
    let response = app().await.oneshot(req("/watch/999999")).await.unwrap();
    assert_eq!(response.status(), StatusCode::OK);
    let body = body_text(response).await;
    assert!(!body.contains("window.__autoTuneChannelId = "));
}

#[tokio::test]
async fn test_guide_has_no_auto_tune() {
    let response = app().await.oneshot(req("/guide")).await.unwrap();
    assert_eq!(response.status(), StatusCode::OK);
    let body = body_text(response).await;
    assert!(!body.contains("window.__autoTuneChannelId = "));
}
