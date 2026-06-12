use axum::{
    body::Bytes,
    extract::{Path, Query, State},
    http::{HeaderMap, HeaderValue, StatusCode},
    response::{IntoResponse, Json, Response},
};
use futures_util::StreamExt;
use serde::{Deserialize, Serialize};
use std::sync::atomic::Ordering;

use crate::{
    media::{hls, mpd, resolver},
    model::{
        channel::{self, ChannelType},
        playlist_item, source,
    },
    AppState,
};

#[derive(Debug, Serialize)]
pub struct TuneResponse {
    pub url: String,
    pub start_offset_secs: i64,
    pub name: String,
    pub logo_url: Option<String>,
    pub category: String,
    pub channel_type: String,
    pub skip_proxy: bool,
    pub ended: bool,
    pub waiting: bool,
}

#[derive(Debug, Deserialize)]
pub struct NextQuery {
    pub failed_url: Option<String>,
}

pub async fn tune(
    State(state): State<AppState>,
    Path(channel_id): Path<i64>,
) -> Result<Json<TuneResponse>, StatusCode> {
    let ch = channel::get(&state.pool, channel_id)
        .await
        .map_err(|_| StatusCode::INTERNAL_SERVER_ERROR)?
        .ok_or(StatusCode::NOT_FOUND)?;

    match ch.channel_type() {
        ChannelType::Live => next_live(&state, &ch, None).await,
        ChannelType::VodLoop => {
            let now_secs = chrono::Utc::now().timestamp();
            tune_vod_at(&state, &ch, now_secs).await
        }
    }
}

pub async fn next(
    State(state): State<AppState>,
    Path(channel_id): Path<i64>,
    Query(q): Query<NextQuery>,
) -> Result<Json<TuneResponse>, StatusCode> {
    let ch = channel::get(&state.pool, channel_id)
        .await
        .map_err(|_| StatusCode::INTERNAL_SERVER_ERROR)?
        .ok_or(StatusCode::NOT_FOUND)?;

    match ch.channel_type() {
        ChannelType::Live => next_live(&state, &ch, q.failed_url.as_deref()).await,
        ChannelType::VodLoop => {
            let now_secs = chrono::Utc::now().timestamp();
            next_vod_at(&state, &ch, now_secs).await
        }
    }
}

fn tune_response(
    ch: &channel::Channel,
    url: String,
    start_offset_secs: i64,
    skip_proxy: bool,
) -> Json<TuneResponse> {
    Json(TuneResponse {
        url,
        start_offset_secs,
        name: ch.name.clone(),
        logo_url: ch.logo_url.clone(),
        category: ch.category.clone(),
        channel_type: ch.r#type.clone(),
        skip_proxy,
        ended: false,
        waiting: false,
    })
}

fn tune_response_ended(ch: &channel::Channel) -> Json<TuneResponse> {
    Json(TuneResponse {
        url: String::new(),
        start_offset_secs: 0,
        name: ch.name.clone(),
        logo_url: ch.logo_url.clone(),
        category: ch.category.clone(),
        channel_type: ch.r#type.clone(),
        skip_proxy: false,
        ended: true,
        waiting: false,
    })
}

fn tune_response_waiting(ch: &channel::Channel) -> Json<TuneResponse> {
    Json(TuneResponse {
        url: String::new(),
        start_offset_secs: 0,
        name: ch.name.clone(),
        logo_url: ch.logo_url.clone(),
        category: ch.category.clone(),
        channel_type: ch.r#type.clone(),
        skip_proxy: false,
        ended: false,
        waiting: true,
    })
}

/// A live source is "ended" when yt-dlp reports the broadcast finished
/// (was_live: recording processed; post_live: just ended) or, as a fallback
/// for extractors without live_status, when the resolved manifest carries the
/// force_finished marker.
fn is_ended_live(status: resolver::LiveStatus, resolved_url: &str) -> bool {
    matches!(
        status,
        resolver::LiveStatus::WasLive | resolver::LiveStatus::PostLive
    ) || resolver::is_finished_live(resolved_url)
}

enum LiveOutcome {
    Play,
    Ended,
    Waiting,
}

/// Maps a resolver result to the action `next_live` takes. `None` means "not
/// usable" (a genuine failure, or an empty URL whose status is not
/// offline/upcoming) — the caller should try the next source.
fn classify_live_outcome(url: &str, status: resolver::LiveStatus) -> Option<LiveOutcome> {
    if is_ended_live(status, url) {
        return Some(LiveOutcome::Ended);
    }
    if url.is_empty() {
        return matches!(
            status,
            resolver::LiveStatus::Offline | resolver::LiveStatus::Upcoming(_)
        )
        .then_some(LiveOutcome::Waiting);
    }
    Some(LiveOutcome::Play)
}

async fn next_live(
    state: &AppState,
    ch: &channel::Channel,
    failed_url: Option<&str>,
) -> Result<Json<TuneResponse>, StatusCode> {
    let sources = source::list_tunable_for_channel(&state.pool, ch.id)
        .await
        .map_err(|_| StatusCode::INTERNAL_SERVER_ERROR)?;

    let mut saw_waiting = false;
    for src in sources
        .iter()
        .filter(|s| Some(s.url.as_str()) != failed_url)
    {
        match resolver::resolve_url_with_status(&src.url).await {
            Ok((url, status)) => match classify_live_outcome(&url, status) {
                Some(LiveOutcome::Ended) => {
                    spawn_live_to_vod_conversion(state, ch.id, ch.name.clone(), src.url.clone());
                    return Ok(tune_response_ended(ch));
                }
                Some(LiveOutcome::Play) => {
                    crate::health::record_source_liveness(&state.pool, src, true).await;
                    return Ok(tune_response(
                        ch,
                        url,
                        0,
                        resolver::needs_resolution(&src.url),
                    ));
                }
                Some(LiveOutcome::Waiting) => {
                    // Offline/Upcoming: keep the source ACTIVE so the backoff poll can
                    // resume it the moment the stream returns. Persisting "offline" to
                    // source health (and the eventual auto-disable) is owned by the
                    // liveness-aware background checker — disabling here would drop the
                    // source from list_active_for_channel and break resume mid-backoff.
                    saw_waiting = true;
                }
                None => {
                    tracing::warn!(url = %src.url, ?status, "resolver returned no usable URL")
                }
            },
            Err(e) => {
                // Liveness lifecycle (disable/re-enable) is owned by the background
                // health checker; the tune path only ever confirms liveness, never
                // penalizes — so a hard resolver error just skips to the next source.
                tracing::warn!(url = %src.url, error = %e, "resolver failed, trying next source")
            }
        }
    }

    if saw_waiting {
        Ok(tune_response_waiting(ch))
    } else {
        Err(StatusCode::SERVICE_UNAVAILABLE)
    }
}

/// DB-only conversion of an ended live channel into a VOD loop: append the
/// recording as a playlist item, flip the channel to vod_loop anchored at
/// `anchor`, and deactivate the (now-finished) live sources. Idempotent: a
/// channel already in vod_loop is left untouched.
async fn convert_channel_to_vod_loop(
    pool: &sqlx::SqlitePool,
    channel_id: i64,
    title: &str,
    watch_url: &str,
    duration_secs: i64,
    anchor: chrono::DateTime<chrono::Utc>,
) -> anyhow::Result<()> {
    // Atomic claim first: only the caller that flips live→vod_loop proceeds to
    // append the recording. A concurrent or repeat tune (two tunes can both
    // observe the ended broadcast before either converts) loses the claim and
    // exits here, so the recording is never inserted twice. The flip is also the
    // idempotency gate — an already-converted channel is left untouched.
    if !channel::set_type_and_anchor_if_live(pool, channel_id, anchor).await? {
        return Ok(());
    }
    playlist_item::create(
        pool,
        playlist_item::NewPlaylistItem {
            channel_id,
            title: title.to_string(),
            url: watch_url.to_string(),
            duration_secs,
            sort_order: 0,
        },
    )
    .await?;
    source::deactivate_all_for_channel(pool, channel_id).await?;
    Ok(())
}

fn spawn_live_to_vod_conversion(
    state: &AppState,
    channel_id: i64,
    channel_name: String,
    source_url: String,
) {
    let pool = state.pool.clone();
    tokio::spawn(async move {
        if let Err(e) = live_to_vod_conversion(&pool, channel_id, &channel_name, &source_url).await
        {
            tracing::warn!(channel_id, error = %e, "ended-live → VOD conversion failed");
        }
    });
}

async fn live_to_vod_conversion(
    pool: &sqlx::SqlitePool,
    channel_id: i64,
    channel_name: &str,
    source_url: &str,
) -> anyhow::Result<()> {
    let watch_url = match resolver::live_url_to_watch_url(source_url) {
        Some(u) => u,
        None => {
            let id = resolver::fetch_video_id(source_url).await?;
            format!("https://www.youtube.com/watch?v={id}")
        }
    };
    let duration = resolver::fetch_duration_secs(&watch_url).await?;
    convert_channel_to_vod_loop(
        pool,
        channel_id,
        channel_name,
        &watch_url,
        duration,
        chrono::Utc::now(),
    )
    .await
}

async fn vod_items_and_index(
    state: &AppState,
    ch: &channel::Channel,
    now_secs: i64,
) -> Result<(Vec<playlist_item::PlaylistItem>, usize, i64), StatusCode> {
    let anchor_secs = ch
        .loop_anchor
        .ok_or(StatusCode::INTERNAL_SERVER_ERROR)?
        .timestamp();

    let items = playlist_item::list_active_for_channel(&state.pool, ch.id)
        .await
        .map_err(|_| StatusCode::INTERNAL_SERVER_ERROR)?;

    if items.is_empty() {
        return Err(StatusCode::SERVICE_UNAVAILABLE);
    }

    let (idx, offset) = playlist_item::current_position(&items, now_secs, anchor_secs)
        .ok_or(StatusCode::SERVICE_UNAVAILABLE)?;
    Ok((items, idx, offset))
}

async fn tune_vod_at(
    state: &AppState,
    ch: &channel::Channel,
    now_secs: i64,
) -> Result<Json<TuneResponse>, StatusCode> {
    let (items, idx, offset) = vod_items_and_index(state, ch, now_secs).await?;
    let item = &items[idx];
    match resolver::resolve_url(&item.url).await {
        Ok(url) => Ok(tune_response(
            ch,
            url,
            offset,
            resolver::needs_resolution(&item.url),
        )),
        Err(e) => {
            tracing::warn!(url = %item.url, error = %e, "resolver failed for vod item");
            Err(StatusCode::SERVICE_UNAVAILABLE)
        }
    }
}

async fn next_vod_at(
    state: &AppState,
    ch: &channel::Channel,
    now_secs: i64,
) -> Result<Json<TuneResponse>, StatusCode> {
    let (items, idx, _) = vod_items_and_index(state, ch, now_secs).await?;
    let next_idx = (idx + 1) % items.len();
    let item = &items[next_idx];
    match resolver::resolve_url(&item.url).await {
        Ok(url) => Ok(tune_response(
            ch,
            url,
            0,
            resolver::needs_resolution(&item.url),
        )),
        Err(e) => {
            tracing::warn!(url = %item.url, error = %e, "resolver failed for vod item");
            Err(StatusCode::SERVICE_UNAVAILABLE)
        }
    }
}

// ── stream proxy ──────────────────────────────────────────────────────────

#[derive(Deserialize)]
pub struct StreamProxyQuery {
    pub url: String,
}

fn resolve_location(location: &str, base_url: &str) -> Option<String> {
    if location.starts_with("http://") || location.starts_with("https://") {
        return Some(location.to_string());
    }
    reqwest::Url::parse(base_url)
        .ok()?
        .join(location)
        .ok()
        .map(|u| u.to_string())
}

fn detect_playlist(content_type: &str, url: &str) -> bool {
    let lower = url.to_lowercase();
    let path = &lower[..lower.find('?').unwrap_or(lower.len())];
    let is_dash = content_type.contains("dash+xml") || path.ends_with(".mpd");
    is_dash || content_type.contains("mpegurl") || path.ends_with(".m3u8") || path.ends_with(".m3u")
}

async fn resolve_direct_segments(state: &AppState, base_url: &str) -> bool {
    let host_key = crate::media::hls::extract_manifest_host(base_url);
    {
        let cache = state.cors_cache.read().await;
        if let Some(&cached) = cache.get(&host_key) {
            return cached;
        }
    }
    // Cache miss: delegate to health::probe_and_cache_cors.
    // Re-fetches the manifest internally; cache misses are rare (once per host per session).
    crate::health::probe_and_cache_cors(&state.http_client, &state.cors_cache, base_url)
        .await
        .unwrap_or(false)
}

pub async fn stream_proxy(
    State(state): State<AppState>,
    Query(q): Query<StreamProxyQuery>,
    request_headers: HeaderMap,
) -> Response {
    if !q.url.starts_with("http://") && !q.url.starts_with("https://") {
        return StatusCode::BAD_REQUEST.into_response();
    }

    let mut url = q.url;
    let mut upstream = None;

    for _ in 0..5 {
        // DNS resolved at check time; a hostile server can rebind between check and connect (TOCTOU).
        if let Err(e) = crate::ssrf::is_safe_url_cached(&url, &state.ssrf_cache).await {
            tracing::warn!(url = %url, reason = %e, "stream proxy SSRF check failed");
            return StatusCode::UNPROCESSABLE_ENTITY.into_response();
        }
        let mut req = state.proxy_client.get(&url);
        if let Some(range) = request_headers.get(axum::http::header::RANGE) {
            req = req.header(axum::http::header::RANGE, range);
        }
        let resp = match req.send().await {
            Ok(r) => r,
            Err(e) => {
                tracing::warn!(url = %url, error = %e, "stream proxy fetch failed");
                return StatusCode::BAD_GATEWAY.into_response();
            }
        };
        if resp.status().is_redirection() {
            let location = match resp
                .headers()
                .get(axum::http::header::LOCATION)
                .and_then(|v| v.to_str().ok())
            {
                Some(loc) => loc.to_string(),
                None => return StatusCode::BAD_GATEWAY.into_response(),
            };
            url = match resolve_location(&location, &url) {
                Some(resolved) => resolved,
                None => return StatusCode::BAD_GATEWAY.into_response(),
            };
            continue;
        }
        upstream = Some(resp);
        break;
    }

    let mut upstream = match upstream {
        Some(r) => r,
        None => return StatusCode::BAD_GATEWAY.into_response(),
    };

    let status = upstream.status();

    let ct = upstream
        .headers()
        .get(axum::http::header::CONTENT_TYPE)
        .and_then(|v| v.to_str().ok())
        .unwrap_or("")
        .to_owned();

    let is_playlist = detect_playlist(&ct, &url);

    // RFC 7230 §6.1: collect headers listed in Connection so we can strip them too.
    let connection_options: Vec<String> = upstream
        .headers()
        .get(axum::http::header::CONNECTION)
        .and_then(|v| v.to_str().ok())
        .map(|s| s.split(',').map(|t| t.trim().to_lowercase()).collect())
        .unwrap_or_default();

    let mut headers = HeaderMap::new();
    headers.insert(
        axum::http::header::ACCESS_CONTROL_ALLOW_ORIGIN,
        HeaderValue::from_static("*"),
    );
    for (key, val) in upstream.headers() {
        // Never forward CORS header (we own it) or hop-by-hop headers (RFC 7230 §6.1).
        if key == axum::http::header::ACCESS_CONTROL_ALLOW_ORIGIN
            || key == axum::http::header::CONNECTION
            || key == axum::http::header::TRANSFER_ENCODING
            || key == axum::http::header::TE
            || key == axum::http::header::TRAILER
            || key == axum::http::header::UPGRADE
            || connection_options.iter().any(|o| o == key.as_str())
        {
            continue;
        }
        headers.append(key.clone(), val.clone());
    }

    if is_playlist {
        headers.remove(axum::http::header::CONTENT_LENGTH);
        const MAX_BODY: usize = 20 * 1024 * 1024;
        let mut collected: Vec<u8> = Vec::new();
        loop {
            match upstream.chunk().await {
                Ok(Some(chunk)) => {
                    if collected.len() + chunk.len() > MAX_BODY {
                        tracing::warn!(url = %url, "stream proxy response exceeds 20 MB cap");
                        return StatusCode::BAD_GATEWAY.into_response();
                    }
                    collected.extend_from_slice(&chunk);
                }
                Ok(None) => break,
                Err(e) => {
                    tracing::warn!(url = %url, error = %e, "stream proxy read failed");
                    return StatusCode::BAD_GATEWAY.into_response();
                }
            }
        }
        let body_bytes = Bytes::from(collected);
        state
            .metrics
            .proxy_bytes
            .fetch_add(body_bytes.len() as u64, Ordering::Relaxed);
        let text = String::from_utf8_lossy(&body_bytes);
        let direct = resolve_direct_segments(&state, &url).await;
        let is_dash_playlist = ct.contains("dash+xml") || url.to_lowercase().ends_with(".mpd");
        let (rewritten, content_type) = if is_dash_playlist {
            (
                mpd::rewrite_mpd_urls(&text, &url, direct),
                "application/dash+xml",
            )
        } else {
            (
                hls::rewrite_hls_urls(&text, &url, direct),
                "application/vnd.apple.mpegurl",
            )
        };
        headers.insert(
            axum::http::header::CONTENT_TYPE,
            HeaderValue::from_static(content_type),
        );
        (status, headers, rewritten).into_response()
    } else {
        // Guard lives inside the closure, so active_streams decrements when the
        // client drops the body stream, not when this handler returns.
        let guard = crate::metrics::ActiveStreamGuard::new(state.metrics.clone());
        let metrics = state.metrics.clone();
        let counted = upstream.bytes_stream().inspect(move |chunk| {
            // Referencing guard makes the move-closure capture it for the stream's lifetime.
            let _hold = &guard;
            if let Ok(c) = chunk {
                metrics
                    .proxy_bytes
                    .fetch_add(c.len() as u64, Ordering::Relaxed);
            }
        });
        (status, headers, axum::body::Body::from_stream(counted)).into_response()
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::{config, db};
    use chrono::DateTime;

    async fn test_state() -> AppState {
        let pool = db::connect("sqlite::memory:").await.unwrap();
        let config = std::sync::Arc::new(config::Config::from_env().unwrap());
        AppState {
            pool,
            config,
            http_client: reqwest::Client::new(),
            proxy_client: reqwest::Client::builder()
                .redirect(reqwest::redirect::Policy::none())
                .build()
                .unwrap(),
            cors_cache: std::sync::Arc::new(tokio::sync::RwLock::new(
                std::collections::HashMap::new(),
            )),
            ssrf_cache: std::sync::Arc::new(tokio::sync::RwLock::new(
                std::collections::HashMap::new(),
            )),
            metrics: std::sync::Arc::new(crate::metrics::Metrics::new()),
            live_cache: std::sync::Arc::new(tokio::sync::RwLock::new(
                std::collections::HashMap::new(),
            )),
        }
    }

    async fn make_live_channel(state: &AppState) -> channel::Channel {
        channel::create(
            &state.pool,
            channel::NewChannel {
                name: "Live Test".into(),
                category: "test".into(),
                logo_url: None,
                channel_type: channel::ChannelType::Live,
                sort_order: 0,
                loop_anchor: None,
            },
        )
        .await
        .unwrap()
    }

    async fn make_vod_channel(state: &AppState, anchor_secs: i64) -> channel::Channel {
        channel::create(
            &state.pool,
            channel::NewChannel {
                name: "VOD Test".into(),
                category: "test".into(),
                logo_url: None,
                channel_type: channel::ChannelType::VodLoop,
                sort_order: 0,
                loop_anchor: Some(DateTime::from_timestamp(anchor_secs, 0).unwrap()),
            },
        )
        .await
        .unwrap()
    }

    // ── tune_live ────────────────────────────────────────────────────────────

    #[tokio::test]
    async fn test_tune_live_returns_primary_hls_source() {
        let state = test_state().await;
        let ch = make_live_channel(&state).await;

        source::create(
            &state.pool,
            source::NewSource {
                channel_id: ch.id,
                kind: source::SourceKind::Hls,
                url: "https://primary.example.com/stream.m3u8".into(),
                priority: 1,
            },
        )
        .await
        .unwrap();

        let result = next_live(&state, &ch, None).await.unwrap();
        assert_eq!(result.url, "https://primary.example.com/stream.m3u8");
        assert_eq!(result.start_offset_secs, 0);
    }

    #[tokio::test]
    async fn test_tune_live_skips_youtube_source_when_ytdlp_unavailable_and_returns_hls_backup() {
        let state = test_state().await;
        let ch = make_live_channel(&state).await;

        source::create(
            &state.pool,
            source::NewSource {
                channel_id: ch.id,
                kind: source::SourceKind::YoutubeLive,
                url: "https://www.youtube.com/watch?v=FAIL_YTDLP_NOT_INSTALLED".into(),
                priority: 1,
            },
        )
        .await
        .unwrap();

        source::create(
            &state.pool,
            source::NewSource {
                channel_id: ch.id,
                kind: source::SourceKind::Hls,
                url: "https://backup.example.com/stream.m3u8".into(),
                priority: 2,
            },
        )
        .await
        .unwrap();

        let result = next_live(&state, &ch, None).await.unwrap();
        assert_eq!(result.url, "https://backup.example.com/stream.m3u8");
    }

    #[tokio::test]
    async fn test_tune_live_returns_503_when_all_sources_fail() {
        let state = test_state().await;
        let ch = make_live_channel(&state).await;

        let err = next_live(&state, &ch, None).await.unwrap_err();
        assert_eq!(err, StatusCode::SERVICE_UNAVAILABLE);
    }

    // ── tune_vod_at ─────────────────────────────────────────────────────────

    #[tokio::test]
    async fn test_tune_vod_returns_correct_url_and_offset() {
        let state = test_state().await;
        let ch = make_vod_channel(&state, 0).await;

        playlist_item::create(
            &state.pool,
            playlist_item::NewPlaylistItem {
                channel_id: ch.id,
                title: "Episode 1".into(),
                url: "https://example.com/ep1.m3u8".into(),
                duration_secs: 3600,
                sort_order: 0,
            },
        )
        .await
        .unwrap();

        let result = tune_vod_at(&state, &ch, 1000).await.unwrap();
        assert_eq!(result.url, "https://example.com/ep1.m3u8");
        assert_eq!(result.start_offset_secs, 1000);
    }

    #[tokio::test]
    async fn test_tune_vod_wraps_to_second_item() {
        let state = test_state().await;
        let ch = make_vod_channel(&state, 0).await;

        playlist_item::create(
            &state.pool,
            playlist_item::NewPlaylistItem {
                channel_id: ch.id,
                title: "A".into(),
                url: "https://example.com/a.m3u8".into(),
                duration_secs: 3600,
                sort_order: 0,
            },
        )
        .await
        .unwrap();

        playlist_item::create(
            &state.pool,
            playlist_item::NewPlaylistItem {
                channel_id: ch.id,
                title: "B".into(),
                url: "https://example.com/b.m3u8".into(),
                duration_secs: 1800,
                sort_order: 1,
            },
        )
        .await
        .unwrap();

        let result = tune_vod_at(&state, &ch, 4000).await.unwrap();
        assert_eq!(result.url, "https://example.com/b.m3u8");
        assert_eq!(result.start_offset_secs, 400);
    }

    #[tokio::test]
    async fn test_tune_vod_returns_503_when_no_playlist() {
        let state = test_state().await;
        let ch = make_vod_channel(&state, 0).await;

        let err = tune_vod_at(&state, &ch, 1000).await.unwrap_err();
        assert_eq!(err, StatusCode::SERVICE_UNAVAILABLE);
    }

    // ── next_live ────────────────────────────────────────────────────────────

    #[tokio::test]
    async fn test_next_live_skips_failed_url_and_returns_backup() {
        let state = test_state().await;
        let ch = make_live_channel(&state).await;

        source::create(
            &state.pool,
            source::NewSource {
                channel_id: ch.id,
                kind: source::SourceKind::Hls,
                url: "https://primary.example.com/stream.m3u8".into(),
                priority: 1,
            },
        )
        .await
        .unwrap();

        source::create(
            &state.pool,
            source::NewSource {
                channel_id: ch.id,
                kind: source::SourceKind::Hls,
                url: "https://backup.example.com/stream.m3u8".into(),
                priority: 2,
            },
        )
        .await
        .unwrap();

        let result = next_live(&state, &ch, Some("https://primary.example.com/stream.m3u8"))
            .await
            .unwrap();
        assert_eq!(result.url, "https://backup.example.com/stream.m3u8");
    }

    #[tokio::test]
    async fn test_next_live_returns_primary_when_no_failed_url() {
        let state = test_state().await;
        let ch = make_live_channel(&state).await;

        source::create(
            &state.pool,
            source::NewSource {
                channel_id: ch.id,
                kind: source::SourceKind::Hls,
                url: "https://primary.example.com/stream.m3u8".into(),
                priority: 1,
            },
        )
        .await
        .unwrap();

        let result = next_live(&state, &ch, None).await.unwrap();
        assert_eq!(result.url, "https://primary.example.com/stream.m3u8");
    }

    // ── next_vod_at ──────────────────────────────────────────────────────────

    #[tokio::test]
    async fn test_next_vod_returns_following_item() {
        let state = test_state().await;
        let ch = make_vod_channel(&state, 0).await;

        playlist_item::create(
            &state.pool,
            playlist_item::NewPlaylistItem {
                channel_id: ch.id,
                title: "A".into(),
                url: "https://example.com/a.m3u8".into(),
                duration_secs: 3600,
                sort_order: 0,
            },
        )
        .await
        .unwrap();

        playlist_item::create(
            &state.pool,
            playlist_item::NewPlaylistItem {
                channel_id: ch.id,
                title: "B".into(),
                url: "https://example.com/b.m3u8".into(),
                duration_secs: 1800,
                sort_order: 1,
            },
        )
        .await
        .unwrap();

        let result = next_vod_at(&state, &ch, 4000).await.unwrap();
        assert_eq!(result.url, "https://example.com/a.m3u8");
        assert_eq!(result.start_offset_secs, 0);
    }

    #[tokio::test]
    async fn test_next_vod_wraps_around_at_end_of_playlist() {
        let state = test_state().await;
        let ch = make_vod_channel(&state, 0).await;

        playlist_item::create(
            &state.pool,
            playlist_item::NewPlaylistItem {
                channel_id: ch.id,
                title: "A".into(),
                url: "https://example.com/a.m3u8".into(),
                duration_secs: 3600,
                sort_order: 0,
            },
        )
        .await
        .unwrap();

        let result = next_vod_at(&state, &ch, 500).await.unwrap();
        assert_eq!(result.url, "https://example.com/a.m3u8");
        assert_eq!(result.start_offset_secs, 0);
    }

    #[tokio::test]
    async fn test_tune_vod_returns_500_when_no_loop_anchor() {
        let state = test_state().await;
        let ch = make_vod_channel(&state, 0).await;
        sqlx::query("UPDATE channels SET loop_anchor = NULL WHERE id = ?")
            .bind(ch.id)
            .execute(&state.pool)
            .await
            .unwrap();
        // Re-fetch the channel
        let ch = channel::get(&state.pool, ch.id).await.unwrap().unwrap();

        let err = tune_vod_at(&state, &ch, 1000).await.unwrap_err();
        assert_eq!(err, StatusCode::INTERNAL_SERVER_ERROR);
    }

    #[tokio::test]
    async fn test_next_live_returns_503_when_only_source_is_failed() {
        let state = test_state().await;
        let ch = make_live_channel(&state).await;

        source::create(
            &state.pool,
            source::NewSource {
                channel_id: ch.id,
                kind: source::SourceKind::Hls,
                url: "https://primary.example.com/stream.m3u8".into(),
                priority: 1,
            },
        )
        .await
        .unwrap();

        // The only source is the failed one — should return 503
        let err = next_live(&state, &ch, Some("https://primary.example.com/stream.m3u8"))
            .await
            .unwrap_err();
        assert_eq!(err, StatusCode::SERVICE_UNAVAILABLE);
    }

    #[tokio::test]
    async fn test_tune_vod_skips_disabled_item() {
        let state = test_state().await;
        let ch = make_vod_channel(&state, 0).await;

        let first = playlist_item::create(
            &state.pool,
            playlist_item::NewPlaylistItem {
                channel_id: ch.id,
                title: "A".into(),
                url: "https://example.com/a.m3u8".into(),
                duration_secs: 3600,
                sort_order: 0,
            },
        )
        .await
        .unwrap();

        playlist_item::create(
            &state.pool,
            playlist_item::NewPlaylistItem {
                channel_id: ch.id,
                title: "B".into(),
                url: "https://example.com/b.m3u8".into(),
                duration_secs: 1800,
                sort_order: 1,
            },
        )
        .await
        .unwrap();

        playlist_item::set_active(&state.pool, first.id, false)
            .await
            .unwrap();

        // Active set = [B] (1800s). Any offset within 1800s lands on B.
        let result = tune_vod_at(&state, &ch, 100).await.unwrap();
        assert_eq!(result.url, "https://example.com/b.m3u8");
    }

    #[tokio::test]
    async fn test_tune_vod_returns_503_when_all_items_disabled() {
        let state = test_state().await;
        let ch = make_vod_channel(&state, 0).await;

        let it = playlist_item::create(
            &state.pool,
            playlist_item::NewPlaylistItem {
                channel_id: ch.id,
                title: "A".into(),
                url: "https://example.com/a.m3u8".into(),
                duration_secs: 3600,
                sort_order: 0,
            },
        )
        .await
        .unwrap();

        playlist_item::set_active(&state.pool, it.id, false)
            .await
            .unwrap();

        let err = tune_vod_at(&state, &ch, 1000).await.unwrap_err();
        assert_eq!(err, StatusCode::SERVICE_UNAVAILABLE);
    }

    #[test]
    fn classify_live_outcome_decision() {
        use crate::media::resolver::LiveStatus::*;
        assert!(matches!(
            classify_live_outcome("https://x/v.m3u8", Live),
            Some(LiveOutcome::Play)
        ));
        assert!(matches!(
            classify_live_outcome("https://x/v.m3u8", Unknown),
            Some(LiveOutcome::Play)
        ));
        assert!(matches!(
            classify_live_outcome("https://x/v.mp4", WasLive),
            Some(LiveOutcome::Ended)
        ));
        assert!(matches!(
            classify_live_outcome("https://x/a/force_finished/1/i.m3u8", Unknown),
            Some(LiveOutcome::Ended)
        ));
        assert!(matches!(
            classify_live_outcome("https://x/v.m3u8", NotLive),
            Some(LiveOutcome::Play)
        ));
        assert!(matches!(
            classify_live_outcome("https://x/v.mp4", PostLive),
            Some(LiveOutcome::Ended)
        ));
        assert!(matches!(
            classify_live_outcome("", Offline),
            Some(LiveOutcome::Waiting)
        ));
        assert!(matches!(
            classify_live_outcome("", Upcoming(None)),
            Some(LiveOutcome::Waiting)
        ));
        assert!(matches!(
            classify_live_outcome("", Upcoming(Some(1234))),
            Some(LiveOutcome::Waiting)
        ));
        assert!(classify_live_outcome("", Unknown).is_none());
    }

    #[test]
    fn is_ended_live_decision() {
        use crate::media::resolver::LiveStatus::*;
        assert!(is_ended_live(WasLive, "https://x.test/v.mp4"));
        assert!(is_ended_live(PostLive, "https://x.test/v.m3u8"));
        assert!(!is_ended_live(Live, "https://x.test/v.m3u8"));
        assert!(!is_ended_live(NotLive, "https://x.test/v.mp4"));
        assert!(!is_ended_live(Unknown, "https://x.test/v.m3u8"));
        assert!(is_ended_live(
            Unknown,
            "https://r5---sn.googlevideo.com/a/force_finished/1/b/index.m3u8"
        ));
    }

    // ── resolve_location ─────────────────────────────────────────────────────

    #[test]
    fn resolve_location_absolute_passthrough() {
        assert_eq!(
            resolve_location(
                "https://cdn.example.com/new.m3u8",
                "https://origin.example.com/old.m3u8",
            ),
            Some("https://cdn.example.com/new.m3u8".to_string())
        );
    }

    #[test]
    fn resolve_location_root_relative() {
        assert_eq!(
            resolve_location("/live/index.m3u8", "https://cdn.example.com/old/path.m3u8"),
            Some("https://cdn.example.com/live/index.m3u8".to_string())
        );
    }

    #[test]
    fn resolve_location_relative_path() {
        assert_eq!(
            resolve_location("index.m3u8", "https://cdn.example.com/live/master.m3u8"),
            Some("https://cdn.example.com/live/index.m3u8".to_string())
        );
    }

    #[test]
    fn resolve_location_unparseable_base_returns_none() {
        assert_eq!(resolve_location("/path", "not-a-url"), None);
    }

    #[test]
    fn resolve_location_protocol_relative() {
        assert_eq!(
            resolve_location(
                "//cdn.example.com/stream.m3u8",
                "https://origin.example.com/master.m3u8",
            ),
            Some("https://cdn.example.com/stream.m3u8".to_string())
        );
    }

    // ── detect_playlist ──────────────────────────────────────────────────────

    #[test]
    fn test_detect_playlist_hls_manifest_by_extension() {
        assert!(detect_playlist(
            "application/octet-stream",
            "https://cdn.example.com/live/index.m3u8"
        ));
    }

    #[test]
    fn test_detect_playlist_hls_manifest_by_content_type() {
        assert!(detect_playlist(
            "application/vnd.apple.mpegurl",
            "https://cdn.example.com/live/stream"
        ));
    }

    #[test]
    fn test_detect_playlist_dash_manifest() {
        assert!(detect_playlist(
            "application/dash+xml",
            "https://cdn.example.com/stream.mpd"
        ));
    }

    #[test]
    fn test_detect_playlist_youtube_live_segment_not_playlist() {
        // YouTube live segment URL has .m3u8 as an intermediate path component,
        // not the final path element — must NOT be treated as a playlist.
        let seg_url = "https://rr1---sn-cgxqc5oqufv-5gos.googlevideo.com/videoplayback/\
            id/abc123/itag/300/source/yt_live_broadcast/\
            playlist/index.m3u8/sq/174385/file/seg.ts";
        assert!(!detect_playlist("application/octet-stream", seg_url));
    }

    #[test]
    fn test_detect_playlist_youtube_live_segment_with_query_not_playlist() {
        let seg_url = "https://cdn.example.com/playlist/index.m3u8/sq/123/file/seg.ts?expire=999";
        assert!(!detect_playlist("application/octet-stream", seg_url));
    }

    #[test]
    fn test_detect_playlist_plain_ts_segment_not_playlist() {
        assert!(!detect_playlist(
            "application/octet-stream",
            "https://cdn.example.com/hls/seg1.ts"
        ));
    }

    // ── convert_channel_to_vod_loop ──────────────────────────────────────────

    #[tokio::test]
    async fn test_convert_channel_to_vod_loop() {
        let state = test_state().await;
        let ch = make_live_channel(&state).await;
        source::create(
            &state.pool,
            source::NewSource {
                channel_id: ch.id,
                kind: source::SourceKind::YoutubeLive,
                url: "https://www.youtube.com/live/abc123".into(),
                priority: 1,
            },
        )
        .await
        .unwrap();

        let anchor = DateTime::from_timestamp(1_700_000_000, 0).unwrap();
        convert_channel_to_vod_loop(
            &state.pool,
            ch.id,
            "Live Test",
            "https://www.youtube.com/watch?v=abc123",
            212,
            anchor,
        )
        .await
        .unwrap();

        let updated = channel::get(&state.pool, ch.id).await.unwrap().unwrap();
        assert_eq!(updated.channel_type(), channel::ChannelType::VodLoop);
        assert_eq!(updated.loop_anchor, Some(anchor));

        let items = playlist_item::list_active_for_channel(&state.pool, ch.id)
            .await
            .unwrap();
        assert_eq!(items.len(), 1);
        assert_eq!(items[0].url, "https://www.youtube.com/watch?v=abc123");
        assert_eq!(items[0].duration_secs, 212);
        assert_eq!(items[0].title, "Live Test");

        assert!(source::list_active_for_channel(&state.pool, ch.id)
            .await
            .unwrap()
            .is_empty());

        // Idempotent: a second run on an already-converted channel is a no-op.
        convert_channel_to_vod_loop(
            &state.pool,
            ch.id,
            "Live Test",
            "https://www.youtube.com/watch?v=abc123",
            212,
            anchor,
        )
        .await
        .unwrap();
        assert_eq!(
            playlist_item::list_active_for_channel(&state.pool, ch.id)
                .await
                .unwrap()
                .len(),
            1,
            "second conversion must not append a duplicate item"
        );
    }

    #[tokio::test]
    async fn test_convert_channel_to_vod_loop_concurrent_appends_once() {
        let state = test_state().await;
        let ch = make_live_channel(&state).await;
        source::create(
            &state.pool,
            source::NewSource {
                channel_id: ch.id,
                kind: source::SourceKind::YoutubeLive,
                url: "https://www.youtube.com/live/abc123".into(),
                priority: 1,
            },
        )
        .await
        .unwrap();

        let anchor = DateTime::from_timestamp(1_700_000_000, 0).unwrap();
        let (a, b) = tokio::join!(
            convert_channel_to_vod_loop(
                &state.pool,
                ch.id,
                "Live Test",
                "https://www.youtube.com/watch?v=abc123",
                212,
                anchor,
            ),
            convert_channel_to_vod_loop(
                &state.pool,
                ch.id,
                "Live Test",
                "https://www.youtube.com/watch?v=abc123",
                212,
                anchor,
            ),
        );
        a.unwrap();
        b.unwrap();

        let items = playlist_item::list_active_for_channel(&state.pool, ch.id)
            .await
            .unwrap();
        assert_eq!(
            items.len(),
            1,
            "two racing conversions must append exactly one item"
        );
    }
}
