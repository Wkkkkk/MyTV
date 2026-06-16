use chrono::{DateTime, Utc};

use crate::epg;
use crate::model::playlist_item::PlaylistItem;

/// Fixed width (percent of the guide window) of each VOD-on-demand item block.
/// On-demand items have no schedule, so they are laid out left-to-right at this
/// width and clipped at the window edge (~4 visible). Off-edge items remain
/// reachable via the player's playlist panel.
pub(super) const ON_DEMAND_ITEM_WIDTH_PCT: f64 = 25.0;

pub(super) struct ProgramSlot {
    pub title: String,
    pub is_live: bool,
    pub left_pct: f64,
    pub width_pct: f64,
    pub channel_id: i64,
    pub item_id: Option<i64>,
}

pub(super) struct TimeLabel {
    pub label: String,
    pub left_pct: f64,
}

pub(super) fn compute_window(now_secs: i64, offset_hours: i64) -> (DateTime<Utc>, DateTime<Utc>) {
    let start_secs = now_secs + offset_hours * 3600;
    let end_secs = start_secs + 4 * 3600;
    let window_start = DateTime::from_timestamp(start_secs, 0).expect("valid timestamp");
    let window_end = DateTime::from_timestamp(end_secs, 0).expect("valid timestamp");
    (window_start, window_end)
}

pub(super) fn entry_to_slot(
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
        item_id: None,
    })
}

/// Lays out VOD-on-demand items as fixed-width clickable blocks, left-to-right,
/// clipped at the window edge. Empty playlist -> one full-width fallback block
/// that tunes the channel (keeps the row tunable from the guide).
pub(super) fn on_demand_slots(
    channel_id: i64,
    name: &str,
    items: &[PlaylistItem],
    width_pct: f64,
) -> Vec<ProgramSlot> {
    if items.is_empty() {
        return vec![ProgramSlot {
            title: format!("{} — On demand", name),
            is_live: false,
            left_pct: 0.0,
            width_pct: 100.0,
            channel_id,
            item_id: None,
        }];
    }

    let mut slots = Vec::new();
    for (i, item) in items.iter().enumerate() {
        let left = i as f64 * width_pct;
        if left >= 100.0 {
            break;
        }
        slots.push(ProgramSlot {
            title: item.title.clone(),
            is_live: false,
            left_pct: left,
            width_pct: width_pct.min(100.0 - left),
            channel_id,
            item_id: Some(item.id),
        });
    }
    slots
}

pub(super) fn now_line_pct(
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

pub(super) fn time_labels(
    window_start: DateTime<Utc>,
    window_end: DateTime<Utc>,
) -> Vec<TimeLabel> {
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

    fn pl_item(id: i64, title: &str) -> PlaylistItem {
        PlaylistItem {
            id,
            channel_id: 6,
            title: title.to_string(),
            url: format!("https://example.com/{}.mp4", id),
            duration_secs: 120,
            sort_order: id,
            is_active: true,
            last_checked_at: None,
            last_status: None,
            consecutive_failures: 0,
            failure_reason: None,
        }
    }

    #[test]
    fn test_on_demand_slots_empty_returns_single_fallback() {
        let slots = on_demand_slots(6, "On Demand", &[], ON_DEMAND_ITEM_WIDTH_PCT);
        assert_eq!(slots.len(), 1);
        assert_eq!(slots[0].item_id, None);
        assert!(slots[0].title.contains("On Demand"));
        assert!((slots[0].left_pct - 0.0).abs() < 0.01);
        assert!((slots[0].width_pct - 100.0).abs() < 0.01);
        assert!(!slots[0].is_live);
    }

    #[test]
    fn test_on_demand_slots_three_items_evenly_placed() {
        let items = vec![pl_item(1, "A"), pl_item(2, "B"), pl_item(3, "C")];
        let slots = on_demand_slots(6, "On Demand", &items, 25.0);
        assert_eq!(slots.len(), 3);
        assert!((slots[0].left_pct - 0.0).abs() < 0.01);
        assert!((slots[1].left_pct - 25.0).abs() < 0.01);
        assert!((slots[2].left_pct - 50.0).abs() < 0.01);
        for s in &slots {
            assert!((s.width_pct - 25.0).abs() < 0.01);
            assert!(!s.is_live);
        }
        assert_eq!(slots[0].item_id, Some(1));
        assert_eq!(slots[1].item_id, Some(2));
        assert_eq!(slots[2].item_id, Some(3));
        assert_eq!(slots[0].title, "A");
    }

    #[test]
    fn test_on_demand_slots_clips_at_window_edge() {
        // 6 items at 25% -> the 5th would start at left=100 -> stop. 4 visible.
        let items: Vec<PlaylistItem> = (1..=6).map(|i| pl_item(i, "x")).collect();
        let slots = on_demand_slots(6, "On Demand", &items, 25.0);
        assert_eq!(slots.len(), 4);
        for s in &slots {
            assert!(s.left_pct < 100.0);
            assert!(s.left_pct + s.width_pct <= 100.0 + 0.01);
        }
        assert!((slots[3].left_pct - 75.0).abs() < 0.01);
        assert!((slots[3].width_pct - 25.0).abs() < 0.01);
    }

    #[test]
    fn test_on_demand_slots_clamps_last_partial_block() {
        // 4 items at 30% -> left 0/30/60/90; last clamped to width 10.
        let items: Vec<PlaylistItem> = (1..=4).map(|i| pl_item(i, "x")).collect();
        let slots = on_demand_slots(6, "On Demand", &items, 30.0);
        assert_eq!(slots.len(), 4);
        assert!((slots[3].left_pct - 90.0).abs() < 0.01);
        assert!((slots[3].width_pct - 10.0).abs() < 0.01);
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
}
