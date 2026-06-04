use axum::{extract::State, Json};
use serde_json::json;
use std::sync::atomic::Ordering;

use crate::{metrics, AppState};

pub async fn metrics_json(State(state): State<AppState>) -> Json<serde_json::Value> {
    let ssrf_entries = state.ssrf_cache.read().await.len();
    let cors_entries = state.cors_cache.read().await.len();
    Json(json!({
        "rss_bytes": metrics::rss_bytes(),
        "routes": state.metrics.route_snapshots(),
        "proxy": {
            "bytes_proxied": state.metrics.proxy_bytes.load(Ordering::Relaxed),
            "active_streams": state.metrics.active_streams.load(Ordering::Relaxed),
        },
        "caches": {
            "ssrf_entries": ssrf_entries,
            "cors_entries": cors_entries,
        },
    }))
}
