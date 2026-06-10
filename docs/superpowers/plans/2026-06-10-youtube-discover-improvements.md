# YouTube Discover Improvements + Live-Status State Coverage Implementation Plan

> **For agentic workers:** REQUIRED SUB-SKILL: Use superpowers:subagent-driven-development (recommended) or superpowers:executing-plans to implement this plan task-by-task. Steps use checkbox (`- [ ]`) syntax for tracking.

**Goal:** (Part 1) Add thumbnails and upcoming-stream handling to YouTube Discover search results; (Part 2) extend the yt-dlp live-status probe from 3 states (Live/Offline/Unknown) to the full `live_status` model (Live/Upcoming/PostLive/WasLive/NotLive/Offline/Unknown) with distinct admin badges.

**Architecture:** Part 1 lives in `src/routes/admin/discover/youtube.rs` (row model, JSON extraction, one extra `part` on the existing `videos.list` call) plus the results template. Part 2 lives in `src/media/resolver.rs` (`LiveStatus` enum, probe args, interpretation) and `src/routes/admin/live_status.rs` (badge rendering) — these two change atomically because `badge_parts` matches exhaustively on the enum. A shared UTC formatter in `src/media/mod.rs` serves both parts. No DB, route, or health-checker changes.

**Tech Stack:** Rust 1.96, Axum 0.7, Askama 0.12 (templates are compile-checked), chrono, serde_json, yt-dlp subprocess. Spec: `docs/superpowers/specs/2026-06-10-youtube-discover-improvements-design.md`.

**Conventions:** Run `cargo fmt` before EVERY commit (CI fails on any formatting diff). `cargo clippy -- -D warnings` must stay clean.

---

### Task 1: Shared UTC time formatter

**Files:**
- Modify: `src/media/mod.rs` (new function + test)
- Modify: `src/routes/admin/discover/youtube.rs` (RFC 3339 wrapper + test)

- [ ] **Step 1: Write the failing tests**

Add to the `mod tests` block in `src/media/mod.rs`:

```rust
    #[test]
    fn format_utc_short_formats_epoch() {
        // 1_781_287_200 = 2026-06-12T18:00:00Z
        let dt = chrono::DateTime::from_timestamp(1_781_287_200, 0).unwrap();
        assert_eq!(format_utc_short(dt), "Jun 12 18:00 UTC");
    }
```

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

- [ ] **Step 2: Run tests to verify they fail**

Run: `cargo test --lib format_utc_short format_scheduled_start`
Expected: COMPILE ERROR — `cannot find function format_utc_short` / `format_scheduled_start`

- [ ] **Step 3: Write minimal implementation**

In `src/media/mod.rs`, below `resolve_url`:

```rust
/// Formats a UTC time as e.g. "Jun 12 18:00 UTC" — used by the discover
/// results (scheduled streams) and the live-status badge.
pub(crate) fn format_utc_short(dt: chrono::DateTime<chrono::Utc>) -> String {
    dt.format("%b %d %H:%M UTC").to_string()
}
```

In `src/routes/admin/discover/youtube.rs`, above `parse_iso8601_duration`:

```rust
pub(super) fn format_scheduled_start(rfc3339: &str) -> String {
    chrono::DateTime::parse_from_rfc3339(rfc3339)
        .map(|dt| crate::media::format_utc_short(dt.with_timezone(&chrono::Utc)))
        .unwrap_or_default()
}
```

(chrono is already a dependency — `src/model/channel.rs` uses it.)

- [ ] **Step 4: Run tests to verify they pass**

Run: `cargo test --lib format_utc_short format_scheduled_start`
Expected: 2 tests PASS

- [ ] **Step 5: Format and commit**

```bash
cargo fmt
git add src/media/mod.rs src/routes/admin/discover/youtube.rs
git commit -m "feat: shared UTC short-time formatter for discover and live-status badge"
```

---

### Task 2: Discover row model + extraction (thumbnails, upcoming, scheduled start)

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

### Task 3: Discover template — thumbnail column, UPCOMING badge, badge CSS

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
- The live-status `checking…` span exists ONLY in the `is_live` branch — upcoming rows must NOT trigger the yt-dlp probe (the Data API already told us the state).
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

### Task 4: `LiveStatus` state model + probe + badge rendering

**Files:**
- Modify: `src/media/resolver.rs` — `LiveStatus` enum, `interpret_is_live` → `interpret_live_status`, `probe_live` args, existing tests
- Modify: `src/routes/admin/live_status.rs` — `badge_parts` for all states, `title` becomes `String`, new tests

These two files change atomically: `badge_parts` matches exhaustively on `LiveStatus`, so adding enum variants without updating it breaks the build. (`tests/http.rs` only uses `LiveStatus::Live` and keeps compiling.)

- [ ] **Step 1: Write the failing tests**

In `mod tests` of `src/media/resolver.rs`, DELETE `interpret_is_live_maps_all_cases` and add:

```rust
    #[test]
    fn interpret_live_status_maps_all_cases() {
        use LiveStatus::*;
        assert_eq!(interpret_live_status(true, "is_live|NA\n", ""), Live);
        assert_eq!(
            interpret_live_status(true, "is_upcoming|1781287200\n", ""),
            Upcoming(Some(1781287200))
        );
        assert_eq!(
            interpret_live_status(true, "is_upcoming|NA\n", ""),
            Upcoming(None)
        );
        assert_eq!(interpret_live_status(true, "post_live|NA\n", ""), PostLive);
        assert_eq!(interpret_live_status(true, "was_live|NA\n", ""), WasLive);
        assert_eq!(interpret_live_status(true, "not_live|NA\n", ""), NotLive);
        assert_eq!(interpret_live_status(true, "NA|NA\n", ""), Unknown);
        assert_eq!(interpret_live_status(true, "None|None\n", ""), Unknown);
        assert_eq!(interpret_live_status(true, "", ""), Unknown);
        assert_eq!(
            interpret_live_status(
                false,
                "",
                "ERROR: [youtube] xyz: The channel is not currently live"
            ),
            Offline
        );
        assert_eq!(
            interpret_live_status(
                false,
                "",
                "ERROR: [youtube] xyz: This live event will begin in 3 hours"
            ),
            Upcoming(None)
        );
        assert_eq!(
            interpret_live_status(false, "", "ERROR: network unreachable"),
            Unknown
        );
    }

    #[tokio::test]
    async fn cached_live_status_upcoming_is_determinate_60s_ttl() {
        // Inserted 30s ago: within the 60s determinate TTL, outside the 10s
        // Unknown TTL. If Upcoming were treated as Unknown, this would re-probe
        // (spawning yt-dlp) and not return the cached value.
        let cache: crate::LiveStatusCache =
            std::sync::Arc::new(tokio::sync::RwLock::new(std::collections::HashMap::new()));
        let thirty_secs_ago = std::time::Instant::now()
            .checked_sub(std::time::Duration::from_secs(30))
            .expect("system uptime > 30s");
        cache.write().await.insert(
            "https://www.youtube.com/watch?v=up".to_string(),
            (LiveStatus::Upcoming(Some(1781287200)), thirty_secs_ago),
        );
        assert_eq!(
            cached_live_status(&cache, "https://www.youtube.com/watch?v=up").await,
            LiveStatus::Upcoming(Some(1781287200))
        );
    }
```

In `src/routes/admin/live_status.rs`, add at the bottom:

```rust
#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn badge_parts_maps_every_state() {
        assert_eq!(badge_parts(LiveStatus::Live).label, "live");

        let up = badge_parts(LiveStatus::Upcoming(Some(1781287200)));
        assert_eq!(up.label, "upcoming");
        assert_eq!(up.title, "Scheduled — starts Jun 12 18:00 UTC");
        assert_eq!(
            badge_parts(LiveStatus::Upcoming(None)).title,
            "Scheduled, start time unknown"
        );

        assert_eq!(badge_parts(LiveStatus::PostLive).label, "ended");
        assert_eq!(badge_parts(LiveStatus::WasLive).label, "recorded");
        assert_eq!(badge_parts(LiveStatus::NotLive).label, "vod");
        assert_eq!(badge_parts(LiveStatus::Offline).label, "offline");
        assert_eq!(badge_parts(LiveStatus::Unknown).label, "?");
    }
}
```

- [ ] **Step 2: Run tests to verify they fail**

Run: `cargo test --lib live_status`
Expected: COMPILE ERROR — no variant `Upcoming` on `LiveStatus`, `interpret_live_status` not found

- [ ] **Step 3: Implement the resolver side**

In `src/media/resolver.rs`, replace the `LiveStatus` enum, `interpret_is_live`, and `probe_live` (keep `cached_live_status` as is — its TTL match already treats every non-Unknown state as determinate):

```rust
/// Result of probing a source URL's broadcast lifecycle state, mirroring
/// yt-dlp's `live_status` field. `Upcoming` carries the scheduled start
/// (`release_timestamp`, unix epoch) when yt-dlp reports one.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum LiveStatus {
    Live,
    Upcoming(Option<i64>),
    PostLive,
    WasLive,
    NotLive,
    Offline,
    Unknown,
}

/// Maps `yt-dlp --print "%(live_status)s|%(release_timestamp)s"` output to a
/// `LiveStatus`. On success, stdout is authoritative; `NA`/`None` (extractors
/// without a live_status) are Unknown. On failure, "not currently live" stderr
/// means Offline (yt-dlp exits non-zero for channels with no active broadcast)
/// and "live event will begin" means Upcoming (fallback in case
/// --ignore-no-formats-error does not suppress the error); any other failure
/// is Unknown.
pub fn interpret_live_status(success: bool, stdout: &str, stderr: &str) -> LiveStatus {
    if success {
        let out = stdout.trim();
        let (status, ts) = out.split_once('|').unwrap_or((out, "NA"));
        return match status {
            "is_live" => LiveStatus::Live,
            "is_upcoming" => LiveStatus::Upcoming(ts.parse::<i64>().ok()),
            "post_live" => LiveStatus::PostLive,
            "was_live" => LiveStatus::WasLive,
            "not_live" => LiveStatus::NotLive,
            _ => LiveStatus::Unknown,
        };
    }
    let err = stderr.to_ascii_lowercase();
    if err.contains("not currently live") {
        return LiveStatus::Offline;
    }
    if err.contains("live event will begin") {
        return LiveStatus::Upcoming(None);
    }
    LiveStatus::Unknown
}

/// Probes a YouTube/Twitch URL's broadcast lifecycle state.
/// `--ignore-no-formats-error` lets yt-dlp print metadata for upcoming streams,
/// which have no formats yet. Times out after 8s; any spawn or timeout failure
/// yields `Unknown`.
pub async fn probe_live(url: &str) -> LiveStatus {
    match yt_dlp_output(
        &[
            "--print",
            "%(live_status)s|%(release_timestamp)s",
            "--ignore-no-formats-error",
            "--no-playlist",
        ],
        url,
        Duration::from_secs(8),
        Duration::from_secs(8),
    )
    .await
    {
        Ok(output) => interpret_live_status(
            output.status.success(),
            &String::from_utf8_lossy(&output.stdout),
            &String::from_utf8_lossy(&output.stderr),
        ),
        Err(_) => LiveStatus::Unknown,
    }
}
```

- [ ] **Step 4: Implement the badge side**

In `src/routes/admin/live_status.rs`, change the template struct's `title` field to `String` and replace `badge_parts`:

```rust
#[derive(Template)]
#[template(path = "admin/partials/live_status_badge.html")]
struct LiveStatusBadgeTemplate {
    symbol: &'static str,
    color: &'static str,
    label: &'static str,
    title: String,
}

fn badge_parts(status: LiveStatus) -> LiveStatusBadgeTemplate {
    match status {
        LiveStatus::Live => LiveStatusBadgeTemplate {
            symbol: "●",
            color: "#4caf50",
            label: "live",
            title: "Currently live".to_string(),
        },
        LiveStatus::Upcoming(ts) => {
            let title = ts
                .and_then(|t| chrono::DateTime::from_timestamp(t, 0))
                .map(|dt| {
                    format!("Scheduled — starts {}", crate::media::format_utc_short(dt))
                })
                .unwrap_or_else(|| "Scheduled, start time unknown".to_string());
            LiveStatusBadgeTemplate {
                symbol: "◷",
                color: "#db4",
                label: "upcoming",
                title,
            }
        }
        LiveStatus::PostLive => LiveStatusBadgeTemplate {
            symbol: "◌",
            color: "#f77",
            label: "ended",
            title: "Broadcast just ended (still processing)".to_string(),
        },
        LiveStatus::WasLive => LiveStatusBadgeTemplate {
            symbol: "◉",
            color: "#88f",
            label: "recorded",
            title: "Finished broadcast — recording available".to_string(),
        },
        LiveStatus::NotLive => LiveStatusBadgeTemplate {
            symbol: "▶",
            color: "#88f",
            label: "vod",
            title: "Regular video (never live)".to_string(),
        },
        LiveStatus::Offline => LiveStatusBadgeTemplate {
            symbol: "○",
            color: "#888",
            label: "offline",
            title: "Not currently live".to_string(),
        },
        LiveStatus::Unknown => LiveStatusBadgeTemplate {
            symbol: "·",
            color: "#666",
            label: "?",
            title: "Live status unknown".to_string(),
        },
    }
}
```

(The badge template `templates/admin/partials/live_status_badge.html` renders `{{ title }}` and needs no change.)

- [ ] **Step 5: Run the full suite**

Run: `cargo test`
Expected: all tests pass, including `interpret_live_status_maps_all_cases`, `cached_live_status_upcoming_is_determinate_60s_ttl`, `badge_parts_maps_every_state`, and the pre-existing `cached_live_status_returns_fresh_cache_hit` (uses `LiveStatus::Live`, unaffected). `tests/http.rs` integration tests using `LiveStatus::Live` must also pass.

- [ ] **Step 6: Format and commit**

```bash
cargo fmt
git add src/media/resolver.rs src/routes/admin/live_status.rs
git commit -m "feat: full yt-dlp live_status state model with distinct admin badges"
```

---

### Task 5: `#[ignore]`d yt-dlp pinning test

**Files:**
- Modify: `src/media/resolver.rs` (one test in `mod tests`)

- [ ] **Step 1: Add the ignored test**

```rust
    #[tokio::test]
    #[ignore = "requires yt-dlp and network"]
    async fn probe_live_real_vod_is_not_live() {
        // "Me at the zoo" — a regular upload that was never a live broadcast.
        // Pins that the new probe args (--print "%(live_status)s|%(release_timestamp)s"
        // --ignore-no-formats-error) produce parseable output end-to-end.
        let status = probe_live("https://www.youtube.com/watch?v=jNQXAC9IVRw").await;
        assert_eq!(status, LiveStatus::NotLive);
    }
```

- [ ] **Step 2: Run it once locally (yt-dlp installed)**

Run: `cargo test --lib probe_live_real_vod_is_not_live -- --ignored`
Expected: PASS. If it fails, the real yt-dlp output shape differs from the spec's assumption — inspect with
`yt-dlp --print "%(live_status)s|%(release_timestamp)s" --ignore-no-formats-error --no-playlist "https://www.youtube.com/watch?v=jNQXAC9IVRw"`
and adjust `interpret_live_status` (NOT the test) to match reality, then re-run Task 4's tests. If an upcoming stream URL is available at implementation time (e.g. a scheduled launch), also probe it manually and confirm `Upcoming(Some(_))`.

- [ ] **Step 3: Confirm the default suite still skips it**

Run: `cargo test --lib probe_live_real_vod_is_not_live`
Expected: `1 ignored`

- [ ] **Step 4: Format and commit**

```bash
cargo fmt
git add src/media/resolver.rs
git commit -m "test: pin real yt-dlp live_status output shape (ignored, needs network)"
```

---

### Task 6: Docs + final verification

**Files:**
- Modify: `docs/IDEAS.md:47` (item #35)

- [ ] **Step 1: Check for unrelated pending edits**

Run: `git diff docs/IDEAS.md`
`docs/IDEAS.md` had uncommitted modifications before this work started. If the diff contains changes unrelated to item #35, do NOT commit them silently — surface them to the user and only stage the #35 edit (use `git add -p docs/IDEAS.md` if needed).

- [ ] **Step 2: Mark idea #35 done**

Replace the item #35 paragraph in `docs/IDEAS.md` with:

```markdown
35. ~~**YouTube Discover improvements**~~ — done: gaps (1) `source_kind` VOD detection and (2) `type=channel` search + channel-URL resolve were already fixed earlier; this change adds (3) thumbnails (`snippet.thumbnails.default`, lazy-loaded 80 px column for video and channel results) and (4) upcoming-stream handling (`liveBroadcastContent="upcoming"` → amber UPCOMING badge + scheduled start from `liveStreamingDetails.scheduledStartTime`, addable as a `youtube_live` source that activates when the stream goes live). Also extends the yt-dlp live-status probe to the full `live_status` state model (`LiveStatus`: Live / Upcoming(ts) / PostLive / WasLive / NotLive / Offline / Unknown) with distinct admin badges — the state foundation for ideas #38/#39. Spec: `docs/superpowers/specs/2026-06-10-youtube-discover-improvements-design.md`.
```

- [ ] **Step 3: Full verification**

```bash
cargo fmt --check
cargo clippy -- -D warnings
cargo test
```

Expected: no formatting diff, no clippy warnings, all tests pass (319+ tests; 6 ignored needing yt-dlp/network — was 5, Task 5 added one).

- [ ] **Step 4: Commit**

```bash
git add docs/IDEAS.md
git commit -m "docs: mark idea #35 (YouTube Discover improvements + live_status states) done"
```
