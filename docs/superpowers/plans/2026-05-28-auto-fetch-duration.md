# Auto-Fetch Duration Implementation Plan

> **For agentic workers:** REQUIRED SUB-SKILL: Use superpowers:subagent-driven-development (recommended) or superpowers:executing-plans to implement this plan task-by-task. Steps use checkbox (`- [ ]`) syntax for tracking.

**Goal:** When an admin adds a VOD playlist item with a YouTube URL and leaves Duration blank, automatically fetch the duration via yt-dlp instead of returning 422.

**Architecture:** The `playlist_item_create` handler in `src/routes/admin.rs` gains an auto-fetch branch: if `duration_secs == 0` and the URL matches `resolver::needs_resolution()`, it calls `resolver::fetch_duration_secs(url)` before creating the item. The Duration field in the admin form becomes optional for YouTube URLs via a placeholder and `required` removal.

**Tech Stack:** Rust, Axum 0.7, Askama 0.12, yt-dlp (subprocess via existing `resolver::fetch_duration_secs`)

---

## File Map

| File | Change |
|---|---|
| `src/routes/admin.rs` | Add `resolver` import; update `playlist_item_create` handler |
| `templates/admin/channel_detail.html` | Update Duration input: remove `required`, change `min`, update placeholder |

---

### Task 1: Update playlist_item_create handler

**Files:**
- Modify: `src/routes/admin.rs`

---

- [ ] **Step 1: Add `resolver` to imports**

Line 12 currently reads:
```rust
use crate::{channel, epg, playlist_item, source, AppState};
```

Change it to:
```rust
use crate::{channel, epg, playlist_item, resolver, source, AppState};
```

---

- [ ] **Step 2: Replace the handler body**

The `playlist_item_create` function currently reads (lines 468–496):

```rust
pub async fn playlist_item_create(
    State(state): State<AppState>,
    Path(channel_id): Path<i64>,
    Form(form): Form<PlaylistItemForm>,
) -> Result<impl IntoResponse, StatusCode> {
    let duration_secs: i64 = form.duration_secs.trim().parse().unwrap_or(0);
    if duration_secs <= 0 {
        return Err(StatusCode::UNPROCESSABLE_ENTITY);
    }

    let existing = playlist_item::list_for_channel(&state.pool, channel_id)
        .await
        .map_err(internal_error)?;
    let sort_order = existing.len() as i64;

    playlist_item::create(
        &state.pool,
        playlist_item::NewPlaylistItem {
            channel_id,
            title: form.title.trim().to_string(),
            url: form.url.trim().to_string(),
            duration_secs,
            sort_order,
        },
    )
    .await
    .map_err(internal_error)?;
    Ok(Redirect::to(&format!("/admin/channels/{channel_id}")))
}
```

Replace it with:

```rust
pub async fn playlist_item_create(
    State(state): State<AppState>,
    Path(channel_id): Path<i64>,
    Form(form): Form<PlaylistItemForm>,
) -> Result<impl IntoResponse, StatusCode> {
    let url = form.url.trim().to_string();
    let mut duration_secs: i64 = form.duration_secs.trim().parse().unwrap_or(0);
    if duration_secs <= 0 {
        if resolver::needs_resolution(&url) {
            duration_secs = resolver::fetch_duration_secs(&url).await.map_err(|e| {
                tracing::warn!(url = %url, error = %e, "failed to auto-fetch duration");
                StatusCode::UNPROCESSABLE_ENTITY
            })?;
        } else {
            return Err(StatusCode::UNPROCESSABLE_ENTITY);
        }
    }

    let existing = playlist_item::list_for_channel(&state.pool, channel_id)
        .await
        .map_err(internal_error)?;
    let sort_order = existing.len() as i64;

    playlist_item::create(
        &state.pool,
        playlist_item::NewPlaylistItem {
            channel_id,
            title: form.title.trim().to_string(),
            url,
            duration_secs,
            sort_order,
        },
    )
    .await
    .map_err(internal_error)?;
    Ok(Redirect::to(&format!("/admin/channels/{channel_id}")))
}
```

Key changes:
- `url` extracted to a `let` binding first (used in both the resolver check and `NewPlaylistItem`)
- `duration_secs` is now `mut`
- `if duration_secs <= 0` branches on `resolver::needs_resolution(&url)`: auto-fetches for YouTube/Twitch, still returns 422 for plain URLs
- `url` reused in `NewPlaylistItem` (no second `.trim()` needed)

---

- [ ] **Step 3: Run tests**

```bash
cd /Users/kunwu/Workspace/playground/MyTV && cargo test
```

Expected: all existing tests pass. If there are compile errors, verify `resolver` is in the import list on line 12 and the function signature is `pub async fn`.

---

- [ ] **Step 4: Commit**

```bash
git add src/routes/admin.rs
git commit -m "feat: auto-fetch duration for YouTube playlist items"
```

---

### Task 2: Update Duration input in channel_detail.html

**Files:**
- Modify: `templates/admin/channel_detail.html`

---

- [ ] **Step 1: Update the Duration input**

In `templates/admin/channel_detail.html`, find the Duration input (inside the "Add Item" form at the bottom of the Playlist section):

```html
<input type="number" name="duration_secs" required min="1" placeholder="3600">
```

Replace it with:

```html
<input type="number" name="duration_secs" min="0" placeholder="3600 (auto for YouTube)">
```

Changes:
- Remove `required` — allows the field to be submitted blank
- Change `min="1"` to `min="0"` — allows 0 as a valid submitted value
- Update `placeholder` to hint that YouTube URLs auto-fetch

---

- [ ] **Step 2: Build to verify**

```bash
cd /Users/kunwu/Workspace/playground/MyTV && cargo build
```

Expected: compiles without errors.

---

- [ ] **Step 3: Commit**

```bash
git add templates/admin/channel_detail.html
git commit -m "feat: make duration optional for YouTube URLs in playlist form"
```
