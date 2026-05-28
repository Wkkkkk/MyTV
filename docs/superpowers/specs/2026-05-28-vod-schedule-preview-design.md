# VOD Schedule Preview — Design Spec
_2026-05-28_

## Goal

Show the current and upcoming items for a VOD loop channel in the admin channel detail page, so operators can verify the loop anchor is set correctly and see what's "on air" at a glance.

## Architecture

No new routes. The existing `GET /admin/channels/:id` handler is extended to compute and pass a schedule when the channel is a `vod_loop`. All schedule math reuses the existing `epg::vod_schedule` function.

**Files changed:**
- `src/routes/admin.rs` — add `AdminScheduleRow` display type; update `ChannelDetailTemplate` and `channel_detail` handler
- `templates/admin/channel_detail.html` — add "Now & Upcoming" section for VOD channels

---

## Data Types

```rust
pub struct AdminScheduleRow {
    pub is_current: bool,      // true for the first (currently playing) row
    pub title: String,
    pub start_time: String,    // formatted as "HH:MM UTC"
    pub duration_secs: i64,
}
```

`ChannelDetailTemplate` gains one new field:

```rust
vod_schedule: Vec<AdminScheduleRow>,  // empty for live channels
```

---

## Handler Logic

In `channel_detail`, after fetching `ch` and `items`, compute the schedule for VOD channels:

```
if vod_loop && loop_anchor.is_some() && items non-empty:
    total_dur = sum of item.duration_secs (skip if 0)
    if total_dur > 0:
        window = now … now + 2 × total_dur seconds
        entries = epg::vod_schedule(ch.id, &items, anchor.timestamp(), window_start, window_end)
        take first 8 entries
        map each to AdminScheduleRow:
            is_current = index == 0
            title      = entry.title
            start_time = entry.start_time.format("%H:%M UTC")
            duration_secs = entry.end_time - entry.start_time (in seconds)
else:
    vod_schedule = vec![]
```

The `duration_secs` of the last displayed entry may be clipped if the window ends mid-item; this is acceptable since it's the 8th row and rarely visible.

---

## Template

A "Now & Upcoming" `<div class="section">` is rendered **above** the Playlist section, only when `!vod_schedule.is_empty()`.

| Column | Content |
|---|---|
| (badge) | `NOW` badge (`badge-live`) for `is_current`, otherwise the row number (1, 2, …) |
| Title | `row.title` |
| Starts | `row.start_time` |
| Duration | `row.duration_secs`s |

First row is visually highlighted (the `NOW` badge suffices; no additional row styling needed).

---

## Error Handling

| Condition | Behaviour |
|---|---|
| `loop_anchor` is `None` | `vod_schedule` is empty; section not rendered |
| Empty playlist | `vod_schedule` is empty; section not rendered |
| Any item has `duration_secs == 0` | `epg::vod_schedule` returns `[]`; section not rendered |
| `total_dur == 0` | Skip computation; section not rendered |

---

## Testing

`epg::vod_schedule` is already covered by 6 unit tests. The new handler mapping is thin (format + take 8); no additional unit tests needed. Verified by building and loading a VOD channel detail page.
