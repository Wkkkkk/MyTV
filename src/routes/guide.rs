use askama::Template;
use axum::{
    extract::{Query, State},
    http::StatusCode,
    response::Html,
};
use chrono::{DateTime, Utc};
use serde::Deserialize;
use sqlx::SqlitePool;

use crate::{
    budget::{budget_badge, status_for_url, BudgetStatus},
    epg,
    model::{
        channel::{self, Channel, ChannelType},
        playlist_item,
    },
    AppState,
};
use serde_json;

// ── display types ──────────────────────────────────────────────────────────

pub struct ProgramSlot {
    pub title: String,
    pub is_live: bool,
    pub left_pct: f64,
    pub width_pct: f64,
    pub channel_id: i64,
}

pub struct TimeLabel {
    pub label: String,
    pub left_pct: f64,
}

#[derive(Debug, Clone, Copy, PartialEq)]
pub enum HealthStatus {
    Healthy,
    Down,
    Unknown,
}

pub struct ChannelRow {
    pub name: String,
    pub category_icon: &'static str,
    pub health_badge_class: &'static str,
    pub health_badge_char: &'static str,
    pub budget_badge_class: &'static str,
    pub budget_badge_char: &'static str,
    pub programs: Vec<ProgramSlot>,
}

fn category_icon(category: &str) -> &'static str {
    let c = category.to_lowercase();
    if c.contains("news") {
        return "📰";
    }
    if c.contains("sport") {
        return "⚽";
    }
    if c.contains("movie") || c.contains("film") || c.contains("cinema") {
        return "🎬";
    }
    if c.contains("music") {
        return "🎵";
    }
    if c.contains("kid") || c.contains("child") {
        return "🧒";
    }
    if c.contains("documentary") || c.contains("docu") {
        return "🎥";
    }
    if c.contains("entertainment") {
        return "🎭";
    }
    if c.contains("cooking") || c.contains("food") {
        return "🍳";
    }
    if c.contains("travel") {
        return "✈️";
    }
    if c.contains("science") || c.contains("tech") {
        return "🔬";
    }
    "📺"
}

fn derive_health_status(
    channel_id: i64,
    channel_type: &ChannelType,
    all_source_ids: &std::collections::HashSet<i64>,
    active_source_ids: &std::collections::HashSet<i64>,
) -> HealthStatus {
    match channel_type {
        ChannelType::VodLoop => HealthStatus::Healthy,
        ChannelType::Live => {
            if !all_source_ids.contains(&channel_id) {
                return HealthStatus::Unknown;
            }
            if active_source_ids.contains(&channel_id) {
                HealthStatus::Healthy
            } else {
                HealthStatus::Down
            }
        }
    }
}

fn derive_budget_status(
    channel_id: i64,
    first_active_urls: &std::collections::HashMap<i64, String>,
    cors_cache: &std::collections::HashMap<String, bool>,
) -> BudgetStatus {
    match first_active_urls.get(&channel_id) {
        Some(url) => status_for_url(url, cors_cache),
        None => BudgetStatus::Unknown,
    }
}

fn health_badge(status: HealthStatus) -> (&'static str, &'static str) {
    match status {
        HealthStatus::Healthy => ("health-ok", "●"),
        HealthStatus::Down => ("health-down", "●"),
        HealthStatus::Unknown => ("health-unknown", "○"),
    }
}

// ── template structs ───────────────────────────────────────────────────────

#[derive(Template)]
#[template(path = "guide.html")]
struct GuidePageTemplate {
    categories: Vec<String>,
    active_category: String,
    offset_hours: i64,
    offset_prev: i64,
    offset_next: i64,
    window_label: String,
    labels: Vec<TimeLabel>,
    now_pct: Option<f64>,
    rows: Vec<ChannelRow>,
    channels_json: String,
}

#[derive(Template)]
#[template(path = "partials/epg_content.html")]
struct EpgContentTemplate {
    categories: Vec<String>,
    active_category: String,
    offset_hours: i64,
    offset_prev: i64,
    offset_next: i64,
    window_label: String,
    labels: Vec<TimeLabel>,
    now_pct: Option<f64>,
    rows: Vec<ChannelRow>,
    channels_json: String,
}

// ── query params ───────────────────────────────────────────────────────────

#[derive(Debug, Deserialize)]
pub struct GuideQuery {
    pub category: Option<String>,
    pub offset: Option<i64>,
}

// ── pure helpers (unchanged from Task 2) ──────────────────────────────────

pub fn compute_window(now_secs: i64, offset_hours: i64) -> (DateTime<Utc>, DateTime<Utc>) {
    let start_secs = now_secs + offset_hours * 3600;
    let end_secs = start_secs + 4 * 3600;
    let window_start = DateTime::from_timestamp(start_secs, 0).expect("valid timestamp");
    let window_end = DateTime::from_timestamp(end_secs, 0).expect("valid timestamp");
    (window_start, window_end)
}

pub fn entry_to_slot(
    entry: &epg::ProgramEntry,
    window_start: DateTime<Utc>,
    window_end: DateTime<Utc>,
) -> Option<ProgramSlot> {
    if entry.end_time <= window_start || entry.start_time >= window_end {
        return None;
    }
    let window_secs = (window_end - window_start).num_seconds() as f64;
    let visible_start = entry.start_time.max(window_start);
    let visible_end = entry.end_time.min(window_end);
    let left_secs = (visible_start - window_start).num_seconds() as f64;
    let width_secs = (visible_end - visible_start).num_seconds() as f64;
    Some(ProgramSlot {
        title: entry.title.clone(),
        is_live: entry.is_live,
        left_pct: (left_secs / window_secs * 100.0).clamp(0.0, 100.0),
        width_pct: (width_secs / window_secs * 100.0).clamp(0.0, 100.0),
        channel_id: entry.channel_id,
    })
}

pub fn now_line_pct(
    now: DateTime<Utc>,
    window_start: DateTime<Utc>,
    window_end: DateTime<Utc>,
) -> Option<f64> {
    if now < window_start || now >= window_end {
        return None;
    }
    let window_secs = (window_end - window_start).num_seconds() as f64;
    let elapsed = (now - window_start).num_seconds() as f64;
    Some((elapsed / window_secs * 100.0).clamp(0.0, 100.0))
}

pub fn time_labels(window_start: DateTime<Utc>, window_end: DateTime<Utc>) -> Vec<TimeLabel> {
    let window_secs = (window_end - window_start).num_seconds() as f64;
    let start_ts = window_start.timestamp();
    let end_ts = window_end.timestamp();
    let rem = start_ts.rem_euclid(3600);
    let first_tick = if rem == 0 {
        start_ts
    } else {
        start_ts + (3600 - rem)
    };
    let mut labels = Vec::new();
    let mut ts = first_tick;
    while ts <= end_ts {
        let dt = DateTime::from_timestamp(ts, 0).expect("valid ts");
        let elapsed = (dt - window_start).num_seconds() as f64;
        labels.push(TimeLabel {
            label: dt.format("%H:%M").to_string(),
            left_pct: (elapsed / window_secs * 100.0).clamp(0.0, 100.0),
        });
        ts += 3600;
    }
    labels
}

// ── data builder ───────────────────────────────────────────────────────────

struct GuideData {
    categories: Vec<String>,
    active_category: String,
    offset_hours: i64,
    offset_prev: i64,
    offset_next: i64,
    window_label: String,
    labels: Vec<TimeLabel>,
    now_pct: Option<f64>,
    rows: Vec<ChannelRow>,
    channels_json: String,
}

async fn build_guide_data(
    pool: &SqlitePool,
    cors_cache: &std::collections::HashMap<String, bool>,
    category: &str,
    offset_hours: i64,
) -> anyhow::Result<GuideData> {
    let now = Utc::now();
    let (window_start, window_end) = compute_window(now.timestamp(), offset_hours);

    let all_channels = channel::list(pool).await?;
    let categories = channel::distinct_categories(&all_channels);

    let channels_json = serde_json::to_string(
        &all_channels
            .iter()
            .map(|c| serde_json::json!({"id": c.id, "name": c.name}))
            .collect::<Vec<_>>(),
    )
    .unwrap_or_else(|_| "[]".to_string())
    .replace("</", r"<\/");

    let channels: Vec<Channel> = if category == "all" {
        all_channels
    } else {
        all_channels
            .into_iter()
            .filter(|c| c.category == category)
            .collect()
    };

    let all_source_ids: std::collections::HashSet<i64> =
        sqlx::query_scalar::<_, i64>("SELECT DISTINCT channel_id FROM sources")
            .fetch_all(pool)
            .await?
            .into_iter()
            .collect();

    let active_source_ids: std::collections::HashSet<i64> =
        sqlx::query_scalar::<_, i64>("SELECT DISTINCT channel_id FROM sources WHERE is_active = 1")
            .fetch_all(pool)
            .await?
            .into_iter()
            .collect();

    #[derive(sqlx::FromRow)]
    struct SourceUrlRow {
        channel_id: i64,
        url: String,
    }

    let source_url_rows = sqlx::query_as::<_, SourceUrlRow>(
        "SELECT channel_id, url FROM sources WHERE is_active = 1 ORDER BY channel_id, priority",
    )
    .fetch_all(pool)
    .await?;

    let first_active_urls: std::collections::HashMap<i64, String> = source_url_rows
        .into_iter()
        .fold(std::collections::HashMap::new(), |mut acc, row| {
            acc.entry(row.channel_id).or_insert(row.url);
            acc
        });

    let mut rows = Vec::new();
    for ch in &channels {
        let entries = match ch.channel_type() {
            ChannelType::Live => vec![epg::live_entry(ch.id, &ch.name, window_start, window_end)],
            ChannelType::VodLoop => {
                if let Some(anchor) = ch.loop_anchor {
                    let items = playlist_item::list_for_channel(pool, ch.id).await?;
                    epg::vod_schedule(ch.id, &items, anchor.timestamp(), window_start, window_end)
                } else {
                    vec![]
                }
            }
        };
        let programs: Vec<ProgramSlot> = entries
            .iter()
            .filter_map(|e| entry_to_slot(e, window_start, window_end))
            .collect();
        let health = derive_health_status(
            ch.id,
            &ch.channel_type(),
            &all_source_ids,
            &active_source_ids,
        );
        let budget = derive_budget_status(ch.id, &first_active_urls, cors_cache);
        let (health_badge_class, health_badge_char) = health_badge(health);
        let (budget_badge_class, budget_badge_char) = budget_badge(budget);
        rows.push(ChannelRow {
            name: ch.name.clone(),
            category_icon: category_icon(&ch.category),
            health_badge_class,
            health_badge_char,
            budget_badge_class,
            budget_badge_char,
            programs,
        });
    }

    Ok(GuideData {
        categories,
        active_category: category.to_string(),
        offset_hours,
        offset_prev: offset_hours - 2,
        offset_next: offset_hours + 2,
        window_label: format!(
            "{} – {}",
            window_start.format("%H:%M"),
            window_end.format("%H:%M")
        ),
        labels: time_labels(window_start, window_end),
        now_pct: now_line_pct(now, window_start, window_end),
        rows,
        channels_json,
    })
}

// ── handlers ───────────────────────────────────────────────────────────────

pub async fn guide_page(
    State(state): State<AppState>,
    Query(params): Query<GuideQuery>,
) -> Result<Html<String>, StatusCode> {
    let category = params.category.unwrap_or_else(|| "all".to_string());
    let offset_hours = params.offset.unwrap_or(-2).clamp(-48, 48);

    let cors_snapshot = state.cors_cache.read().await.clone();
    let data = build_guide_data(&state.pool, &cors_snapshot, &category, offset_hours)
        .await
        .map_err(|e| {
            tracing::error!("guide data error: {e}");
            StatusCode::INTERNAL_SERVER_ERROR
        })?;

    let html = GuidePageTemplate {
        categories: data.categories,
        active_category: data.active_category,
        offset_hours: data.offset_hours,
        offset_prev: data.offset_prev,
        offset_next: data.offset_next,
        window_label: data.window_label,
        labels: data.labels,
        now_pct: data.now_pct,
        rows: data.rows,
        channels_json: data.channels_json,
    }
    .render()
    .map_err(|e| {
        tracing::error!("template render error: {e}");
        StatusCode::INTERNAL_SERVER_ERROR
    })?;

    Ok(Html(html))
}

pub async fn guide_partial(
    State(state): State<AppState>,
    Query(params): Query<GuideQuery>,
) -> Result<Html<String>, StatusCode> {
    let category = params.category.unwrap_or_else(|| "all".to_string());
    let offset_hours = params.offset.unwrap_or(-2).clamp(-48, 48);

    let cors_snapshot = state.cors_cache.read().await.clone();
    let data = build_guide_data(&state.pool, &cors_snapshot, &category, offset_hours)
        .await
        .map_err(|e| {
            tracing::error!("guide data error: {e}");
            StatusCode::INTERNAL_SERVER_ERROR
        })?;

    let html = EpgContentTemplate {
        categories: data.categories,
        active_category: data.active_category,
        offset_hours: data.offset_hours,
        offset_prev: data.offset_prev,
        offset_next: data.offset_next,
        window_label: data.window_label,
        labels: data.labels,
        now_pct: data.now_pct,
        rows: data.rows,
        channels_json: data.channels_json,
    }
    .render()
    .map_err(|e| {
        tracing::error!("template render error: {e}");
        StatusCode::INTERNAL_SERVER_ERROR
    })?;

    Ok(Html(html))
}

// ── tests (all 15 from Task 2 — unchanged) ────────────────────────────────

#[cfg(test)]
mod tests {
    use super::*;

    fn dt(secs: i64) -> DateTime<Utc> {
        DateTime::from_timestamp(secs, 0).unwrap()
    }

    fn make_entry(channel_id: i64, start: i64, end: i64, is_live: bool) -> epg::ProgramEntry {
        epg::ProgramEntry {
            channel_id,
            title: "Test".to_string(),
            url: String::new(),
            start_time: dt(start),
            end_time: dt(end),
            is_live,
            start_offset_secs: 0,
        }
    }

    fn w() -> (DateTime<Utc>, DateTime<Utc>) {
        (dt(0), dt(14400))
    }

    #[test]
    fn test_compute_window_default_offset() {
        let now = 100_000i64;
        let (start, end) = compute_window(now, -2);
        assert_eq!(start.timestamp(), now - 7200);
        assert_eq!(end.timestamp(), now + 7200);
        assert_eq!((end - start).num_hours(), 4);
    }

    #[test]
    fn test_compute_window_positive_offset() {
        let now = 100_000i64;
        let (start, end) = compute_window(now, 4);
        assert_eq!(start.timestamp(), now + 4 * 3600);
        assert_eq!(end.timestamp(), now + 8 * 3600);
    }

    #[test]
    fn test_entry_to_slot_fully_within_window() {
        let (ws, we) = w();
        let e = make_entry(1, 3600, 7200, false);
        let slot = entry_to_slot(&e, ws, we).unwrap();
        assert!(
            (slot.left_pct - 25.0).abs() < 0.01,
            "left={}",
            slot.left_pct
        );
        assert!(
            (slot.width_pct - 25.0).abs() < 0.01,
            "width={}",
            slot.width_pct
        );
        assert!(!slot.is_live);
    }

    #[test]
    fn test_entry_to_slot_live_flag_preserved() {
        let (ws, we) = w();
        let e = make_entry(1, 0, 14400, true);
        let slot = entry_to_slot(&e, ws, we).unwrap();
        assert!(slot.is_live);
    }

    #[test]
    fn test_entry_to_slot_clipped_left() {
        let (ws, we) = w();
        let e = make_entry(1, -3600, 3600, false);
        let slot = entry_to_slot(&e, ws, we).unwrap();
        assert!((slot.left_pct - 0.0).abs() < 0.01, "left={}", slot.left_pct);
        assert!(
            (slot.width_pct - 25.0).abs() < 0.01,
            "width={}",
            slot.width_pct
        );
    }

    #[test]
    fn test_entry_to_slot_clipped_right() {
        let (ws, we) = w();
        let e = make_entry(1, 10800, 18000, false);
        let slot = entry_to_slot(&e, ws, we).unwrap();
        assert!(
            (slot.left_pct - 75.0).abs() < 0.01,
            "left={}",
            slot.left_pct
        );
        assert!(
            (slot.width_pct - 25.0).abs() < 0.01,
            "width={}",
            slot.width_pct
        );
    }

    #[test]
    fn test_entry_to_slot_entirely_before_window() {
        let (ws, we) = w();
        assert!(entry_to_slot(&make_entry(1, -7200, -3600, false), ws, we).is_none());
    }

    #[test]
    fn test_entry_to_slot_entirely_after_window() {
        let (ws, we) = w();
        assert!(entry_to_slot(&make_entry(1, 18000, 21600, false), ws, we).is_none());
    }

    #[test]
    fn test_now_line_pct_at_midpoint() {
        let (ws, we) = w();
        let pct = now_line_pct(dt(7200), ws, we).unwrap();
        assert!((pct - 50.0).abs() < 0.01, "pct={}", pct);
    }

    #[test]
    fn test_now_line_pct_outside_window() {
        let (ws, we) = w();
        assert!(now_line_pct(dt(-1), ws, we).is_none());
        assert!(now_line_pct(dt(14401), ws, we).is_none());
    }

    #[test]
    fn test_time_labels_aligned_4h_window() {
        let (ws, we) = w();
        let labels = time_labels(ws, we);
        assert_eq!(labels.len(), 5, "expected 5 labels, got {}", labels.len());
        assert_eq!(labels[0].label, "00:00");
        assert!((labels[0].left_pct - 0.0).abs() < 0.01);
        assert_eq!(labels[4].label, "04:00");
        assert!((labels[4].left_pct - 100.0).abs() < 0.01);
    }

    #[test]
    fn test_time_labels_non_aligned_start() {
        let labels = time_labels(dt(1800), dt(16200));
        assert_eq!(labels.len(), 4, "expected 4 labels, got {}", labels.len());
        assert_eq!(labels[0].label, "01:00");
        assert!((labels[0].left_pct - 12.5).abs() < 0.01);
    }

    #[test]
    fn test_now_line_pct_at_window_start() {
        let (ws, we) = w();
        let pct = now_line_pct(dt(0), ws, we).unwrap();
        assert!((pct - 0.0).abs() < 0.01, "pct={}", pct);
    }

    #[test]
    fn test_now_line_pct_at_window_end_returns_none() {
        let (ws, we) = w();
        assert!(now_line_pct(dt(14400), ws, we).is_none());
    }

    #[test]
    fn test_entry_to_slot_touching_window_start() {
        let (ws, we) = w();
        assert!(entry_to_slot(&make_entry(1, -3600, 0, false), ws, we).is_none());
        let slot = entry_to_slot(&make_entry(1, 0, 3600, false), ws, we).unwrap();
        assert!((slot.left_pct - 0.0).abs() < 0.01);
    }

    #[test]
    fn test_category_icon_known_categories() {
        assert_eq!(category_icon("News"), "📰");
        assert_eq!(category_icon("SPORTS"), "⚽");
        assert_eq!(category_icon("Movies"), "🎬");
        assert_eq!(category_icon("Films"), "🎬");
        assert_eq!(category_icon("cinema"), "🎬");
        assert_eq!(category_icon("Music"), "🎵");
        assert_eq!(category_icon("Kids"), "🧒");
        assert_eq!(category_icon("Children"), "🧒");
        assert_eq!(category_icon("Documentary"), "🎥");
        assert_eq!(category_icon("Docu"), "🎥");
        assert_eq!(category_icon("Entertainment"), "🎭");
        assert_eq!(category_icon("Cooking"), "🍳");
        assert_eq!(category_icon("Food"), "🍳");
        assert_eq!(category_icon("Travel"), "✈️");
        assert_eq!(category_icon("Science"), "🔬");
        assert_eq!(category_icon("Tech"), "🔬");
        assert_eq!(category_icon("Unknown"), "📺");
        assert_eq!(category_icon(""), "📺");
    }

    #[test]
    fn test_derive_health_status_live_has_active_source() {
        use std::collections::HashSet;
        let all: HashSet<i64> = [1].into_iter().collect();
        let active: HashSet<i64> = [1].into_iter().collect();
        assert_eq!(
            derive_health_status(1, &ChannelType::Live, &all, &active),
            HealthStatus::Healthy
        );
    }

    #[test]
    fn test_derive_health_status_live_all_inactive() {
        use std::collections::HashSet;
        let all: HashSet<i64> = [1].into_iter().collect();
        let active: HashSet<i64> = HashSet::new();
        assert_eq!(
            derive_health_status(1, &ChannelType::Live, &all, &active),
            HealthStatus::Down
        );
    }

    #[test]
    fn test_derive_health_status_no_sources_unknown() {
        use std::collections::HashSet;
        let all: HashSet<i64> = HashSet::new();
        let active: HashSet<i64> = HashSet::new();
        assert_eq!(
            derive_health_status(1, &ChannelType::Live, &all, &active),
            HealthStatus::Unknown
        );
    }

    #[test]
    fn test_derive_health_status_vod_always_healthy() {
        use std::collections::HashSet;
        let all: HashSet<i64> = [1].into_iter().collect();
        let active: HashSet<i64> = HashSet::new();
        assert_eq!(
            derive_health_status(1, &ChannelType::VodLoop, &all, &active),
            HealthStatus::Healthy
        );
    }

    #[test]
    fn test_derive_health_status_vod_no_sources_still_healthy() {
        use std::collections::HashSet;
        let all: HashSet<i64> = HashSet::new();
        let active: HashSet<i64> = HashSet::new();
        assert_eq!(
            derive_health_status(1, &ChannelType::VodLoop, &all, &active),
            HealthStatus::Healthy
        );
    }

    #[test]
    fn test_derive_budget_status_no_source_unknown() {
        use std::collections::HashMap;
        assert_eq!(
            derive_budget_status(1, &HashMap::new(), &HashMap::new()),
            BudgetStatus::Unknown
        );
    }
}
