use axum::{
    extract::{Path, Query, State},
    http::{HeaderMap, StatusCode},
    response::{Json, Response},
};
use serde::{Deserialize, Serialize};

use crate::{
    media::resolver,
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
    pub source_id: Option<i64>,
    pub source_url: Option<String>,
    pub playlist_item_id: Option<i64>,
}

#[derive(Debug, Deserialize)]
pub struct NextQuery {
    pub failed_url: Option<String>,
}

#[derive(Debug, Serialize)]
pub struct PlaylistEntry {
    pub id: i64,
    pub title: String,
    pub duration_secs: i64,
    /// False for disabled items — the ☰ panel renders them dimmed and
    /// non-clickable instead of hiding them (and playback skips them).
    pub available: bool,
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
        ChannelType::VodOnDemand => tune_vod_on_demand(&state, &ch).await,
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
        ChannelType::VodOnDemand => tune_vod_on_demand(&state, &ch).await,
    }
}

pub async fn item(
    State(state): State<AppState>,
    Path((channel_id, item_id)): Path<(i64, i64)>,
) -> Result<Json<TuneResponse>, StatusCode> {
    let ch = channel::get(&state.pool, channel_id)
        .await
        .map_err(|_| StatusCode::INTERNAL_SERVER_ERROR)?
        .ok_or(StatusCode::NOT_FOUND)?;

    let items = playlist_item::list_active_for_channel(&state.pool, channel_id)
        .await
        .map_err(|_| StatusCode::INTERNAL_SERVER_ERROR)?;

    let item = items
        .iter()
        .find(|i| i.id == item_id)
        .ok_or(StatusCode::UNPROCESSABLE_ENTITY)?;

    match resolver::resolve_url(&item.url).await {
        Ok(url) => Ok(tune_response(
            &ch,
            url,
            0,
            resolver::should_skip_proxy(&item.url),
            None,
            Some(item.id),
        )),
        Err(e) => {
            tracing::warn!(url = %item.url, error = %e, "resolver failed for vod item");
            let _ =
                playlist_item::apply_health_result(&state.pool, item, false, Some(&e.to_string()))
                    .await;
            Err(StatusCode::SERVICE_UNAVAILABLE)
        }
    }
}

pub async fn playlist(
    State(state): State<AppState>,
    Path(channel_id): Path<i64>,
) -> Result<Json<Vec<PlaylistEntry>>, StatusCode> {
    channel::get(&state.pool, channel_id)
        .await
        .map_err(|_| StatusCode::INTERNAL_SERVER_ERROR)?
        .ok_or(StatusCode::NOT_FOUND)?;

    let items = playlist_item::list_for_channel(&state.pool, channel_id)
        .await
        .map_err(|_| StatusCode::INTERNAL_SERVER_ERROR)?;

    Ok(Json(
        items
            .into_iter()
            .map(|i| PlaylistEntry {
                id: i.id,
                title: i.title,
                duration_secs: i.duration_secs,
                available: i.is_active,
            })
            .collect(),
    ))
}

fn tune_response(
    ch: &channel::Channel,
    url: String,
    start_offset_secs: i64,
    skip_proxy: bool,
    source: Option<&source::Source>,
    playlist_item_id: Option<i64>,
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
        source_id: source.map(|s| s.id),
        source_url: source.map(|s| s.url.clone()),
        playlist_item_id,
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
        source_id: None,
        source_url: None,
        playlist_item_id: None,
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
        source_id: None,
        source_url: None,
        playlist_item_id: None,
    })
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
        match resolver::resolve_live(&src.url).await {
            Ok(resolver::LiveResolution::Ended) => {
                crate::broadcast::spawn_conversion(
                    state.pool.clone(),
                    ch.id,
                    ch.name.clone(),
                    src.url.clone(),
                );
                return Ok(tune_response_ended(ch));
            }
            Ok(resolver::LiveResolution::Playable { url }) => {
                crate::health::record_source_liveness(&state.pool, src, true).await;
                return Ok(tune_response(
                    ch,
                    url,
                    0,
                    resolver::needs_resolution(&src.url),
                    Some(src),
                    None,
                ));
            }
            Ok(resolver::LiveResolution::Waiting) => {
                // Offline/Upcoming: keep the source ACTIVE so the backoff poll can
                // resume it the moment the stream returns. Persisting "offline" to
                // source health (and the eventual auto-disable) is owned by the
                // liveness-aware background checker — disabling here would drop the
                // source from list_active_for_channel and break resume mid-backoff.
                saw_waiting = true;
            }
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
            resolver::should_skip_proxy(&item.url),
            None,
            Some(item.id),
        )),
        Err(e) => {
            tracing::warn!(url = %item.url, error = %e, "resolver failed for vod item");
            Err(StatusCode::SERVICE_UNAVAILABLE)
        }
    }
}

async fn tune_vod_on_demand(
    state: &AppState,
    ch: &channel::Channel,
) -> Result<Json<TuneResponse>, StatusCode> {
    let items = playlist_item::list_active_for_channel(&state.pool, ch.id)
        .await
        .map_err(|_| StatusCode::INTERNAL_SERVER_ERROR)?;
    let item = items.first().ok_or(StatusCode::SERVICE_UNAVAILABLE)?;
    match resolver::resolve_url(&item.url).await {
        Ok(url) => Ok(tune_response(
            ch,
            url,
            0,
            resolver::should_skip_proxy(&item.url),
            None,
            Some(item.id),
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
            resolver::should_skip_proxy(&item.url),
            None,
            Some(item.id),
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

pub async fn stream_proxy(
    State(state): State<AppState>,
    Query(q): Query<StreamProxyQuery>,
    request_headers: HeaderMap,
) -> Response {
    crate::proxy::fetch_rewritten(&state, q.url, &request_headers).await
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
}
