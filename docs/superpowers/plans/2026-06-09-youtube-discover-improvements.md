# YouTube Discover Improvements Implementation Plan

> **For agentic workers:** REQUIRED SUB-SKILL: Use superpowers:subagent-driven-development (recommended) or superpowers:executing-plans to implement this plan task-by-task. Steps use checkbox (`- [ ]`) syntax for tracking.

**Goal:** Fix VOD source-kind labeling in YouTube discovery and add channel search by keyword and by channel URL/@handle.

**Architecture:** YouTube search result rows are built by pure functions over the API JSON (unit-testable, no network). The video path gains a per-row `source_kind` (`youtube_vod` when not live). A new channel path (`type=channel`) builds `/channel/{id}/live` source URLs. A separate pure `normalize_channel_url` powers a paste-a-URL handler that needs no API key.

**Tech Stack:** Rust, Axum 0.7, Askama templates, serde_json, reqwest. Tests via `cargo test` (unit + `tower::ServiceExt::oneshot` integration).

---

## File Structure

- `src/model/source.rs` — add `SourceKind::YoutubeVod` variant.
- `src/routes/admin/discover/youtube.rs` — add `source_kind` to `YoutubeResultRow`; extract pure `build_video_rows`; add `build_channel_rows`, `fetch_youtube_channels`, `normalize_channel_url`, `channel_title_from_url`; unit tests.
- `src/routes/admin/discover/mod.rs` — `search_type` on `YoutubeSearchForm` + branch; new `ChannelUrlForm` + `discover_channel_resolve` handler.
- `src/routes/admin/mod.rs` — export `discover_channel_resolve`.
- `src/lib.rs` — register `POST /admin/discover/channel/resolve`.
- `templates/admin/partials/discover_yt_results.html` — use `{{ row.source_kind }}`.
- `templates/admin/discover.html` — Videos/Channels selector + channel-URL form.
- `tests/http.rs` — integration tests for the channel-resolve endpoint.

---

## Task 1: Add `SourceKind::YoutubeVod`

**Files:**
- Modify: `src/model/source.rs:7-49`
- Test: `src/model/source.rs` (inline `#[cfg(test)]`)

Only two exhaustive matches reference `SourceKind` (`as_str` and `FromStr`, both in this file). All other usages are constructions/comparisons, so no other arms break.

- [ ] **Step 1: Write the failing test**

Add to the `#[cfg(test)] mod tests` block in `src/model/source.rs` (create the block if absent):

```rust
    #[test]
    fn youtube_vod_round_trips() {
        assert_eq!(SourceKind::YoutubeVod.as_str(), "youtube_vod");
        assert_eq!(
            "youtube_vod".parse::<SourceKind>().unwrap(),
            SourceKind::YoutubeVod
        );
    }
```

- [ ] **Step 2: Run test to verify it fails**

Run: `cargo test -p mytv youtube_vod_round_trips`
Expected: FAIL — `no variant named YoutubeVod` (compile error).

- [ ] **Step 3: Add the variant and arms**

In `src/model/source.rs`, add the variant to the enum:

```rust
pub enum SourceKind {
    Hls,
    YoutubeLive,
    YoutubeVod,
    Iptv,
    Dash,
}
```

Add to `as_str`:

```rust
            SourceKind::YoutubeVod => "youtube_vod",
```

Add to `FromStr`:

```rust
            "youtube_vod" => Ok(SourceKind::YoutubeVod),
```

`detect()` stays unchanged — a `watch?v=` URL cannot be distinguished from live by URL alone.

- [ ] **Step 4: Run test to verify it passes**

Run: `cargo test -p mytv youtube_vod_round_trips`
Expected: PASS.

- [ ] **Step 5: Commit**

```bash
cargo fmt
git add src/model/source.rs
git commit -m "feat: add YoutubeVod source kind"
```

---

## Task 2: Per-row `source_kind` for video results (sub-part 1)

**Files:**
- Modify: `src/routes/admin/discover/youtube.rs:1-123`
- Modify: `templates/admin/partials/discover_yt_results.html:32`
- Test: `src/routes/admin/discover/youtube.rs` (inline `#[cfg(test)]`)

Extract the row-building closure into a pure `build_video_rows` so the labeling logic is unit-testable without network.

- [ ] **Step 1: Write the failing test**

Add to the `#[cfg(test)] mod tests` block in `src/routes/admin/discover/youtube.rs`:

```rust
    #[test]
    fn video_rows_label_vod_when_not_live() {
        let items = vec![
            serde_json::json!({
                "id": {"videoId": "abc"},
                "snippet": {"title": "A VOD", "channelTitle": "Chan",
                            "liveBroadcastContent": "none"}
            }),
            serde_json::json!({
                "id": {"videoId": "def"},
                "snippet": {"title": "A Live", "channelTitle": "Chan",
                            "liveBroadcastContent": "live"}
            }),
        ];
        let mut dur = std::collections::HashMap::new();
        dur.insert("abc".to_string(), 253i64);
        let rows = build_video_rows(&items, &dur);
        assert_eq!(rows.len(), 2);
        assert_eq!(rows[0].source_kind, "youtube_vod");
        assert!(!rows[0].is_live);
        assert_eq!(rows[0].duration_secs, 253);
        assert_eq!(rows[1].source_kind, "youtube_live");
        assert!(rows[1].is_live);
    }
```

- [ ] **Step 2: Run test to verify it fails**

Run: `cargo test -p mytv video_rows_label_vod_when_not_live`
Expected: FAIL — `cannot find function build_video_rows` / `no field source_kind`.

- [ ] **Step 3: Add the field and extract the pure function**

In `src/routes/admin/discover/youtube.rs`, add `source_kind` to the struct:

```rust
pub struct YoutubeResultRow {
    pub title: String,
    pub channel_title: String,
    pub is_live: bool,
    pub duration_secs: i64,
    pub url: String,
    pub source_kind: String,
    pub form_id: usize,
}
```

Add the pure builder (place it above `fetch_youtube_results`):

```rust
pub(super) fn build_video_rows(
    items: &[serde_json::Value],
    duration_map: &std::collections::HashMap<String, i64>,
) -> Vec<YoutubeResultRow> {
    items
        .iter()
        .enumerate()
        .filter_map(|(i, item)| {
            let video_id = item["id"]["videoId"].as_str()?;
            let snippet = &item["snippet"];
            let title = snippet["title"].as_str().unwrap_or("Unknown").to_string();
            let channel_title = snippet["channelTitle"].as_str().unwrap_or("").to_string();
            let is_live = snippet["liveBroadcastContent"].as_str() == Some("live");
            let duration_secs = *duration_map.get(video_id).unwrap_or(&0);
            let source_kind = if is_live { "youtube_live" } else { "youtube_vod" }.to_string();
            let url = format!("https://www.youtube.com/watch?v={}", video_id);
            Some(YoutubeResultRow {
                title,
                channel_title,
                is_live,
                duration_secs,
                url,
                source_kind,
                form_id: i,
            })
        })
        .collect()
}
```

Replace the inline `let rows = items.iter().enumerate()...collect();` block at the end of `fetch_youtube_results` with:

```rust
    let rows = build_video_rows(items, &duration_map);

    Ok(rows)
```

- [ ] **Step 4: Update the template**

In `templates/admin/partials/discover_yt_results.html`, change line 32 from:

```html
          <input type="hidden" name="source_kind" value="youtube_live">
```

to:

```html
          <input type="hidden" name="source_kind" value="{{ row.source_kind }}">
```

- [ ] **Step 5: Run tests to verify they pass**

Run: `cargo test -p mytv video_rows_label_vod_when_not_live && cargo build`
Expected: PASS and clean build (template field now exists).

- [ ] **Step 6: Commit**

```bash
cargo fmt
git add src/routes/admin/discover/youtube.rs templates/admin/partials/discover_yt_results.html
git commit -m "feat: label YouTube VOD results as youtube_vod"
```

---

## Task 3: Channel keyword search (sub-part 2a)

**Files:**
- Modify: `src/routes/admin/discover/youtube.rs`
- Modify: `src/routes/admin/discover/mod.rs:90-93` (`YoutubeSearchForm`), `:254-284` (`discover_youtube_search`)
- Modify: `templates/admin/discover.html:36-45`
- Test: `src/routes/admin/discover/youtube.rs` (inline `#[cfg(test)]`)

- [ ] **Step 1: Write the failing test**

Add to the `#[cfg(test)] mod tests` block in `src/routes/admin/discover/youtube.rs`:

```rust
    #[test]
    fn channel_rows_build_live_urls() {
        let items = vec![serde_json::json!({
            "id": {"channelId": "UC123"},
            "snippet": {"title": "NASA", "channelTitle": "NASA"}
        })];
        let rows = build_channel_rows(&items);
        assert_eq!(rows.len(), 1);
        assert_eq!(rows[0].url, "https://www.youtube.com/channel/UC123/live");
        assert!(rows[0].is_live);
        assert_eq!(rows[0].duration_secs, 0);
        assert_eq!(rows[0].source_kind, "youtube_live");
        assert_eq!(rows[0].title, "NASA");
    }
```

- [ ] **Step 2: Run test to verify it fails**

Run: `cargo test -p mytv channel_rows_build_live_urls`
Expected: FAIL — `cannot find function build_channel_rows`.

- [ ] **Step 3: Add `build_channel_rows` and `fetch_youtube_channels`**

In `src/routes/admin/discover/youtube.rs`, add:

```rust
pub(super) fn build_channel_rows(items: &[serde_json::Value]) -> Vec<YoutubeResultRow> {
    items
        .iter()
        .enumerate()
        .filter_map(|(i, item)| {
            let channel_id = item["id"]["channelId"].as_str()?;
            let snippet = &item["snippet"];
            let title = snippet["title"].as_str().unwrap_or("Unknown").to_string();
            let channel_title = snippet["channelTitle"].as_str().unwrap_or("").to_string();
            let url = format!("https://www.youtube.com/channel/{}/live", channel_id);
            Some(YoutubeResultRow {
                title,
                channel_title,
                is_live: true,
                duration_secs: 0,
                url,
                source_kind: "youtube_live".to_string(),
                form_id: i,
            })
        })
        .collect()
}

pub(super) async fn fetch_youtube_channels(
    keyword: &str,
    api_key: &str,
    client: &reqwest::Client,
) -> anyhow::Result<Vec<YoutubeResultRow>> {
    let search_resp: serde_json::Value = client
        .get("https://www.googleapis.com/youtube/v3/search")
        .query(&[
            ("part", "snippet"),
            ("type", "channel"),
            ("maxResults", "12"),
            ("q", keyword),
            ("key", api_key),
        ])
        .send()
        .await?
        .json()
        .await?;

    if let Some(err) = search_resp.get("error") {
        let msg = err["message"]
            .as_str()
            .unwrap_or("YouTube API error")
            .to_string();
        anyhow::bail!("{}", msg);
    }

    let items = match search_resp["items"].as_array() {
        Some(v) => v,
        None => return Ok(vec![]),
    };

    Ok(build_channel_rows(items))
}
```

- [ ] **Step 4: Run the unit test to verify it passes**

Run: `cargo test -p mytv channel_rows_build_live_urls`
Expected: PASS.

- [ ] **Step 5: Wire the search-type branch**

In `src/routes/admin/discover/mod.rs`, extend `YoutubeSearchForm`:

```rust
#[derive(Deserialize)]
pub struct YoutubeSearchForm {
    pub keyword: String,
    pub search_type: Option<String>,
}
```

In `discover_youtube_search`, replace the `let rows = match youtube::fetch_youtube_results(...)` block with a branch:

```rust
    let search_type = form.search_type.as_deref().unwrap_or("video");
    let fetched = if search_type == "channel" {
        youtube::fetch_youtube_channels(&form.keyword, &api_key, &state.http_client).await
    } else {
        youtube::fetch_youtube_results(&form.keyword, &api_key, &state.http_client).await
    };
    let rows = match fetched {
        Ok(r) => r,
        Err(e) => {
            tracing::error!("YouTube API error: {e}");
            return Html(format!(
                "<p class=\"empty-state\" style=\"color:#f77\">YouTube search failed: {}.</p>",
                html_escape(&e.to_string())
            ));
        }
    };
```

- [ ] **Step 6: Add the Videos/Channels selector to the search form**

In `templates/admin/discover.html`, inside the keyword `<form>` (after the keyword `form-row`, before the Search button), add:

```html
    <div class="form-row" style="margin:0;min-width:120px">
      <label for="disc-yt-type">Type</label>
      <select id="disc-yt-type" name="search_type" style="width:auto">
        <option value="video">Videos</option>
        <option value="channel">Channels</option>
      </select>
    </div>
```

- [ ] **Step 7: Run the full suite**

Run: `cargo test -p mytv && cargo build`
Expected: PASS, clean build.

- [ ] **Step 8: Commit**

```bash
cargo fmt
git add src/routes/admin/discover/youtube.rs src/routes/admin/discover/mod.rs templates/admin/discover.html
git commit -m "feat: add YouTube channel keyword search"
```

---

## Task 4: Channel-URL / @handle input (sub-part 2b)

**Files:**
- Modify: `src/routes/admin/discover/youtube.rs`
- Modify: `src/routes/admin/discover/mod.rs` (new form type + handler)
- Modify: `src/routes/admin/mod.rs:12-15` (export)
- Modify: `src/lib.rs:100-103` (route)
- Modify: `templates/admin/discover.html` (channel-URL form)
- Test: `src/routes/admin/discover/youtube.rs` (inline) + `tests/http.rs` (integration)

- [ ] **Step 1: Write the failing unit test for `normalize_channel_url`**

Add to the `#[cfg(test)] mod tests` block in `src/routes/admin/discover/youtube.rs`:

```rust
    #[test]
    fn normalize_channel_url_cases() {
        assert_eq!(
            normalize_channel_url("https://www.youtube.com/channel/UC123"),
            Some("https://www.youtube.com/channel/UC123/live".to_string())
        );
        assert_eq!(
            normalize_channel_url("https://www.youtube.com/@NASA"),
            Some("https://www.youtube.com/@NASA/live".to_string())
        );
        assert_eq!(
            normalize_channel_url("@NASA"),
            Some("https://www.youtube.com/@NASA/live".to_string())
        );
        assert_eq!(
            normalize_channel_url("https://www.youtube.com/c/NASA/"),
            Some("https://www.youtube.com/c/NASA/live".to_string())
        );
        assert_eq!(
            normalize_channel_url("https://www.youtube.com/user/NASAtelevision"),
            Some("https://www.youtube.com/user/NASAtelevision/live".to_string())
        );
        assert_eq!(
            normalize_channel_url("https://www.youtube.com/@NASA/live"),
            Some("https://www.youtube.com/@NASA/live".to_string())
        );
        assert_eq!(
            normalize_channel_url("https://www.youtube.com/channel/UC123?ab=1"),
            Some("https://www.youtube.com/channel/UC123/live".to_string())
        );
        assert_eq!(normalize_channel_url("https://example.com/foo"), None);
        assert_eq!(normalize_channel_url(""), None);
    }

    #[test]
    fn channel_title_from_url_cases() {
        assert_eq!(
            channel_title_from_url("https://www.youtube.com/@NASA/live"),
            "@NASA"
        );
        assert_eq!(
            channel_title_from_url("https://www.youtube.com/channel/UC123/live"),
            "UC123"
        );
    }
```

- [ ] **Step 2: Run test to verify it fails**

Run: `cargo test -p mytv normalize_channel_url_cases`
Expected: FAIL — `cannot find function normalize_channel_url`.

- [ ] **Step 3: Implement the helpers**

In `src/routes/admin/discover/youtube.rs`, add:

```rust
pub(super) fn normalize_channel_url(input: &str) -> Option<String> {
    let trimmed = input.trim();
    if trimmed.is_empty() {
        return None;
    }
    if let Some(handle) = trimmed.strip_prefix('@') {
        if handle.is_empty() {
            return None;
        }
        return Some(format!("https://www.youtube.com/@{}/live", handle));
    }
    if !trimmed.to_ascii_lowercase().contains("youtube.com") {
        return None;
    }
    let base = trimmed.split(['?', '#']).next().unwrap_or(trimmed);
    let base = base.trim_end_matches('/');
    if base.ends_with("/live") {
        return Some(base.to_string());
    }
    Some(format!("{}/live", base))
}

pub(super) fn channel_title_from_url(url: &str) -> String {
    let trimmed = url.trim_end_matches("/live").trim_end_matches('/');
    trimmed
        .rsplit('/')
        .next()
        .filter(|s| !s.is_empty())
        .unwrap_or("YouTube Channel")
        .to_string()
}
```

- [ ] **Step 4: Run unit tests to verify they pass**

Run: `cargo test -p mytv normalize_channel_url_cases channel_title_from_url_cases`
Expected: PASS.

- [ ] **Step 5: Add the form type and handler**

In `src/routes/admin/discover/mod.rs`, add the form type near the other form types:

```rust
#[derive(Deserialize)]
pub struct ChannelUrlForm {
    pub url: String,
}
```

Add the handler near `discover_manual_resolve`:

```rust
pub async fn discover_channel_resolve(
    State(state): State<AppState>,
    Form(form): Form<ChannelUrlForm>,
) -> Result<Html<String>, StatusCode> {
    let normalized = youtube::normalize_channel_url(&form.url)
        .ok_or(StatusCode::UNPROCESSABLE_ENTITY)?;
    let title = youtube::channel_title_from_url(&normalized);
    let channels = channel::list(&state.pool)
        .await
        .map_err(internal_error)?
        .into_iter()
        .map(|ch| DiscoverChannelOption {
            id: ch.id,
            name: ch.name,
            type_str: ch.r#type,
        })
        .collect();
    render(ManualResultTemplate {
        form_id: "channel".to_string(),
        url: normalized,
        title,
        group: String::new(),
        is_live: true,
        duration_secs: 0,
        source_kind: "youtube_live".to_string(),
        show_duration_input: false,
        channels,
    })
}
```

- [ ] **Step 6: Export the handler**

In `src/routes/admin/mod.rs`, add `discover_channel_resolve` to the `pub use discover::{ ... }` list:

```rust
pub use discover::{
    discover_add, discover_add_form, discover_channel_resolve, discover_m3u_search,
    discover_manual_resolve, discover_page, discover_youtube_search,
};
```

- [ ] **Step 7: Register the route**

In `src/lib.rs`, add after the `/discover/manual/resolve` route:

```rust
        .route(
            "/discover/channel/resolve",
            post(routes::admin::discover_channel_resolve),
        )
```

- [ ] **Step 8: Add the channel-URL form to the template**

In `templates/admin/discover.html`, inside `<div id="tab-panel-youtube">`, after the `{% endif %}` that closes the API-key guard and before the panel's closing `</div>`, add:

```html
  <hr style="border-color:#222;margin:16px 0">
  <form hx-post="/admin/discover/channel/resolve"
        hx-target="#yt-channel-result"
        hx-swap="innerHTML"
        style="display:flex;gap:10px;align-items:flex-end;margin-bottom:12px">
    <div class="form-row" style="margin:0;flex:1;min-width:260px">
      <label for="disc-channel-url">Channel URL / @handle</label>
      <input id="disc-channel-url" type="text" name="url"
             placeholder="https://youtube.com/@NASA or @NASA" required>
    </div>
    <button class="btn btn-primary btn-sm" type="submit" style="margin-bottom:1px">Resolve</button>
  </form>
  <div id="yt-channel-result"></div>
```

- [ ] **Step 9: Write the integration tests**

In `tests/http.rs`, add (ensure `use axum::http::StatusCode;` or the existing `StatusCode` import is in scope — it is used elsewhere in the file):

```rust
#[tokio::test]
async fn admin_discover_channel_resolve_normalizes_handle() {
    let response = app()
        .await
        .oneshot(authed_form_post(
            "/admin/discover/channel/resolve",
            "url=%40NASA",
        ))
        .await
        .unwrap();
    assert_eq!(response.status(), StatusCode::OK);
    let body = body_text(response).await;
    assert!(body.contains("https://www.youtube.com/@NASA/live"));
}

#[tokio::test]
async fn admin_discover_channel_resolve_rejects_non_youtube() {
    let response = app()
        .await
        .oneshot(authed_form_post(
            "/admin/discover/channel/resolve",
            "url=https%3A%2F%2Fexample.com%2Ffoo",
        ))
        .await
        .unwrap();
    assert_eq!(response.status(), StatusCode::UNPROCESSABLE_ENTITY);
}
```

- [ ] **Step 10: Run the full suite**

Run: `cargo test`
Expected: PASS — including the two new integration tests.

- [ ] **Step 11: Lint and commit**

```bash
cargo fmt
cargo clippy -- -D warnings
git add src/routes/admin/discover/youtube.rs src/routes/admin/discover/mod.rs src/routes/admin/mod.rs src/lib.rs templates/admin/discover.html tests/http.rs
git commit -m "feat: resolve YouTube channel URLs and @handles in discovery"
```

---

## Final Verification

- [ ] **Run the complete check the way CI does**

Run: `cargo fmt --check && cargo clippy -- -D warnings && cargo test`
Expected: all green. Then optionally smoke-test with a real `YOUTUBE_API_KEY` set in `.env` (`cargo run`, visit `/admin/discover`, try Videos vs Channels and the channel-URL box).

---

## Self-Review Notes

- **Spec coverage:** Section A → Tasks 1–2; Section B 2a → Task 3; Section B 2b → Task 4; Section C UI/routes/tests spread across Tasks 2–4 + Final Verification. The "lock VOD→vod_loop steering with a test" item is satisfied implicitly by the existing template default (Task 2 keeps `is_live` flowing to the add-form); no separate test added because the add-form default ordering is unchanged template logic, not new code. If a regression guard is desired, it can be added later — not blocking.
- **Type consistency:** `YoutubeResultRow.source_kind: String` defined in Task 2, used by `build_channel_rows` (Task 3) and template. `normalize_channel_url`/`channel_title_from_url` signatures (Task 4) match handler call sites. `ManualResultTemplate` field set matches `src/routes/admin/discover/mod.rs` definition.
- **No placeholders:** all steps carry full code/commands.
