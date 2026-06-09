# YouTube VOD Playback — Design

## Goal

Make YouTube VOD URLs (e.g. `youtube.com/watch?v=...`) play with audio in MyTV's player.

## Root Cause

Two independent bugs, both required to fix:

**Bug 1 — resolver:** `yt-dlp -g` with no format flag returns two lines for YouTube VOD: a video-only stream and an audio-only stream. `resolver.rs` takes the first line only → silent video.

**Bug 2 — player:** `_loadSource` in `base.html` has two paths — DASH (`.mpd`) and HLS (everything else via hls.js). There is no path for direct MP4. A `googlevideo.com` URL fed to hls.js fails with a fatal manifest-parse error, which triggers "try next source" and eventually shows the player error state.

## Changes

### 1. `src/media/resolver.rs`

Add `-f "b[ext=mp4]/b"` to the `yt-dlp -g` call:

```rust
Command::new("yt-dlp")
    .args(["-g", "--no-playlist", "-f", "b[ext=mp4]/b", "--", url])
```

- `b[ext=mp4]` — best single-file combined MP4 (video+audio in one stream)
- `/b` — fallback to best single-file in any container if no combined MP4 exists
- YouTube always has combined streams available up to ~720p; this selects the best one

This change affects all URLs that go through `resolve_url` (YouTube and Twitch). HLS/IPTV URLs are returned unchanged (they bypass yt-dlp via the `needs_resolution` guard).

### 2. `templates/base.html`

Add a third branch in `_loadSource` for direct MP4 (URLs containing neither `.mpd` nor `.m3u8`):

```javascript
var isDash = url.indexOf('.mpd') >= 0;
var isHls  = url.indexOf('.m3u8') >= 0;

if (isDash) {
  // existing dash.js path — unchanged
} else if (isHls) {
  // existing hls.js path — unchanged
} else {
  // direct MP4 — native <video> element
  if (hls) { hls.stopLoad(); hls.detachMedia(); }
  if (dash) { dash.reset(); dash = null; }
  video.src = url;
  if (offset > 0) {
    video.addEventListener('loadedmetadata', function onMeta() {
      video.removeEventListener('loadedmetadata', onMeta);
      video.currentTime = offset;
      video.play().catch(function(){});
    });
  } else {
    video.play().catch(function(){});
  }
}
```

`<video src="url">` loads cross-origin media without CORS headers — it uses the browser's opaque response model. No proxying is needed for YouTube CDN URLs.

## Data Flow

```
User clicks channel in guide
  → tune(channelId) → GET /channel/:id/tune
  → resolve_url("https://youtube.com/watch?v=...") via yt-dlp -f b[ext=mp4]/b -g
  → returns single https://googlevideo.com/videoplayback?... URL
  → _loadSource(url): not .mpd, not .m3u8 → video.src = url
  → browser loads MP4 directly from YouTube CDN
  → video plays with audio
```

## URL Expiry

YouTube CDN URLs carry an `expire=` timestamp (~6 hours). Since `resolve_url` is called at tune time (not stored in the DB), the URL is always fresh. The stored playlist item URL remains `youtube.com/watch?v=...` permanently.

## Error Handling

- yt-dlp fails (network error, video unavailable, private video) → `resolve_url` returns `Err` → tune endpoint returns HTTP 503 → player shows "Channel unavailable"
- Video fails to load after `video.src` is set → add `video.onerror` handler in the direct-MP4 branch that calls `showPlayerError()`, matching the pattern used in the existing native HLS path

## What This Does Not Change

- Live YouTube streams — need explicit verification. `b[ext=mp4]` may not match an HLS live manifest; the `/b` fallback should return the best available format (HLS), but this must be confirmed by running the existing `#[ignore]` test `test_resolve_youtube_live_returns_hls_url` against a live channel with the format flag applied. If the flag breaks live resolution, scope it to VOD-only via a separate yt-dlp invocation path.
- HLS/IPTV sources — `needs_resolution()` returns false; they bypass yt-dlp entirely
- DASH sources — `.mpd` detection in the player is unchanged
- Bilibili — out of scope; tracked separately

## Testing

- Unit: update `test_resolve_youtube_live_returns_hls_url` to also verify the resolved URL is a single line (no newline)
- Integration: add `#[ignore]` test `test_resolve_youtube_vod_returns_single_mp4_url` — resolves `dQw4w9WgXcQ`, asserts single-line HTTPS URL with no `\n`
- Manual: add `https://www.youtube.com/watch?v=dQw4w9WgXcQ` as a VOD playlist item, play via guide, confirm audio present
