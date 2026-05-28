# Auto-Fetch Duration — Design Spec
_2026-05-28_

## Goal

When an admin adds a VOD playlist item with a YouTube URL and leaves the Duration field blank, automatically fetch the video duration via yt-dlp instead of requiring manual entry.

## Architecture

No new routes. The `playlist_item_create` handler in `src/routes/admin.rs` is extended to call `resolver::fetch_duration_secs(url)` when `duration_secs` is zero and the URL matches a YouTube/yt-dlp-resolvable pattern. The duration field in the admin form becomes optional for YouTube URLs.

**Files changed:**
- `src/routes/admin.rs` — update `playlist_item_create` handler
- `templates/admin/channel_detail.html` — update Duration input (remove `required`, update placeholder)

---

## Handler Logic

In `playlist_item_create`, replace the current early-return on `duration_secs <= 0` with:

```
let mut duration_secs = parse(form.duration_secs) or 0
if duration_secs <= 0:
    if resolver::needs_resolution(url):
        duration_secs = resolver::fetch_duration_secs(url).await
            → on error: tracing::warn + return 422
    else:
        return 422          // unchanged: non-YouTube still requires manual entry
```

If the admin provides a non-zero duration, it is used as-is — no fetch attempted.

`resolver::needs_resolution(url)` is the existing function in `src/resolver.rs` that returns `true` for YouTube, youtu.be, and Twitch URLs.

---

## Template Change

In `templates/admin/channel_detail.html`, the Duration input currently reads:

```html
<input type="number" name="duration_secs" required min="1" placeholder="3600">
```

Change it to:

```html
<input type="number" name="duration_secs" min="0" placeholder="3600 (auto for YouTube)">
```

Changes: remove `required`, change `min="1"` to `min="0"`, update placeholder. When left blank the browser submits an empty string, which `parse().unwrap_or(0)` turns into `0`, triggering the auto-fetch path.

---

## Error Handling

| Condition | Behaviour |
|---|---|
| Non-YouTube URL, duration blank/0 | 422 (unchanged) |
| YouTube URL, duration blank/0, fetch succeeds | Item created with fetched duration |
| YouTube URL, duration blank/0, `fetch_duration_secs` returns error | `tracing::warn` + 422 |
| YouTube URL, duration blank/0, yt-dlp times out (30s) | Error propagates from resolver → 422 |
| Any URL, duration > 0 provided | Used as-is, no fetch |

---

## Testing

`resolver::fetch_duration_secs` is already covered by an `#[ignore]` integration test (requires yt-dlp + network). The new handler logic is thin; no new unit tests needed. Verified by:
1. `cargo test` — all existing tests pass
2. `cargo build` — template change compiles
3. Manual test: add a YouTube URL with blank duration → item created with correct duration
