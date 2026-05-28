# Channel Unavailable Message Implementation Plan

> **For agentic workers:** REQUIRED SUB-SKILL: Use superpowers:subagent-driven-development (recommended) or superpowers:executing-plans to implement this plan task-by-task. Steps use checkbox (`- [ ]`) syntax for tracking.

**Goal:** Show a "Channel unavailable" message in the player panel when all sources fail (server returns 503), replacing the current silent failure.

**Architecture:** Two template files change — `guide.html` gains a hidden error div inside `#player-panel`, and `base.html` gains two helper functions (`showPlayerError` / `hidePlayerError`) that are called from the three existing JS error paths. No new routes, no Rust changes.

**Tech Stack:** Vanilla JS, HTML, Askama 0.12 (templates compiled into binary at `cargo build`)

---

## File Map

| File | Change |
|---|---|
| `templates/guide.html` | Add `<div id="player-error">` inside `#player-panel` |
| `templates/base.html` | Add `showPlayerError()` / `hidePlayerError()` helpers; update 3 error paths |

---

### Task 1: Add error div to guide.html

**Files:**
- Modify: `templates/guide.html`

---

- [ ] **Step 1: Replace the player-panel block**

`templates/guide.html` currently reads:

```html
{% extends "base.html" %}
{% block content %}
<div id="player-panel">
  <video id="video" controls></video>
</div>

<div id="epg-content">
  {% include "partials/epg_content.html" %}
</div>
{% endblock %}
```

Replace it with:

```html
{% extends "base.html" %}
{% block content %}
<div id="player-panel">
  <video id="video" controls></video>
  <div id="player-error" style="display:none;padding:32px;text-align:center;color:#e94560;font-size:1rem">
    Channel unavailable
  </div>
</div>

<div id="epg-content">
  {% include "partials/epg_content.html" %}
</div>
{% endblock %}
```

The error div is hidden by default (`display:none`). It shares the `#player-panel` space with the video element.

---

- [ ] **Step 2: Build to verify template compiles**

```bash
cd /Users/kunwu/Workspace/playground/MyTV && cargo build
```

Expected: compiles without errors.

---

- [ ] **Step 3: Commit**

```bash
git add templates/guide.html
git commit -m "feat: add player-error div to player panel"
```

---

### Task 2: Update player JS in base.html

**Files:**
- Modify: `templates/base.html`

The player script block in `base.html` currently looks like this (lines 78–152). You will make four targeted edits within it.

---

- [ ] **Step 1: Add showPlayerError / hidePlayerError helpers**

Locate the `_loadSource` function in `base.html` (around line 102). It ends with a closing `}` before the `tune` function. Add the two helpers immediately after `_loadSource`'s closing brace:

The current code around line 121 ends `_loadSource` and starts `tune`:
```javascript
      }

      function tune(channelId) {
```

Insert the two helpers between them:
```javascript
      }

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

      function tune(channelId) {
```

---

- [ ] **Step 2: Update tune() — clear error on new tune, show on failure**

The current `tune` function (around line 123):
```javascript
      function tune(channelId) {
        currentChannelId = channelId;
        document.getElementById('player-panel').style.display = 'block';
        fetch('/channel/' + channelId + '/tune')
          .then(function(r) {
            if (!r.ok) throw new Error('tune failed: ' + r.status);
            return r.json();
          })
          .then(function(d) { _loadSource(d.url, d.start_offset_secs); })
          .catch(function(err) { console.error('tune error:', err); });
      }
```

Replace it with:
```javascript
      function tune(channelId) {
        currentChannelId = channelId;
        hidePlayerError();
        document.getElementById('player-panel').style.display = 'block';
        fetch('/channel/' + channelId + '/tune')
          .then(function(r) {
            if (!r.ok) throw new Error('tune failed: ' + r.status);
            return r.json();
          })
          .then(function(d) { _loadSource(d.url, d.start_offset_secs); })
          .catch(function(err) { console.error('tune error:', err); showPlayerError(); });
      }
```

Changes: `hidePlayerError()` at the top; `showPlayerError()` appended to the `.catch`.

---

- [ ] **Step 3: Update HLS fatal error handler — check r.ok before parsing JSON**

The current HLS error handler (around line 91):
```javascript
        hls.on(Hls.Events.ERROR, function(event, data) {
          if (data.fatal && currentChannelId) {
            console.warn('hls fatal error, trying next source:', data.type, data.details);
            fetch('/channel/' + currentChannelId + '/next')
              .then(function(r) { return r.json(); })
              .then(function(d) { if (d.url) _loadSource(d.url, d.start_offset_secs); })
              .catch(function(err) { console.error('failover error:', err); });
          }
        });
```

Replace it with:
```javascript
        hls.on(Hls.Events.ERROR, function(event, data) {
          if (data.fatal && currentChannelId) {
            console.warn('hls fatal error, trying next source:', data.type, data.details);
            fetch('/channel/' + currentChannelId + '/next')
              .then(function(r) {
                if (!r.ok) { showPlayerError(); return; }
                return r.json();
              })
              .then(function(d) { if (d && d.url) _loadSource(d.url, d.start_offset_secs); })
              .catch(function(err) { console.error('failover error:', err); showPlayerError(); });
          }
        });
```

Changes: added `r.ok` guard that calls `showPlayerError()` and returns early; added null-guard `d &&` before `d.url` since the previous `.then` may return `undefined` after the early return; added `showPlayerError()` to `.catch`.

---

- [ ] **Step 4: Update video.ended handler — show error on next failure**

The current `video.ended` handler (around line 136):
```javascript
      if (video) {
        video.addEventListener('ended', function() {
          if (!currentChannelId) return;
          fetch('/channel/' + currentChannelId + '/next')
            .then(function(r) {
              if (!r.ok) throw new Error('next failed: ' + r.status);
              return r.json();
            })
            .then(function(d) {
              if (d.url) _loadSource(d.url, d.start_offset_secs);
            })
            .catch(function(err) { console.error('next error:', err); });
        });
      }
```

Replace it with:
```javascript
      if (video) {
        video.addEventListener('ended', function() {
          if (!currentChannelId) return;
          fetch('/channel/' + currentChannelId + '/next')
            .then(function(r) {
              if (!r.ok) throw new Error('next failed: ' + r.status);
              return r.json();
            })
            .then(function(d) {
              if (d.url) _loadSource(d.url, d.start_offset_secs);
            })
            .catch(function(err) { console.error('next error:', err); showPlayerError(); });
        });
      }
```

Change: `showPlayerError()` appended to the `.catch`.

---

- [ ] **Step 5: Build to verify**

```bash
cd /Users/kunwu/Workspace/playground/MyTV && cargo build
```

Expected: compiles without errors.

---

- [ ] **Step 6: Manual smoke test**

1. Run the server: `cargo run`
2. Navigate to `http://localhost:3000/guide`
3. Click a channel that has no active sources (or disable all sources in admin).
4. Confirm: the player panel shows "Channel unavailable" in red.
5. Click a working channel.
6. Confirm: the error message disappears and the video plays.

---

- [ ] **Step 7: Commit**

```bash
git add templates/base.html
git commit -m "feat: show channel unavailable message when all sources fail"
```
