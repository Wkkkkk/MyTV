# VOD Schedule Preview Implementation Plan

> **For agentic workers:** REQUIRED SUB-SKILL: Use superpowers:subagent-driven-development (recommended) or superpowers:executing-plans to implement this plan task-by-task. Steps use checkbox (`- [ ]`) syntax for tracking.

**Goal:** Show "Now & Upcoming" items on the admin channel detail page for VOD loop channels, so operators can verify the loop anchor is set correctly at a glance.

**Architecture:** Extend the existing `channel_detail` handler in `src/routes/admin.rs` to compute a schedule (up to 8 entries) using the already-tested `epg::vod_schedule` function, pass it through a new `AdminScheduleRow` display type, and render it in `templates/admin/channel_detail.html` above the Playlist section.

**Tech Stack:** Rust, Axum 0.7, Askama 0.12, chrono, sqlx 0.7 (SQLite)

---

## File Map

| File | Change |
|---|---|
| `src/routes/admin.rs` | Add `AdminScheduleRow` struct; add `vod_schedule` field to `ChannelDetailTemplate`; compute schedule in `channel_detail` handler |
| `templates/admin/channel_detail.html` | Add "Now & Upcoming" section above Playlist section |

---

### Task 1: Add `AdminScheduleRow`, update template struct, compute schedule in handler

**Files:**
- Modify: `src/routes/admin.rs`

---

- [ ] **Step 1: Add `epg` to the imports**

In `src/routes/admin.rs`, line 12, change:

```rust
use crate::{channel, playlist_item, source, AppState};
```

to:

```rust
use crate::{channel, epg, playlist_item, source, AppState};
```

---

- [ ] **Step 2: Add `AdminScheduleRow` struct**

In `src/routes/admin.rs`, after the closing `}` of `AdminPlaylistItemRow` (around line 92), add:

```rust
struct AdminScheduleRow {
    is_current: bool,
    title: String,
    start_time: String, // "HH:MM UTC"
    duration_secs: i64,
}
```

No `pub` — this type is only used inside this module.

---

- [ ] **Step 3: Add `vod_schedule` field to `ChannelDetailTemplate`**

`ChannelDetailTemplate` currently looks like (lines 104–112):

```rust
#[derive(Template)]
#[template(path = "admin/channel_detail.html")]
struct ChannelDetailTemplate {
    channel_id: i64,
    channel_name: String,
    channel_type: String,
    sources: Vec<AdminSourceRow>,
    playlist_items: Vec<AdminPlaylistItemRow>,
}
```

Add the new field:

```rust
#[derive(Template)]
#[template(path = "admin/channel_detail.html")]
struct ChannelDetailTemplate {
    channel_id: i64,
    channel_name: String,
    channel_type: String,
    sources: Vec<AdminSourceRow>,
    playlist_items: Vec<AdminPlaylistItemRow>,
    vod_schedule: Vec<AdminScheduleRow>,
}
```

---

- [ ] **Step 4: Compute the schedule in `channel_detail`**

The `channel_detail` handler currently ends with (lines 341–366):

```rust
    render(ChannelDetailTemplate {
        channel_id: ch.id,
        channel_name: ch.name,
        channel_type: ch.r#type,
        sources: srcs
            .into_iter()
            .map(|s| AdminSourceRow {
                id: s.id,
                kind: s.kind,
                url: s.url,
                priority: s.priority,
                is_active: s.is_active,
            })
            .collect(),
        playlist_items: items
            .into_iter()
            .map(|i| AdminPlaylistItemRow {
                id: i.id,
                title: i.title,
                url: i.url,
                duration_secs: i.duration_secs,
                sort_order: i.sort_order,
            })
            .collect(),
    })
```

Replace it with:

```rust
    let vod_schedule: Vec<AdminScheduleRow> = if ch.r#type == "vod_loop" {
        if let Some(anchor) = ch.loop_anchor {
            let total_dur: i64 = items.iter().map(|i| i.duration_secs).sum();
            if total_dur > 0 {
                let now = Utc::now();
                let window_end = now + chrono::Duration::seconds(2 * total_dur);
                epg::vod_schedule(ch.id, &items, anchor.timestamp(), now, window_end)
                    .into_iter()
                    .take(8)
                    .enumerate()
                    .map(|(i, e)| AdminScheduleRow {
                        is_current: i == 0,
                        title: e.title,
                        start_time: e.start_time.format("%H:%M UTC").to_string(),
                        duration_secs: (e.end_time - e.start_time).num_seconds(),
                    })
                    .collect()
            } else {
                vec![]
            }
        } else {
            vec![]
        }
    } else {
        vec![]
    };

    render(ChannelDetailTemplate {
        channel_id: ch.id,
        channel_name: ch.name,
        channel_type: ch.r#type,
        sources: srcs
            .into_iter()
            .map(|s| AdminSourceRow {
                id: s.id,
                kind: s.kind,
                url: s.url,
                priority: s.priority,
                is_active: s.is_active,
            })
            .collect(),
        playlist_items: items
            .into_iter()
            .map(|i| AdminPlaylistItemRow {
                id: i.id,
                title: i.title,
                url: i.url,
                duration_secs: i.duration_secs,
                sort_order: i.sort_order,
            })
            .collect(),
        vod_schedule,
    })
```

Note: `epg::vod_schedule` returns `[]` when any item has `duration_secs == 0`, so the `total_dur > 0` guard above (which sums without filtering) is a quick-exit for the common empty-playlist case. The `vod_schedule` function itself enforces the zero-duration rule.

---

- [ ] **Step 5: Run tests**

```bash
cargo test
```

Expected: all existing tests pass (82+). If there are compile errors, check:
- `epg` is imported on line 12
- `AdminScheduleRow` is defined before `ChannelDetailTemplate`
- `vod_schedule` field is in `ChannelDetailTemplate`
- `vod_schedule` variable is computed before `render(...)`

---

- [ ] **Step 6: Commit**

```bash
git add src/routes/admin.rs
git commit -m "feat: add AdminScheduleRow and vod_schedule to channel detail handler"
```

---

### Task 2: Add "Now & Upcoming" section to the channel detail template

**Files:**
- Modify: `templates/admin/channel_detail.html`

---

- [ ] **Step 1: Insert the "Now & Upcoming" section**

In `templates/admin/channel_detail.html`, the Playlist section starts at line 83:

```html
<!-- Playlist items (vod_loop only) -->
{% if channel_type.as_str() == "vod_loop" %}
```

Insert the following block **immediately before** that comment (i.e., between line 81 `</div>` closing Sources and line 83 `<!-- Playlist items`):

```html
<!-- Now & Upcoming (vod_loop only, rendered when schedule is computable) -->
{% if !vod_schedule.is_empty() %}
<div class="section">
  <h3>Now &amp; Upcoming</h3>
  <table>
    <thead>
      <tr><th></th><th>Title</th><th>Starts</th><th>Duration</th></tr>
    </thead>
    <tbody>
      {% for row in vod_schedule %}
      <tr>
        <td style="white-space:nowrap">
          {% if row.is_current %}
          <span class="badge badge-live">NOW</span>
          {% else %}
          <span style="color:#555">{{ loop.index0 }}</span>
          {% endif %}
        </td>
        <td>{{ row.title }}</td>
        <td style="white-space:nowrap;color:#777;font-size:0.8rem">{{ row.start_time }}</td>
        <td style="white-space:nowrap;color:#777;font-size:0.8rem">{{ row.duration_secs }}s</td>
      </tr>
      {% endfor %}
    </tbody>
  </table>
</div>
{% endif %}

```

`loop.index0` is Askama's 0-based loop counter. For the first row (`is_current = true`) the `NOW` badge renders instead, so the `0` is never shown. Upcoming rows display `1`, `2`, `3`, … matching the spec.

---

- [ ] **Step 2: Build to verify the template compiles**

```bash
cargo build
```

Expected: compiles without errors. Askama templates are compiled at build time — any template syntax error will surface here.

---

- [ ] **Step 3: Manual smoke test**

1. Run the server: `cargo run`
2. Open a VOD loop channel with a loop anchor set and at least one playlist item with `duration_secs > 0`.
3. Navigate to `http://localhost:3000/admin/channels/<id>`.
4. Confirm: "Now & Upcoming" section appears above "Playlist", first row shows `NOW` badge, remaining rows show `1`, `2`, … with correct times.
5. Open a live channel detail page. Confirm: "Now & Upcoming" section is absent.
6. Open a VOD loop channel with no loop anchor set. Confirm: "Now & Upcoming" section is absent.

---

- [ ] **Step 4: Commit**

```bash
git add templates/admin/channel_detail.html
git commit -m "feat: show Now & Upcoming schedule on VOD loop channel detail page"
```
