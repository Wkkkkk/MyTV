use askama::Template;
use axum::{
    extract::{Query, State},
    http::StatusCode,
    response::Html,
};
use chrono::{DateTime, Utc};
use serde::Deserialize;

use crate::{
    channel::{self, Channel, ChannelType},
    epg, playlist_item, AppState,
};

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

pub struct ChannelRow {
    pub id: i64,
    pub name: String,
    pub programs: Vec<ProgramSlot>,
}

// ── template structs (fields added in Task 3) ──────────────────────────────

#[derive(Template)]
#[template(path = "guide.html")]
struct GuidePageTemplate {}

#[derive(Template)]
#[template(path = "partials/epg_content.html")]
struct EpgContentTemplate {}

// ── query params ───────────────────────────────────────────────────────────

#[derive(Debug, Deserialize)]
pub struct GuideQuery {
    pub category: Option<String>,
    pub offset: Option<i64>,
}

// ── pure helpers ───────────────────────────────────────────────────────────

/// Returns (window_start, window_end) for the EPG grid.
/// offset_hours: hours from now to window start (default -2 centers "now" in a 4-hour window).
/// Window is always 4 hours wide.
pub fn compute_window(now_secs: i64, offset_hours: i64) -> (DateTime<Utc>, DateTime<Utc>) {
    let start_secs = now_secs + offset_hours * 3600;
    let end_secs = start_secs + 4 * 3600;
    let window_start = DateTime::from_timestamp(start_secs, 0).expect("valid timestamp");
    let window_end = DateTime::from_timestamp(end_secs, 0).expect("valid timestamp");
    (window_start, window_end)
}

/// Converts a ProgramEntry to a ProgramSlot with percentage positioning within the window.
/// Returns None if the entry is completely outside [window_start, window_end).
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

/// Returns the "now" line position as a percentage of the window, or None if outside.
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

/// Returns hourly time labels for the visible window, each with a left percentage.
pub fn time_labels(window_start: DateTime<Utc>, window_end: DateTime<Utc>) -> Vec<TimeLabel> {
    let window_secs = (window_end - window_start).num_seconds() as f64;
    let start_ts = window_start.timestamp();
    let end_ts = window_end.timestamp();
    let rem = start_ts.rem_euclid(3600);
    let first_tick = if rem == 0 { start_ts } else { start_ts + (3600 - rem) };
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

// ── stub handlers (replaced in Task 3) ────────────────────────────────────

pub async fn guide_page(
    State(_state): State<AppState>,
    Query(_params): Query<GuideQuery>,
) -> Result<Html<String>, StatusCode> {
    let html = GuidePageTemplate {}
        .render()
        .map_err(|e| {
            tracing::error!("template render error: {e}");
            StatusCode::INTERNAL_SERVER_ERROR
        })?;
    Ok(Html(html))
}

pub async fn guide_partial(
    State(_state): State<AppState>,
    Query(_params): Query<GuideQuery>,
) -> Result<Html<String>, StatusCode> {
    let html = EpgContentTemplate {}
        .render()
        .map_err(|e| {
            tracing::error!("template render error: {e}");
            StatusCode::INTERNAL_SERVER_ERROR
        })?;
    Ok(Html(html))
}

// ── tests ──────────────────────────────────────────────────────────────────

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

    // window: ts 0–14400 (4 hours at Unix epoch)
    fn w() -> (DateTime<Utc>, DateTime<Utc>) {
        (dt(0), dt(14400))
    }

    // ── compute_window ─────────────────────────────────────────

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

    // ── entry_to_slot ──────────────────────────────────────────

    #[test]
    fn test_entry_to_slot_fully_within_window() {
        let (ws, we) = w();
        let e = make_entry(1, 3600, 7200, false);
        let slot = entry_to_slot(&e, ws, we).unwrap();
        assert!((slot.left_pct - 25.0).abs() < 0.01, "left={}", slot.left_pct);
        assert!((slot.width_pct - 25.0).abs() < 0.01, "width={}", slot.width_pct);
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
        assert!((slot.width_pct - 25.0).abs() < 0.01, "width={}", slot.width_pct);
    }

    #[test]
    fn test_entry_to_slot_clipped_right() {
        let (ws, we) = w();
        let e = make_entry(1, 10800, 18000, false);
        let slot = entry_to_slot(&e, ws, we).unwrap();
        assert!((slot.left_pct - 75.0).abs() < 0.01, "left={}", slot.left_pct);
        assert!((slot.width_pct - 25.0).abs() < 0.01, "width={}", slot.width_pct);
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

    // ── now_line_pct ───────────────────────────────────────────

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
    fn test_now_line_pct_at_window_start() {
        let (ws, we) = w();
        // Exactly at window_start → 0%
        let pct = now_line_pct(dt(0), ws, we).unwrap();
        assert!((pct - 0.0).abs() < 0.01, "pct={}", pct);
    }

    #[test]
    fn test_now_line_pct_at_window_end_returns_none() {
        let (ws, we) = w();
        // Exactly at window_end → None (half-open convention)
        assert!(now_line_pct(dt(14400), ws, we).is_none());
    }

    #[test]
    fn test_entry_to_slot_touching_window_start() {
        let (ws, we) = w();
        // Entry ends exactly at window_start → excluded (half-open [start, end))
        assert!(entry_to_slot(&make_entry(1, -3600, 0, false), ws, we).is_none());
        // Entry starts exactly at window_start → included
        let slot = entry_to_slot(&make_entry(1, 0, 3600, false), ws, we).unwrap();
        assert!((slot.left_pct - 0.0).abs() < 0.01);
    }

    // ── time_labels ────────────────────────────────────────────

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
        // window 00:30–04:30 UTC (ts 1800–16200)
        let labels = time_labels(dt(1800), dt(16200));
        assert_eq!(labels.len(), 4, "expected 4 labels, got {}", labels.len());
        assert_eq!(labels[0].label, "01:00");
        // (3600-1800)/(16200-1800)*100 = 12.5%
        assert!((labels[0].left_pct - 12.5).abs() < 0.01);
    }
}
