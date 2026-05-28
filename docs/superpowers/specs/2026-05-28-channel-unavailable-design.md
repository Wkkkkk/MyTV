# Channel Unavailable Message — Design Spec
_2026-05-28_

## Goal

When all sources for a channel fail (server returns 503), show a visible "Channel unavailable" message in the player panel instead of silently leaving the video blank.

## Architecture

No new routes. The server already returns `503 SERVICE_UNAVAILABLE` when all sources are exhausted. The fix is entirely client-side: catch non-OK responses in the three existing JS error paths and surface a message in the player panel.

**Files changed:**
- `templates/guide.html` — add `<div id="player-error">` inside `#player-panel`
- `templates/base.html` — add `showPlayerError()` / `hidePlayerError()` helpers; update 3 error paths

---

## HTML Change

Inside `#player-panel` in `guide.html`, add an error div alongside the existing `<video>`:

```html
<div id="player-panel">
  <video id="video" controls></video>
  <div id="player-error" style="display:none;padding:32px;text-align:center;color:#e94560;font-size:1rem">
    Channel unavailable
  </div>
</div>
```

The error div is hidden by default. When shown, the video is hidden so they don't both occupy space.

---

## JavaScript Changes

Two helper functions added to the player script block in `base.html`:

```js
function showPlayerError() {
  if (video) video.style.display = 'none';
  var el = document.getElementById('player-error');
  if (el) el.style.display = 'block';
}

function hidePlayerError() {
  if (video) video.style.display = '';
  var el = document.getElementById('player-error');
  if (el) el.style.display = 'none';
}
```

### Three error paths updated

**1. `tune()` — initial tune call fails**

At the start of `tune()`, call `hidePlayerError()` (clears any previous error when the user switches channels).

In the `.catch()` at the end of `tune()`, call `showPlayerError()` instead of just `console.error`.

**2. HLS fatal error failover — `fetch('/next')` response not OK**

The current HLS error handler calls `fetch('/next').then(r => r.json())` without checking `r.ok`. Add a check: if `!r.ok`, call `showPlayerError()` and return.

**3. `video.ended` next-item call fails**

In the `video.ended` handler's `.catch()`, call `showPlayerError()` instead of just `console.error`.

---

## Error Handling

| Condition | Behaviour |
|---|---|
| `tune()` returns non-OK (e.g. 503) | `showPlayerError()` — video hidden, error message shown |
| HLS fatal error, `/next` returns non-OK | `showPlayerError()` |
| `video.ended`, `/next` returns non-OK | `showPlayerError()` |
| User clicks a channel while error is shown | `hidePlayerError()` called at start of `tune()`, clears the error |
| Channel is working normally | Error div stays `display:none` |

---

## Testing

No unit tests needed — this is purely a JS/HTML change. Verified by:
1. `cargo build` — confirms templates compile
2. Manual test: tune to a channel with no active sources → error message appears
3. Manual test: click a working channel → error clears and video plays
