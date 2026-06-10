# YouTube Discover Improvements Implementation Plan

> **For agentic workers:** REQUIRED SUB-SKILL: Use superpowers:subagent-driven-development (recommended) or superpowers:executing-plans to implement this plan task-by-task. Steps use checkbox (`- [ ]`) syntax for tracking.

**Goal:** Add thumbnails to YouTube Discover search results and handle scheduled ("upcoming") streams with an UPCOMING badge + scheduled start time, addable as `youtube_live` sources.

**Architecture:** All Rust changes live in `src/routes/admin/discover/youtube.rs` (row model, JSON extraction, one extra `part` on the existing `videos.list` call). Display changes live in `templates/admin/partials/discover_yt_results.html` plus one CSS rule in `templates/admin/base.html`. No DB, route, or health-checker changes.

**Tech Stack:** Rust 1.96, Axum 0.7, Askama 0.12 (templates are compile-checked), chrono, serde_json. Spec: `docs/superpowers/specs/2026-06-10-youtube-discover-improvements-design.md`.

**Conventions:** Run `cargo fmt` before EVERY commit (CI fails on any formatting diff). `cargo clippy -- -D warnings` must stay clean.

---

### Task 1: Scheduled-start timestamp formatter

**Files:**
- Modify: `src/routes/admin/discover/youtube.rs` (add function near `parse_iso8601_duration`, test in existing `mod tests`)

- [ ] **Step 1: Write the failing test**

Add to the `mod tests` block at the bottom of `src/routes/admin/discover/youtube.rs`:

```rust
    #[test]
    fn format_scheduled_start_cases() {
        assert_eq!(
            format_scheduled_start("2026-06-12T18:00:00Z"),
            "Jun 12 18:00 UTC"
        );
        // offset timestamps are normalized to UTC
        assert_eq!(
            format_scheduled_start("2026-06-12T20:30:00+02:00"),
            "Jun 12 18:30 UTC"
        );
        assert_eq!(format_scheduled_start("not-a-date"), "");
        assert_eq!(format_scheduled_start(""), "");
    }
```

- [ ] **Step 2: Run test to verify it fails**

Run: `cargo test --lib format_scheduled_start_cases`
Expected: COMPILE ERROR — `cannot find function format_scheduled_start in this scope`

- [ ] **Step 3: Write minimal implementation**

Add above `parse_iso8601_duration` in the same file:

```rust
pub(super) fn format_scheduled_start(rfc3339: &str) -> String {
    chrono::DateTime::parse_from_rfc3339(rfc3339)
        .map(|dt| {
            dt.with_timezone(&chrono::Utc)
                .format("%b %d %H:%M UTC")
                .to_string()
        })
        .unwrap_or_default()
}
```

(chrono is already a dependency — `src/model/channel.rs` uses it.)

- [ ] **Step 4: Run test to verify it passes**

Run: `cargo test --lib format_scheduled_start_cases`
Expected: `test ... ok`

- [ ] **Step 5: Format and commit**

```bash
cargo fmt
git add src/routes/admin/discover/youtube.rs
git commit -m "feat: add scheduled-start timestamp formatter for YouTube discover"
```

---

### Task 2: Row model + extraction (thumbnails, upcoming, scheduled start)

**Files:**
- Modify: `src/routes/admin/discover/youtube.rs` — `YoutubeResultRow`, `build_video_rows`, `build_channel_rows`, `fetch_youtube_results`, existing tests

This task changes the `build_video_rows` signature, so its caller `fetch_youtube_results` (same file) is updated in the same task to keep the crate compiling.

- [ ] **Step 1: Write the failing tests**

In `mod tests` of `src/routes/admin/discover/youtube.rs`, add two new tests:

```rust
    #[test]
    fn video_rows_mark_upcoming_as_live_source_with_schedule() {
        let items = vec![serde_json::json!({
            "id": {"videoId": "up1"},
            "snippet": {"title": "Launch", "channelTitle": "SpaceX",
                        "liveBroadcastContent": "upcoming",
                        "thumbnails": {"default": {"url": "https://i.ytimg.com/vi/up1/default.jpg"}}}
        })];
        let dur = std::collections::HashMap::new();
        let mut sched = std::collections::HashMap::new();
        sched.insert("up1".to_string(), "2026-06-12T18:00:00Z".to_string());
        let rows = build_video_rows(&items, &dur, &sched);
        assert_eq!(rows.len(), 1);
        assert!(rows[0].is_upcoming);
        assert!(!rows[0].is_live);
        assert_eq!(rows[0].source_kind, "youtube_live");
        assert_eq!(rows[0].scheduled_start, "Jun 12 18:00 UTC");
        assert_eq!(
            rows[0].thumbnail_url,
            "https://i.ytimg.com/vi/up1/default.jpg"
        );
    }

    #[test]
    fn rows_without_thumbnails_get_empty_thumbnail_url() {
        let items = vec![serde_json::json!({
            "id": {"videoId": "abc"},
            "snippet": {"title": "A VOD", "channelTitle": "Chan",
                        "liveBroadcastContent": "none"}
        })];
        let dur = std::collections::HashMap::new();
        let sched = std::collections::HashMap::new();
        let rows = build_video_rows(&items, &dur, &sched);
        assert_eq!(rows[0].thumbnail_url, "");
        assert_eq!(rows[0].scheduled_start, "");
        assert!(!rows[0].is_upcoming);
    }
```

Update the two existing tests to the new shape:

In `channel_rows_build_live_urls`, change the items JSON to include a thumbnail and add two assertions at the end:

```rust
    #[test]
    fn channel_rows_build_live_urls() {
        let items = vec![serde_json::json!({
            "id": {"channelId": "UC123"},
            "snippet": {"title": "NASA", "channelTitle": "NASA",
                        "thumbnails": {"default": {"url": "https://yt3.ggpht.com/nasa.jpg"}}}
        })];
        let rows = build_channel_rows(&items);
        assert_eq!(rows.len(), 1);
        assert_eq!(rows[0].url, "https://www.youtube.com/channel/UC123/live");
        assert!(rows[0].is_live);
        assert_eq!(rows[0].duration_secs, 0);
        assert_eq!(rows[0].source_kind, "youtube_live");
        assert_eq!(rows[0].title, "NASA");
        assert_eq!(rows[0].channel_title, "");
        assert!(!rows[0].is_upcoming);
        assert_eq!(rows[0].thumbnail_url, "https://yt3.ggpht.com/nasa.jpg");
    }
```

In `video_rows_label_vod_when_not_live`, change the `build_video_rows` call to pass an empty scheduled map:

```rust
        let rows = build_video_rows(&items, &dur, &std::collections::HashMap::new());
```

- [ ] **Step 2: Run tests to verify they fail**

Run: `cargo test --lib discover`
Expected: COMPILE ERROR — `build_video_rows` takes 2 arguments but 3 were supplied / no field `is_upcoming` on `YoutubeResultRow`

- [ ] **Step 3: Implement**

Replace `YoutubeResultRow` in `src/routes/admin/discover/youtube.rs`:

```rust
pub struct YoutubeResultRow {
    pub title: String,
    pub channel_title: String,
    pub is_live: bool,
    pub is_upcoming: bool,
    pub duration_secs: i64,
    pub scheduled_start: String,
    pub thumbnail_url: String,
    pub url: String,
    pub source_kind: String,
    pub form_id: usize,
}
```

Replace `build_video_rows`:

```rust
pub(super) fn build_video_rows(
    items: &[serde_json::Value],
    duration_map: &std::collections::HashMap<String, i64>,
    scheduled_map: &std::collections::HashMap<String, String>,
) -> Vec<YoutubeResultRow> {
    items
        .iter()
        .enumerate()
        .filter_map(|(i, item)| {
            let video_id = item["id"]["videoId"].as_str()?;
            let snippet = &item["snippet"];
            let title = snippet["title"].as_str().unwrap_or("Unknown").to_string();
            let channel_title = snippet["channelTitle"].as_str().unwrap_or("").to_string();
            let broadcast = snippet["liveBroadcastContent"].as_str().unwrap_or("none");
            let is_live = broadcast == "live";
            let is_upcoming = broadcast == "upcoming";
            let duration_secs = *duration_map.get(video_id).unwrap_or(&0);
            let scheduled_start = if is_upcoming {
                scheduled_map
                    .get(video_id)
                    .map(|ts| format_scheduled_start(ts))
                    .unwrap_or_default()
            } else {
                String::new()
            };
            let thumbnail_url = snippet["thumbnails"]["default"]["url"]
                .as_str()
                .unwrap_or("")
                .to_string();
            let source_kind = if is_live || is_upcoming {
                "youtube_live"
            } else {
                "youtube_vod"
            }
            .to_string();
            let url = format!("https://www.youtube.com/watch?v={}", video_id);
            Some(YoutubeResultRow {
                title,
                channel_title,
                is_live,
                is_upcoming,
                duration_secs,
                scheduled_start,
                thumbnail_url,
                url,
                source_kind,
                form_id: i,
            })
        })
        .collect()
}
```

Replace `build_channel_rows`:

```rust
pub(super) fn build_channel_rows(items: &[serde_json::Value]) -> Vec<YoutubeResultRow> {
    items
        .iter()
        .enumerate()
        .filter_map(|(i, item)| {
            let channel_id = item["id"]["channelId"].as_str()?;
            let snippet = &item["snippet"];
            let title = snippet["title"].as_str().unwrap_or("Unknown").to_string();
            let thumbnail_url = snippet["thumbnails"]["default"]["url"]
                .as_str()
                .unwrap_or("")
                .to_string();
            let url = format!("https://www.youtube.com/channel/{}/live", channel_id);
            Some(YoutubeResultRow {
                title,
                channel_title: String::new(),
                is_live: true,
                is_upcoming: false,
                duration_secs: 0,
                scheduled_start: String::new(),
                thumbnail_url,
                url,
                source_kind: "youtube_live".to_string(),
                form_id: i,
            })
        })
        .collect()
}
```

In `fetch_youtube_results`, change the `videos.list` request `part` parameter:

```rust
        .query(&[
            ("part", "contentDetails,liveStreamingDetails"),
            ("id", ids_joined.as_str()),
            ("key", api_key),
        ])
```

Replace the duration-map block and the `build_video_rows` call at the end of `fetch_youtube_results`:

```rust
    let mut duration_map = std::collections::HashMap::<String, i64>::new();
    let mut scheduled_map = std::collections::HashMap::<String, String>::new();
    if let Some(detail_items) = details_resp["items"].as_array() {
        for item in detail_items {
            let id = item["id"].as_str().unwrap_or("").to_string();
            let dur_str = item["contentDetails"]["duration"]
                .as_str()
                .unwrap_or("PT0S");
            duration_map.insert(id.clone(), parse_iso8601_duration(dur_str));
            if let Some(ts) = item["liveStreamingDetails"]["scheduledStartTime"].as_str() {
                scheduled_map.insert(id, ts.to_string());
            }
        }
    }

    let rows = build_video_rows(items, &duration_map, &scheduled_map);

    Ok(rows)
```

- [ ] **Step 4: Run tests to verify they pass**

Run: `cargo test --lib discover`
Expected: PASS. (The template still compiles unchanged — Askama only checks fields the template references, and the template doesn't reference the new fields until Task 3.) All discover tests must pass:

```
test routes::admin::discover::youtube::tests::format_scheduled_start_cases ... ok
test routes::admin::discover::youtube::tests::channel_rows_build_live_urls ... ok
test routes::admin::discover::youtube::tests::video_rows_label_vod_when_not_live ... ok
test routes::admin::discover::youtube::tests::video_rows_mark_upcoming_as_live_source_with_schedule ... ok
test routes::admin::discover::youtube::tests::rows_without_thumbnails_get_empty_thumbnail_url ... ok
```

If anything else fails, stop and fix before committing.

- [ ] **Step 5: Format and commit**

```bash
cargo fmt
git add src/routes/admin/discover/youtube.rs
git commit -m "feat: extract thumbnails, upcoming state and scheduled start in YouTube discover"
```

---

### Task 3: Template — thumbnail column, UPCOMING badge, badge CSS

**Files:**
- Modify: `templates/admin/partials/discover_yt_results.html` (full replacement below)
- Modify: `templates/admin/base.html:52` (one CSS line added after `.badge-vod`)

- [ ] **Step 1: Replace the results partial**

Replace the entire content of `templates/admin/partials/discover_yt_results.html` with:

```html
{% if rows.is_empty() %}
<p class="empty-state">No results found.</p>
{% else %}
<table>
  <thead>
    <tr><th></th><th>Title</th><th>Channel</th><th>Type</th><th>Duration</th><th></th></tr>
  </thead>
  <tbody>
    {% for row in rows %}
    <tr>
      <td style="width:88px">
        {% if !row.thumbnail_url.is_empty() %}
        <img src="{{ row.thumbnail_url }}" width="80" loading="lazy" alt=""
             style="display:block;border-radius:3px">
        {% endif %}
      </td>
      <td>{{ row.title }}</td>
      <td style="color:#777;font-size:0.8rem">{{ row.channel_title }}</td>
      <td>
        {% if row.is_upcoming %}
        <span class="badge badge-upcoming">UPCOMING</span>
        {% if !row.scheduled_start.is_empty() %}
        <span style="color:#777;font-size:0.75rem;margin-left:6px">{{ row.scheduled_start }}</span>
        {% endif %}
        {% else if row.is_live %}
        <span class="badge badge-live">LIVE</span>
        {% if row.source_kind == "youtube_live" %}
        <span hx-get="/admin/live-status?url={{ row.url|urlencode }}"
              hx-trigger="load" hx-swap="outerHTML" style="color:#666;margin-left:6px">checking…</span>
        {% endif %}
        {% else %}
        <span class="badge badge-vod">VOD</span>
        {% endif %}
      </td>
      <td style="white-space:nowrap;color:#777;font-size:0.8rem">
        {% if row.duration_secs > 0 %}{{ row.duration_secs }}s{% else %}—{% endif %}
      </td>
      <td style="white-space:nowrap">
        <form hx-post="/admin/discover/add-form"
              hx-target="#yt-add-form-{{ row.form_id }}"
              hx-swap="innerHTML"
              style="display:inline">
          <input type="hidden" name="url" value="{{ row.url }}">
          <input type="hidden" name="title" value="{{ row.title }}">
          <input type="hidden" name="is_live" value="{% if row.is_live || row.is_upcoming %}true{% else %}false{% endif %}">
          <input type="hidden" name="duration_secs" value="{{ row.duration_secs }}">
          <input type="hidden" name="source_kind" value="{{ row.source_kind }}">
          <input type="hidden" name="form_id" value="yt{{ row.form_id }}">
          <button type="submit" class="btn btn-primary btn-sm">Add</button>
        </form>
      </td>
    </tr>
    <tr>
      <td colspan="6" style="padding:0">
        <div id="yt-add-form-{{ row.form_id }}" style="padding:10px 14px;background:#080810"></div>
      </td>
    </tr>
    {% endfor %}
  </tbody>
</table>
{% endif %}
```

Notes for the implementer:
- The live-status `checking…` span exists ONLY in the `is_live` branch — upcoming rows must NOT trigger the yt-dlp probe.
- `colspan` changed from 5 to 6 (new thumbnail column).
- Askama uses `{% else if %}` (not Jinja's `{% elif %}`); `!expr` and `||` are valid Askama boolean syntax (see `templates/admin/channel_detail.html:65` for an existing `!…is_empty()` use).

- [ ] **Step 2: Add the badge CSS**

In `templates/admin/base.html`, directly after the `.badge-vod` rule (line 52), add:

```css
    .badge-upcoming{background:#2a1f0a;color:#db4;border:1px solid #4a3a1a}
```

- [ ] **Step 3: Compile-check the template and run the full suite**

Run: `cargo test`
Expected: all tests pass (Askama compile-checks the template against the new `YoutubeResultRow` fields; a typo in a field name fails the build here).

- [ ] **Step 4: Format and commit**

```bash
cargo fmt
git add templates/admin/partials/discover_yt_results.html templates/admin/base.html
git commit -m "feat: thumbnail column and UPCOMING badge in YouTube discover results"
```

---

### Task 4: Docs + final verification

**Files:**
- Modify: `docs/IDEAS.md:47` (item #35)

- [ ] **Step 1: Check for unrelated pending edits**

Run: `git diff docs/IDEAS.md`
`docs/IDEAS.md` had uncommitted modifications before this work started. If the diff contains changes unrelated to item #35, do NOT commit them silently — surface them to the user and only stage the #35 edit (use `git add -p docs/IDEAS.md` if needed).

- [ ] **Step 2: Mark idea #35 done**

Replace the item #35 paragraph in `docs/IDEAS.md` with:

```markdown
35. ~~**YouTube Discover improvements**~~ — done: gaps (1) `source_kind` VOD detection and (2) `type=channel` search + channel-URL resolve were already fixed earlier; this change adds (3) thumbnails (`snippet.thumbnails.default`, lazy-loaded 80 px column for video and channel results) and (4) upcoming-stream handling (`liveBroadcastContent="upcoming"` → amber UPCOMING badge + scheduled start time from `liveStreamingDetails.scheduledStartTime`, addable as a `youtube_live` source that activates when the stream goes live; the yt-dlp live-status probe is skipped for upcoming rows). Spec: `docs/superpowers/specs/2026-06-10-youtube-discover-improvements-design.md`.
```

- [ ] **Step 3: Full verification**

```bash
cargo fmt --check
cargo clippy -- -D warnings
cargo test
```

Expected: no formatting diff, no clippy warnings, all tests pass (318+ tests; 5 ignored needing yt-dlp/network is normal).

- [ ] **Step 4: Commit**

```bash
git add docs/IDEAS.md
git commit -m "docs: mark idea #35 (YouTube Discover improvements) done"
```
