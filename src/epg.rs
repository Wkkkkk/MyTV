use chrono::{DateTime, Duration, Utc};
use serde::Serialize;

use crate::playlist_item::{self, PlaylistItem};

#[derive(Debug, Clone, Serialize)]
pub struct ProgramEntry {
    pub channel_id: i64,
    pub title: String,
    pub url: String,
    pub start_time: DateTime<Utc>,
    pub end_time: DateTime<Utc>,
    pub is_live: bool,
    pub start_offset_secs: i64,
}

/// Returns the single EPG entry for a live channel spanning the given window.
pub fn live_entry(
    channel_id: i64,
    name: &str,
    window_start: DateTime<Utc>,
    window_end: DateTime<Utc>,
) -> ProgramEntry {
    ProgramEntry {
        channel_id,
        title: format!("{} — Live", name),
        url: String::new(),
        start_time: window_start,
        end_time: window_end,
        is_live: true,
        start_offset_secs: 0,
    }
}

/// Computes program entries for a VOD loop channel over the given time window.
/// Entries are in chronological order. The first entry may start with a non-zero
/// offset if the window starts mid-item. The last entry is clipped to window_end.
pub fn vod_schedule(
    channel_id: i64,
    items: &[PlaylistItem],
    anchor_secs: i64,
    window_start: DateTime<Utc>,
    window_end: DateTime<Utc>,
) -> Vec<ProgramEntry> {
    if items.is_empty() || window_start >= window_end {
        return vec![];
    }

    if items.iter().any(|i| i.duration_secs <= 0) {
        return vec![];
    }

    let (mut idx, mut offset) =
        match playlist_item::current_position(items, window_start.timestamp(), anchor_secs) {
            Some(pos) => pos,
            None => return vec![],
        };

    let mut entries = Vec::new();
    let mut cursor = window_start;

    while cursor < window_end {
        let item = &items[idx];
        let remaining_secs = item.duration_secs - offset;
        let entry_end = (cursor + Duration::seconds(remaining_secs)).min(window_end);

        entries.push(ProgramEntry {
            channel_id,
            title: item.title.clone(),
            url: item.url.clone(),
            start_time: cursor,
            end_time: entry_end,
            is_live: false,
            start_offset_secs: offset,
        });

        cursor = entry_end;
        offset = 0;
        idx = (idx + 1) % items.len();
    }

    entries
}

#[cfg(test)]
mod tests {
    use super::*;
    use chrono::TimeZone;

    fn item(id: i64, title: &str, duration_secs: i64) -> PlaylistItem {
        PlaylistItem {
            id,
            channel_id: 1,
            title: title.to_string(),
            url: format!("https://example.com/{}.mp4", title),
            duration_secs,
            sort_order: id - 1,
        }
    }

    #[test]
    fn test_live_entry_spans_full_window() {
        let start = Utc.with_ymd_and_hms(2026, 1, 1, 0, 0, 0).unwrap();
        let end = start + Duration::hours(24);
        let entry = live_entry(1, "CNN", start, end);
        assert!(entry.is_live);
        assert_eq!(entry.start_time, start);
        assert_eq!(entry.end_time, end);
        assert_eq!(entry.channel_id, 1);
        assert!(entry.title.contains("CNN"));
    }

    #[test]
    fn test_vod_schedule_empty_playlist_returns_empty() {
        let start = Utc.with_ymd_and_hms(2026, 1, 1, 0, 0, 0).unwrap();
        let end = start + Duration::hours(1);
        let entries = vod_schedule(1, &[], 0, start, end);
        assert!(entries.is_empty());
    }

    #[test]
    fn test_vod_schedule_window_start_equals_end_returns_empty() {
        let t = Utc.with_ymd_and_hms(2026, 1, 1, 0, 0, 0).unwrap();
        let entries = vod_schedule(1, &[item(1, "A", 3600)], 0, t, t);
        assert!(entries.is_empty());
    }

    #[test]
    fn test_vod_schedule_two_full_loops() {
        // Playlist: A (3600s) + B (1800s) = 5400s total. Window = 3h = two full loops.
        let items = vec![item(1, "A", 3600), item(2, "B", 1800)];
        let anchor_secs = 0;
        let start = Utc.with_ymd_and_hms(2026, 1, 1, 0, 0, 0).unwrap(); // exactly at anchor
        let end = start + Duration::hours(3);

        let entries = vod_schedule(1, &items, anchor_secs, start, end);

        assert_eq!(entries.len(), 4); // A, B, A, B
        assert_eq!(entries[0].title, "A");
        assert_eq!(entries[0].start_offset_secs, 0);
        assert_eq!(entries[1].title, "B");
        assert_eq!(entries[1].start_offset_secs, 0);
        assert_eq!(entries[2].title, "A");
        assert_eq!(entries[2].start_offset_secs, 0);
        assert_eq!(entries[3].title, "B");
        assert_eq!(entries[3].start_offset_secs, 0);
    }

    #[test]
    fn test_vod_schedule_starts_mid_first_item() {
        // Playlist: A (3600s). Window starts 1800s into A. Window = 3600s.
        // Expected: first entry is tail of A (1800s remaining, offset 1800),
        //           second entry is head of next-loop A (1800s, offset 0).
        let items = vec![item(1, "A", 3600)];
        let anchor_secs = 0;
        let start = Utc.timestamp_opt(1800, 0).unwrap();
        let end = start + Duration::seconds(3600);

        let entries = vod_schedule(1, &items, anchor_secs, start, end);

        assert_eq!(entries.len(), 2);
        assert_eq!(entries[0].start_offset_secs, 1800);
        assert_eq!(entries[0].end_time, start + Duration::seconds(1800));
        assert_eq!(entries[1].start_offset_secs, 0);
        assert_eq!(entries[1].end_time, end);
    }

    #[test]
    fn test_vod_schedule_last_entry_clipped_to_window_end() {
        // Playlist: A (3600s). Window = 30 min. Should produce one entry ending at window_end.
        let items = vec![item(1, "A", 3600)];
        let anchor_secs = 0;
        let start = Utc.timestamp_opt(0, 0).unwrap();
        let end = start + Duration::minutes(30);

        let entries = vod_schedule(1, &items, anchor_secs, start, end);

        assert_eq!(entries.len(), 1);
        assert_eq!(entries[0].title, "A");
        assert_eq!(entries[0].start_offset_secs, 0);
        assert_eq!(entries[0].end_time, end); // clipped, not 3600s
    }

    #[test]
    fn test_vod_schedule_zero_duration_item_returns_empty() {
        let items = vec![item(1, "A", 0)];
        let start = Utc.with_ymd_and_hms(2026, 1, 1, 0, 0, 0).unwrap();
        let end = start + Duration::hours(1);
        let entries = vod_schedule(1, &items, 0, start, end);
        assert!(entries.is_empty());
    }

    #[test]
    fn test_vod_schedule_mid_second_item() {
        // Playlist: A (3600s) + B (1800s). Window starts 4000s into anchor (400s into B).
        let items = vec![item(1, "A", 3600), item(2, "B", 1800)];
        let anchor_secs = 0;
        let start = Utc.timestamp_opt(4000, 0).unwrap(); // 4000s = 3600 (A) + 400s into B
        let end = start + Duration::minutes(30);

        let entries = vod_schedule(1, &items, anchor_secs, start, end);

        assert_eq!(entries[0].title, "B");
        assert_eq!(entries[0].start_offset_secs, 400); // 400s into B
    }
}
