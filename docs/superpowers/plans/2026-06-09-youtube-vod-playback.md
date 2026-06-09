# YouTube VOD Playback Implementation Plan

> **For agentic workers:** REQUIRED SUB-SKILL: Use superpowers:subagent-driven-development (recommended) or superpowers:executing-plans to implement this plan task-by-task. Steps use checkbox (`- [ ]`) syntax for tracking.

**Goal:** Make YouTube VOD URLs play with audio in MyTV by fixing the yt-dlp format selection and adding a direct-MP4 playback path in the player.

**Architecture:** Two independent fixes. (1) `resolver.rs` adds `-f "b[ext=mp4]/b"` to yt-dlp so it returns a single combined stream instead of separate video+audio lines. (2) `base.html` adds an `isHls` guard and a new direct-MP4 `else` branch in `_loadSource` so URLs that are neither `.mpd` nor `.m3u8` play via `video.src` instead of being fed to hls.js. All external URLs are already wrapped through `/stream-proxy` by `proxyUrl()` before the branch check, so seeking via Range requests works for direct MP4 too.

**Tech Stack:** Rust/Axum, yt-dlp CLI, vanilla JS (hls.js 1.5, dash.js 4.7)

**Spec:** `docs/superpowers/specs/2026-06-09-youtube-vod-playback-design.md`

---

## Files

- Modify: `src/media/resolver.rs` — add format flag to yt-dlp invocation, add unit test
- Modify: `templates/base.html` — add `isHls` variable + direct-MP4 else branch in `_loadSource`

---

### Task 1: Verify live YouTube stream still resolves correctly with the format flag

The spec flags this as a required check before shipping. Run yt-dlp manually with the new flag against a known live YouTube channel to confirm it still returns an HLS manifest.

**Files:** none

- [ ] **Step 1.1: Run yt-dlp with the new flag against a live YouTube stream**

```bash
yt-dlp -g --no-playlist -f "b[ext=mp4]/b" -- "https://www.youtube.com/watch?v=jfKfPfyJRdk"
```

Expected: a single URL containing `.m3u8` (HLS manifest). If the output contains `.m3u8`, live streams are unaffected and the flag is safe to apply globally. Proceed to Task 2.

- [ ] **Step 1.2: Handle failure (only if Step 1.1 does NOT return an HLS URL)**

If the flag breaks live resolution, the format flag must be scoped to VOD-only. In that case, modify `resolver.rs` in Task 2 to pass `-f "b[ext=mp4]/b"` only when the URL does not match a known live-stream pattern:

```rust
// Only apply format flag for non-live URLs
let is_live = url.contains("/live") || url.contains("twitch.tv");
let mut args = vec!["-g", "--no-playlist"];
if !is_live {
    args.extend_from_slice(&["-f", "b[ext=mp4]/b"]);
}
args.extend_from_slice(&["--", url]);
Command::new("yt-dlp").args(&args).output()
```

Skip this step if Step 1.1 passed.

---

### Task 2: Add format flag to `resolver.rs` + unit test

**Files:**
- Modify: `src/media/resolver.rs`

- [ ] **Step 2.1: Write a failing test asserting single-line output**

Add this test inside the `#[cfg(test)]` block at the bottom of `src/media/resolver.rs`, after the existing `test_fetch_duration_returns_seconds` test:

```rust
#[tokio::test]
#[ignore = "requires yt-dlp installed and network access — run manually"]
async fn test_resolve_youtube_vod_returns_single_line_mp4_url() {
    let url = "https://www.youtube.com/watch?v=dQw4w9WgXcQ";
    let result = resolve_url(url).await;
    assert!(result.is_ok(), "expected resolved URL, got: {:?}", result);
    let resolved = result.unwrap();
    assert!(
        !resolved.contains('\n'),
        "expected single-line URL (no separate audio stream), got multiple lines: {}",
        resolved
    );
    assert!(
        resolved.contains("mime=video%2Fmp4") || resolved.contains("video/mp4"),
        "expected video/mp4 URL, got: {}",
        resolved
    );
}
```

- [ ] **Step 2.2: Run the test to confirm it fails before the fix**

```bash
cargo test test_resolve_youtube_vod_returns_single_line_mp4_url -- --ignored --nocapture 2>&1
```

Expected: FAIL — the current code returns two lines (video + audio), so `resolved.contains('\n')` is true.

- [ ] **Step 2.3: Add the format flag to the yt-dlp invocation**

In `src/media/resolver.rs`, change line 23:

```rust
// Before:
            .args(["-g", "--no-playlist", "--", url])
// After:
            .args(["-g", "--no-playlist", "-f", "b[ext=mp4]/b", "--", url])
```

Full updated `resolve_url` function for reference:

```rust
pub async fn resolve_url(url: &str) -> Result<String> {
    if !url.starts_with("http://") && !url.starts_with("https://") {
        bail!("invalid URL scheme: {}", url);
    }
    if !needs_resolution(url) {
        return Ok(url.to_string());
    }
    let output = tokio::time::timeout(
        Duration::from_secs(30),
        Command::new("yt-dlp")
            .args(["-g", "--no-playlist", "-f", "b[ext=mp4]/b", "--", url])
            .output(),
    )
    .await
    .map_err(|_| anyhow::anyhow!("yt-dlp timed out after 30s for {}", url))??;

    if !output.status.success() {
        bail!(
            "yt-dlp failed for {}: {}",
            url,
            String::from_utf8_lossy(&output.stderr).trim()
        );
    }
    let resolved = String::from_utf8_lossy(&output.stdout).into_owned();
    let first_line = resolved.lines().next().unwrap_or("").trim().to_string();
    if first_line.is_empty() {
        bail!("yt-dlp returned empty output for {}", url);
    }
    Ok(first_line)
}
```

- [ ] **Step 2.4: Run the test to confirm it passes**

```bash
cargo test test_resolve_youtube_vod_returns_single_line_mp4_url -- --ignored --nocapture 2>&1
```

Expected: PASS — single-line URL with `video/mp4`.

- [ ] **Step 2.5: Run the full unit test suite to check for regressions**

```bash
cargo test 2>&1
```

Expected: all tests pass (the two `#[ignore]` network tests are skipped by default).

- [ ] **Step 2.6: Commit**

```bash
git add src/media/resolver.rs
git commit -m "fix: add -f b[ext=mp4]/b to yt-dlp to resolve single combined stream"
```

---

### Task 3: Add direct-MP4 branch to `_loadSource` in `base.html`

**Files:**
- Modify: `templates/base.html`

The `_loadSource` function lives at line 226. `proxyUrl(url)` is called on line 230 before the branch — this stays unchanged. The new `isHls` variable is added on line 230 (after `isDash`), and the `else` block (lines 259–295) is replaced with two branches: `else if (isHls)` for the existing hls.js path, and `else` for direct MP4.

- [ ] **Step 3.1: Replace the `else` block in `_loadSource`**

Find and replace this exact block in `templates/base.html`:

```javascript
        var isDash = url.indexOf('.mpd') >= 0;
        url = proxyUrl(url);

        if (isDash) {
          if (hls) { hls.stopLoad(); hls.detachMedia(); }
          if (dash) { dash.reset(); dash = null; }
          dash = dashjs.MediaPlayer().create();
          dash.on(dashjs.MediaPlayer.events.ERROR, function(e) {
            if (dashErrorFired || !currentChannelId) return;
            dashErrorFired = true;
            if (typeof debugLog === 'function') debugLog('warn', 'DASH error: ' + (e && e.error ? e.error.code || e.error : 'unknown') + ', trying next source');
            var nextUrl = '/channel/' + currentChannelId + '/next';
            if (currentUrl) nextUrl += '?failed_url=' + encodeURIComponent(currentUrl);
            fetch(nextUrl)
              .then(function(r) { if (!r.ok) { showPlayerError(); return null; } return r.json(); })
              .then(function(d) { if (d && d.url) _loadSource(d.url, d.start_offset_secs); })
              .catch(function(err) {
                if (typeof debugLog === 'function') debugLog('error', 'DASH failover: ' + err);
                showPlayerError();
              });
          });
          if (offset > 0) {
            dash.on(dashjs.MediaPlayer.events.MANIFEST_LOADED, function onManifest() {
              dash.off(dashjs.MediaPlayer.events.MANIFEST_LOADED, onManifest);
              video.currentTime = offset;
              video.play().catch(function(){});
            });
          }
          // autoPlay=true: dash.js calls video.play() internally after manifest load
          dash.initialize(video, url, true);
        } else {
          if (dash) { dash.reset(); dash = null; }
          if (hls) hls.attachMedia(video);
          if (hls) {
            hls.loadSource(url);
            hls.once(Hls.Events.MANIFEST_PARSED, function() {
              if (offset > 0) video.currentTime = offset;
              video.play().catch(function(){});
            });
          } else if (video && video.canPlayType('application/vnd.apple.mpegurl')) {
            video.onerror = function() {
              var nextUrl = '/channel/' + currentChannelId + '/next';
              if (currentUrl) nextUrl += '?failed_url=' + encodeURIComponent(currentUrl);
              fetch(nextUrl)
                .then(function(r) {
                  if (!r.ok) { showPlayerError(); return; }
                  return r.json();
                })
                .then(function(d) { if (d && d.url) _loadSource(d.url, d.start_offset_secs); })
                .catch(function(err) {
                  if (typeof debugLog === 'function') debugLog('error', 'native HLS: ' + err);
                  console.error('native hls error:', err);
                  showPlayerError();
                });
            };
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
        }
```

Replace with:

```javascript
        var isDash = url.indexOf('.mpd') >= 0;
        var isHls  = url.indexOf('.m3u8') >= 0;
        url = proxyUrl(url);

        if (isDash) {
          if (hls) { hls.stopLoad(); hls.detachMedia(); }
          if (dash) { dash.reset(); dash = null; }
          dash = dashjs.MediaPlayer().create();
          dash.on(dashjs.MediaPlayer.events.ERROR, function(e) {
            if (dashErrorFired || !currentChannelId) return;
            dashErrorFired = true;
            if (typeof debugLog === 'function') debugLog('warn', 'DASH error: ' + (e && e.error ? e.error.code || e.error : 'unknown') + ', trying next source');
            var nextUrl = '/channel/' + currentChannelId + '/next';
            if (currentUrl) nextUrl += '?failed_url=' + encodeURIComponent(currentUrl);
            fetch(nextUrl)
              .then(function(r) { if (!r.ok) { showPlayerError(); return null; } return r.json(); })
              .then(function(d) { if (d && d.url) _loadSource(d.url, d.start_offset_secs); })
              .catch(function(err) {
                if (typeof debugLog === 'function') debugLog('error', 'DASH failover: ' + err);
                showPlayerError();
              });
          });
          if (offset > 0) {
            dash.on(dashjs.MediaPlayer.events.MANIFEST_LOADED, function onManifest() {
              dash.off(dashjs.MediaPlayer.events.MANIFEST_LOADED, onManifest);
              video.currentTime = offset;
              video.play().catch(function(){});
            });
          }
          // autoPlay=true: dash.js calls video.play() internally after manifest load
          dash.initialize(video, url, true);
        } else if (isHls) {
          if (dash) { dash.reset(); dash = null; }
          if (hls) hls.attachMedia(video);
          if (hls) {
            hls.loadSource(url);
            hls.once(Hls.Events.MANIFEST_PARSED, function() {
              if (offset > 0) video.currentTime = offset;
              video.play().catch(function(){});
            });
          } else if (video && video.canPlayType('application/vnd.apple.mpegurl')) {
            video.onerror = function() {
              var nextUrl = '/channel/' + currentChannelId + '/next';
              if (currentUrl) nextUrl += '?failed_url=' + encodeURIComponent(currentUrl);
              fetch(nextUrl)
                .then(function(r) {
                  if (!r.ok) { showPlayerError(); return; }
                  return r.json();
                })
                .then(function(d) { if (d && d.url) _loadSource(d.url, d.start_offset_secs); })
                .catch(function(err) {
                  if (typeof debugLog === 'function') debugLog('error', 'native HLS: ' + err);
                  console.error('native hls error:', err);
                  showPlayerError();
                });
            };
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
        } else {
          // Direct MP4 (e.g. YouTube VOD resolved via yt-dlp)
          if (hls) { hls.stopLoad(); hls.detachMedia(); }
          if (dash) { dash.reset(); dash = null; }
          video.onerror = function() { showPlayerError(); };
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

- [ ] **Step 3.2: Run the full test suite**

```bash
cargo test 2>&1
```

Expected: all tests pass. (Templates are not compiled by `cargo test` but this confirms the Rust side is unaffected.)

- [ ] **Step 3.3: Run `cargo fmt` and `cargo clippy`**

```bash
cargo fmt && cargo clippy -- -D warnings 2>&1
```

Expected: no warnings or errors.

- [ ] **Step 3.4: Commit**

```bash
git add templates/base.html
git commit -m "fix: add direct-MP4 playback path in _loadSource for YouTube VOD"
```

---

### Task 4: Manual end-to-end verification

**Files:** none

- [ ] **Step 4.1: Start the server**

```bash
cargo run 2>&1
```

Server starts on `http://localhost:3000`.

- [ ] **Step 4.2: Add a YouTube VOD item**

Navigate to `http://localhost:3000/admin/discover`. Click **Manual URL**. Paste:

```
https://www.youtube.com/watch?v=dQw4w9WgXcQ
```

Click **Resolve**. Expected: a resolved `googlevideo.com` URL displayed. Note the duration (~213s).

- [ ] **Step 4.3: Add to a VOD channel**

Select an existing `vod_loop` channel (or create one: Admin → Channels → New → type `vod_loop`). Click **Add to Channel**. Expected: item appears in the channel detail with title and duration > 0.

- [ ] **Step 4.4: Play via the guide**

Navigate to `http://localhost:3000/guide`. Find the VOD channel row. Click to tune. Expected: video plays **with audio** in the player panel.

- [ ] **Step 4.5: Verify live YouTube stream still works (regression)**

Add a known live YouTube channel URL (e.g. `https://www.youtube.com/watch?v=jfKfPfyJRdk`) as a live source on a live channel. Tune to it via the guide. Expected: HLS stream loads and plays normally — the format flag should not have broken live resolution (confirmed in Task 1).
