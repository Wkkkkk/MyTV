# MyTV Plan 5: Discovery Tools Design

**Goal:** Add a `/admin/discover` page with three tools — YouTube search, iptv-org M3U import, and manual URL entry — that let users find streams and add them to channels with minimal friction.

**Architecture:** New `admin_discover.rs` route module wired into the existing auth-protected admin router. HTMX-powered tabs for each tool; results load inline without full-page reloads. A shared "add form" lets the user create a new channel or add to an existing one. All mutations go through the existing `channel`, `source`, and `playlist_item` DB modules. External HTTP (YouTube API, M3U fetch) handled by a new `reqwest` dependency.

**Tech Stack:** Rust, Axum 0.7, Askama 0.12, sqlx 0.7 (SQLite), reqwest 0.12, htmx 1.9, existing `resolver.rs` for yt-dlp calls.

---

## Dependencies

Add to `Cargo.toml`:
```toml
reqwest = { version = "0.12", features = ["rustls-tls"], default-features = false }
```

---

## Configuration

`src/config.rs` gains:
```rust
pub youtube_api_key: Option<String>,  // from YOUTUBE_API_KEY env var, None if unset
```

---

## File Structure

```
Cargo.toml                                              — add reqwest
src/
  config.rs                                             — add youtube_api_key: Option<String>
  routes/
    mod.rs                                              — add pub mod admin_discover
    admin_discover.rs                                   — NEW: all discover handlers
  main.rs                                               — wire discover routes into admin router
templates/
  admin/
    discover.html                                       — NEW: page shell, three HTMX tabs
    partials/
      discover_yt_results.html                          — NEW: YouTube result rows
      discover_m3u_results.html                         — NEW: M3U result rows
      discover_manual_result.html                       — NEW: resolved URL metadata + add form
      discover_add_form.html                            — NEW: inline new/existing channel form
```

---

## Routes

```
GET  /admin/discover                      — discover page (three tabs, initial state)
POST /admin/discover/youtube/search       — returns YouTube results partial
POST /admin/discover/m3u/search           — fetches + filters iptv-org M3U, returns partial
POST /admin/discover/manual/resolve       — resolves URL metadata, returns manual result partial
GET  /admin/discover/add-form             — returns inline add form for a given result
POST /admin/discover/add                  — commits the add, redirects to /admin/channels/:id
```

All routes inherit HTTP Basic Auth from the admin `route_layer`.

---

## Tab 1: YouTube Search

**When `YOUTUBE_API_KEY` is not set:** The YouTube tab body shows:
> "Set the `YOUTUBE_API_KEY` environment variable to enable YouTube search."
No search form is rendered.

**When configured:** A keyword input form is shown. `POST /admin/discover/youtube/search` calls:
```
https://www.googleapis.com/youtube/v3/search
  ?part=snippet&type=video,channel&maxResults=12&q={keyword}&key={api_key}
```
For each video result, a second call fetches `contentDetails` (duration in ISO 8601). Duration is converted to seconds server-side.

Results partial (`discover_yt_results.html`) shows per row: title, channel name, LIVE/VOD badge, duration. Each row has an "Add" button:
```html
hx-get="/admin/discover/add-form?url=...&title=...&is_live=true&duration_secs=0"
hx-target="#add-form-{id}"
hx-swap="innerHTML"
```

---

## Tab 2: M3U Import

Filter form with two optional text inputs: **Country** and **Category** (both case-insensitive substring match against the M3U metadata).

`POST /admin/discover/m3u/search`:
1. Fetches `https://iptv-org.github.io/iptv/index.m3u` with a 10s timeout.
2. Parses into `Vec<M3uChannel>`:
   ```rust
   struct M3uChannel {
       name: String,
       group: String,   // group-title attribute
       country: String, // country attribute (may be empty)
       url: String,
   }
   ```
3. Filters: country substring match AND/OR group substring match (if either input is non-empty, both are checked independently — a channel passes if it matches at least the non-empty inputs).
4. Returns first 50 matches.

Results partial (`discover_m3u_results.html`) shows per row: name, group, country, URL (truncated). Each row has an "Add" button using the same `hx-get` add-form mechanism.

If the fetch fails → inline error message. If no results after filtering → "No channels found — try different filters."

---

## Tab 3: Manual Entry

Single URL field. `POST /admin/discover/manual/resolve`:
1. Validates URL scheme (`http://` or `https://`).
2. Calls `resolver::needs_resolution(url)` to classify.
3. If it's a YouTube URL: calls `resolver::fetch_duration_secs(url)` with a 5s timeout to get VOD duration. On failure, duration stays 0 and the add form shows a manual "Duration (seconds)" input.
4. Returns `discover_manual_result.html` partial: URL, detected type (live stream / VOD), duration if known, and the pre-populated add form inline.

---

## Common Add Form (`discover_add_form.html`)

Injected inline below the clicked result row via HTMX. Carries stream metadata as hidden fields: `url`, `title`, `is_live`, `duration_secs`, `source_kind`.

**Source kind auto-detection** (server-side, `detect_source_kind(url: &str) -> &str`):
- YouTube URL → `"youtube_live"`
- URL contains `.m3u8` → `"hls"`
- Anything else → `"iptv"`

**Radio: New Channel | Add to Existing**

*New Channel (default):*
| Field | Default |
|---|---|
| Name | Pre-filled from result title |
| Category | Empty (required) |
| Channel type | `live` if `is_live`, else `vod_loop` |
| Sort order | `0` |

*Add to Existing:*
- Dropdown of all channels (loaded at discover page render, available as template variable `channels: Vec<AdminChannelRow>`).

---

## Add Handler (`POST /admin/discover/add`)

Determines action by target channel type:
- `channel_type == "vod_loop"` → create **playlist item** (title, url, duration_secs, sort_order = existing count)
- otherwise → create **source** (kind, url, priority = 0, is_active = true)

Four paths:

| Choice | channel_type | Action |
|---|---|---|
| New channel | live | `channel::create` → `source::create` |
| New channel | vod_loop | `channel::create` (loop_anchor = now) → `playlist_item::create` |
| Existing channel | live/iptv | `source::create` |
| Existing channel | vod_loop | `playlist_item::create` |

**Validation (returns 422 on failure):**
- URL non-empty and valid scheme
- Name non-empty (new channel)
- Category non-empty (new channel)
- `channel_type` ∈ `["live", "vod_loop"]`
- `source_kind` ∈ `["hls", "youtube_live", "iptv"]`
- `duration_secs > 0` when adding playlist item

On success: `Redirect::to("/admin/channels/:id")` (303).

---

## Error Handling

| Scenario | Behavior |
|---|---|
| YouTube API error / quota exceeded | Inline error in tab partial |
| M3U fetch timeout (>10s) | Inline error in tab partial |
| `fetch_duration_secs` failure | duration=0, manual input field shown |
| Validation failure on add | 422 Unprocessable Entity |
| Channel not found on add-to-existing | 404 Not Found |

---

## Testing

**Unit tests in `admin_discover.rs`:**

1. `parse_m3u` — M3U parser: standard entry, missing optional attributes, malformed lines (no URL line, no `#EXTINF` prefix), multi-entry file, filter matching by country, filter matching by group, both filters, no filters (returns all up to 50).
2. `detect_source_kind` — YouTube URL → `"youtube_live"`, `.m3u8` URL → `"hls"`, other → `"iptv"`.

**Integration tests (`POST /admin/discover/add`) using sqlx test pool:**
1. New channel + source (live)
2. New channel + playlist item (vod_loop)
3. Existing channel + source
4. Existing channel + playlist item
5. Validation failure: missing category
6. Validation failure: zero duration on VOD playlist item

**Not unit-tested (require network):**
- YouTube API call (isolated in `fetch_youtube_results(keyword, api_key) -> Result<Vec<YoutubeResult>>`)
- M3U HTTP fetch (isolated in `fetch_m3u() -> Result<String>`)
- Both are marked `#[ignore]` if tested at all, with a comment explaining the network requirement.
