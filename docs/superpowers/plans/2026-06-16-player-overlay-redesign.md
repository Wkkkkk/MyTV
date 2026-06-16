# Player Overlay Redesign Implementation Plan

> **For agentic workers:** REQUIRED SUB-SKILL: Use superpowers:subagent-driven-development (recommended) or superpowers:executing-plans to implement this plan task-by-task. Steps use checkbox (`- [ ]`) syntax for tracking.

**Goal:** Replace the keyboard-only player overlay with a gesture-forward, channel-type-aware player (Direction C) where touch and keyboard are co-equal, and on-demand channels get first-class item + seek navigation.

**Architecture:** Frontend-only change in two template files. The native `<video controls>` bar is removed and replaced with a custom transport (play/pause, scrubber, time, prev/next, playlist, fullscreen). A single `navigate(dir)` function unifies "next item" and "next channel" into one continuous-feed axis driven by `↑`/`↓`, swipe up/down, and the `⏮`/`⏭` buttons. A touch-gesture layer (tap / double-tap-edge / vertical swipe) is bound to `#player-panel`. Fullscreen uses the native Fullscreen API on the panel so custom chrome stays visible.

**Tech Stack:** Askama HTML templates, vanilla ES5-style JS (matching the existing inline script), hls.js / dash.js (unchanged). No new dependencies. No backend/API/DB changes.

---

## Testing approach (read first)

This project has **no JavaScript test harness** — automated tests are Rust (`cargo test`: integration via `tower::oneshot` + the prod e2e suite). Adding a JS runner is out of scope (the spec forbids new dependencies). Therefore:

- **Automated gate, every task:** `cargo fmt && cargo clippy -- -D warnings && cargo test`. Askama compiles templates at build time, so a broken template *fails the build* — this is the safety net that the markup still compiles and the endpoints are untouched.
- **Manual/visual gate, every task:** run `cargo run` (server on `:3000`), open `http://localhost:3000`, and walk the explicit verification steps in the task. Use seed-equivalent channels: a live channel, a `vod_loop` channel, and a `vod_on_demand` channel.

Each task lists the exact manual checks. "Verify" steps below mean: perform the listed interaction and confirm the described result in the browser.

---

## File structure

- **`templates/guide.html`** — owns the player markup (`#player-panel` and children, including `<video>` and `#player-toolbar`). All markup changes land here.
- **`templates/base.html`** — owns (a) the inline `<style>` block with all `#player-*` CSS (~L25–74) and (b) the inline player JS (~L183–866). All CSS and JS changes land here.

No files are created. No other files change.

---

## Task 1: New chrome markup + styles, remove native controls

**Files:**
- Modify: `templates/guide.html` (the `#player-panel` block, ~L8–31)
- Modify: `templates/base.html` (inline `<style>`, ~L25–74)

- [ ] **Step 1: Replace the toolbar markup and the `<video>` element in `templates/guide.html`**

Replace the current block (from `<div id="player-toolbar">` through `<video id="video" controls></video>`, lines ~9–26) with:

```html
  <div id="player-toolbar">
    <button type="button" class="ov-btn" id="ov-close" title="Close" aria-label="Close player">✕</button>
    <span id="ov-title"></span>
    <span class="ov-spacer"></span>
    <button type="button" class="ov-btn" id="ov-help" title="Controls" aria-label="Controls help">?</button>
  </div>
  <button type="button" id="ov-center-play" aria-label="Play / pause">▶</button>
  <div id="player-help" hidden>
    <strong>Controls</strong>
    <div>Tap center / Space — play / pause</div>
    <div>Swipe ↑↓ / ↑ ↓ keys — prev / next (item, then channel)</div>
    <div>Double-tap edge / ← → — seek 10s</div>
    <div>⛶ / F — fullscreen · ✕ / Esc — close</div>
  </div>
  <div id="player-playlist" hidden></div>
  <div id="player-buffering"><span class="spinner-lg"></span> Loading…</div>
  <video id="video" playsinline></video>
  <div id="player-transport">
    <button type="button" class="ov-btn" id="ov-prev-item" title="Previous" aria-label="Previous">⏮</button>
    <button type="button" class="ov-btn" id="ov-playpause" title="Play / pause" aria-label="Play / pause">⏸</button>
    <input type="range" id="ov-seek" min="0" max="1000" value="0" step="1" aria-label="Seek">
    <span id="ov-time" class="ov-time">0:00 / 0:00</span>
    <button type="button" class="ov-btn" id="ov-next-item" title="Next" aria-label="Next">⏭</button>
    <button type="button" class="ov-btn" id="ov-playlist" title="Playlist" aria-label="Playlist" hidden>☰</button>
    <button type="button" class="ov-btn" id="ov-fullscreen" title="Fullscreen" aria-label="Fullscreen">⛶</button>
  </div>
```

Notes: the old `#player-help` and `#player-playlist` and `#player-buffering` lines are folded into the block above (so they are not duplicated — delete the originals). The `controls` attribute is removed from `<video>`; `playsinline` is added so iOS plays inline rather than forcing native fullscreen. The `#player-error`, `#player-ended`, `#player-toast`, `#player-waiting` lines stay exactly as they are, immediately after.

- [ ] **Step 2: Replace the player CSS in `templates/base.html`**

In the inline `<style>`, replace the `#player-toolbar` / `.ov-btn` / `.ov-spacer` / `#player-help` rules (~L35–50) with the rules below, and add the new transport/center-play/fullscreen rules. Keep `#player-playlist`, `.pl-*`, `#player-buffering`, `.spinner-lg` rules unchanged.

```css
    /* chrome reveal is gated on .show-controls (set by JS) */
    #player-toolbar{position:absolute;top:0;left:0;right:0;z-index:6;
      display:flex;align-items:center;gap:8px;padding:10px 12px;
      background:linear-gradient(#000,transparent);
      opacity:0;transition:opacity 0.2s;pointer-events:none}
    #player-panel.show-controls #player-toolbar{opacity:1;pointer-events:auto}
    #ov-title{color:#fff;font-size:0.85rem;font-weight:600;
      white-space:nowrap;overflow:hidden;text-overflow:ellipsis}
    .ov-btn{min-width:40px;height:40px;display:flex;align-items:center;justify-content:center;
      background:rgba(20,20,20,0.7);color:#eee;border:1px solid var(--border-strong);
      border-radius:4px;cursor:pointer;font-size:1rem;font-family:inherit}
    .ov-btn:hover{background:var(--accent);border-color:var(--accent);color:#fff}
    .ov-btn:focus-visible{outline:2px solid var(--accent);outline-offset:1px}
    .ov-btn[hidden]{display:none}
    .ov-spacer{flex:1}
    #ov-center-play{position:absolute;top:50%;left:50%;transform:translate(-50%,-50%);
      z-index:6;width:64px;height:64px;border-radius:50%;border:none;cursor:pointer;
      background:rgba(255,255,255,0.92);color:#111;font-size:1.6rem;
      display:none;align-items:center;justify-content:center}
    #player-panel.show-controls #ov-center-play{display:flex}
    #player-transport{position:absolute;bottom:0;left:0;right:0;z-index:6;
      display:flex;align-items:center;gap:8px;padding:10px 12px;
      background:linear-gradient(transparent,#000);
      opacity:0;transition:opacity 0.2s;pointer-events:none}
    #player-panel.show-controls #player-transport{opacity:1;pointer-events:auto}
    #ov-seek{flex:1;height:4px;cursor:pointer;accent-color:var(--accent)}
    .ov-time{font-size:0.75rem;color:#ddd;font-variant-numeric:tabular-nums;flex-shrink:0}
    /* fullscreen: panel fills the screen so custom chrome stays visible */
    #player-panel:fullscreen{display:flex;align-items:center;justify-content:center;background:#000}
    #player-panel:fullscreen video{max-height:100vh;width:100%}
    #player-help{position:absolute;top:56px;right:12px;z-index:7;
      background:rgba(10,10,10,0.92);border:1px solid var(--border-strong);
      border-radius:5px;padding:10px 14px;font-size:0.8rem;color:var(--text);line-height:1.7}
    #player-help strong{display:block;margin-bottom:4px;color:var(--accent)}
```

Also update `#player-playlist`'s `bottom:100px` and `#player-toast`'s `bottom:120px` to `bottom:64px` and `bottom:80px` respectively (they previously cleared the native controls bar which no longer exists; the custom transport sits ~52px tall).

- [ ] **Step 3: Automated gate**

Run: `cargo fmt && cargo clippy -- -D warnings && cargo test`
Expected: PASS (templates compile; no Rust changed).

- [ ] **Step 4: Manual gate**

Run: `cargo run`, open `http://localhost:3000`, click a `vod_loop` channel in the guide to tune it.
Expected: the video plays (autoplay is still driven by existing JS calling `video.play()`). The native control bar is gone. Moving the mouse over the panel reveals the top bar (✕, title, ?) and the bottom transport row; they fade after ~3s. Controls are not yet wired (clicking them does nothing) — that is Task 2+.

- [ ] **Step 5: Commit**

```bash
git add templates/guide.html templates/base.html
git commit -m "feat(player): new chrome markup + styles, drop native video controls (idea #51)"
```

---

## Task 2: Wire play/pause, fullscreen, close, and chrome reveal

**Files:**
- Modify: `templates/base.html` (player JS, the toolbar wiring block ~L747–792, and the `<video>` setup)

- [ ] **Step 1: Add transport helper functions**

In `templates/base.html`, immediately after `window.tune = tune;` (~L745), add:

```js
      // ── custom transport (idea #51) ───────────────────────────
      function togglePlay() {
        if (!video) return;
        if (video.paused) { video.play().catch(function(){}); }
        else { video.pause(); }
      }

      function syncPlayPauseIcon() {
        var pp = document.getElementById('ov-playpause');
        var cp = document.getElementById('ov-center-play');
        var paused = !video || video.paused;
        if (pp) pp.textContent = paused ? '▶' : '⏸';
        if (cp) cp.textContent = paused ? '▶' : '⏸';
      }

      function closePlayer() {
        stopPlayback();
        var p = document.getElementById('player-panel');
        if (p) p.style.display = 'none';
        var pl = document.getElementById('player-playlist');
        if (pl) pl.hidden = true;
        currentChannelId = null;
        odChannelId = null; odItems = []; odIndex = -1;
      }

      function toggleFullscreen() {
        var p = document.getElementById('player-panel');
        if (!p) return;
        if (document.fullscreenElement) { document.exitFullscreen(); }
        else { p.requestFullscreen().catch(function(){}); }
      }
```

- [ ] **Step 2: Replace the toolbar wiring block**

Replace the existing wiring (the block starting `var ovPrev = document.getElementById('ov-prev');` down to the end of the `ov-playlist` click handler, ~L765–792) with:

```js
      var ovClose = document.getElementById('ov-close');
      var ovHelp = document.getElementById('ov-help');
      var ovPlayPause = document.getElementById('ov-playpause');
      var ovCenterPlay = document.getElementById('ov-center-play');
      var ovFull = document.getElementById('ov-fullscreen');
      if (ovClose) ovClose.addEventListener('click', closePlayer);
      if (ovHelp) ovHelp.addEventListener('click', function() {
        if (helpBox) helpBox.hidden = !helpBox.hidden;
      });
      if (ovPlayPause) ovPlayPause.addEventListener('click', togglePlay);
      if (ovCenterPlay) ovCenterPlay.addEventListener('click', togglePlay);
      if (ovFull) ovFull.addEventListener('click', toggleFullscreen);

      var ovPlaylist = document.getElementById('ov-playlist');
      var plBox = document.getElementById('player-playlist');
      if (ovPlaylist) ovPlaylist.addEventListener('click', function() {
        if (!plBox || odChannelId !== currentChannelId || !odItems.length) return;
        plBox.hidden = !plBox.hidden;
      });

      if (video) {
        video.addEventListener('play', syncPlayPauseIcon);
        video.addEventListener('pause', syncPlayPauseIcon);
      }
```

Note: `panel`, `helpBox`, and the `showControls()` function defined just above (~L748–763) are unchanged and reused.

- [ ] **Step 3: Automated gate**

Run: `cargo fmt && cargo clippy -- -D warnings && cargo test`
Expected: PASS.

- [ ] **Step 4: Manual gate**

`cargo run`, tune a `vod_loop` channel. Reveal chrome (mouse move). Verify:
- Center play and the transport play/pause button both toggle playback, and both icons flip ▶/⏸ in sync with the video state.
- `⛶` makes the panel fill the screen with the video and chrome still visible; clicking it again (or Esc) exits.
- `✕` stops playback and hides the panel.
- `?` toggles the controls help box.

- [ ] **Step 5: Commit**

```bash
git add templates/base.html
git commit -m "feat(player): wire play/pause, fullscreen, close, help (idea #51)"
```

---

## Task 3: Scrubber + time display, channel-type-aware control visibility

**Files:**
- Modify: `templates/base.html` (player JS)

- [ ] **Step 1: Add the time formatter and transport-visibility updater**

After the helpers from Task 2, add:

```js
      function fmtTime(secs) {
        if (!isFinite(secs) || secs < 0) secs = 0;
        secs = Math.floor(secs);
        var m = Math.floor(secs / 60), s = secs % 60;
        return m + ':' + (s < 10 ? '0' : '') + s;
      }

      // Show/hide transport pieces for the current channel type.
      // Called from renderInfoBar so it always matches the playing channel.
      function updateTransport(channel) {
        var seekable = channel && (channel.channel_type === 'vod_loop'
                                || channel.channel_type === 'vod_on_demand');
        var seek = document.getElementById('ov-seek');
        var time = document.getElementById('ov-time');
        if (seek) seek.style.display = seekable ? '' : 'none';
        if (time) time.style.display = seekable ? '' : 'none';
        // ⏮/⏭ stay visible for all types: they call navigate() (prev/next in
        // the continuous feed — item on on-demand, channel otherwise).
      }
```

- [ ] **Step 2: Call `updateTransport` from `renderInfoBar`**

In `renderInfoBar` (`base.html` ~L223), add as the last line before the closing `}` (after `bar.style.display = 'flex';`):

```js
        updateTransport(channel);
```

- [ ] **Step 3: Wire the seek bar**

In the `if (video) { ... }` block added in Task 2 Step 2 (the one with the `play`/`pause` listeners), extend it to:

```js
      if (video) {
        video.addEventListener('play', syncPlayPauseIcon);
        video.addEventListener('pause', syncPlayPauseIcon);
        var seekBar = document.getElementById('ov-seek');
        var timeEl = document.getElementById('ov-time');
        var dragging = false;
        video.addEventListener('timeupdate', function() {
          if (dragging || !seekBar) return;
          var d = video.duration;
          if (isFinite(d) && d > 0) seekBar.value = Math.round(video.currentTime / d * 1000);
          if (timeEl) timeEl.textContent = fmtTime(video.currentTime) + ' / ' + fmtTime(d);
        });
        if (seekBar) {
          seekBar.addEventListener('input', function() {
            dragging = true;
            var d = video.duration;
            if (isFinite(d) && d > 0 && timeEl) {
              timeEl.textContent = fmtTime(seekBar.value / 1000 * d) + ' / ' + fmtTime(d);
            }
          });
          seekBar.addEventListener('change', function() {
            var d = video.duration;
            if (isFinite(d) && d > 0) video.currentTime = seekBar.value / 1000 * d;
            dragging = false;
          });
        }
      }
```

Note: this replaces the smaller `if (video) { ... }` block from Task 2 Step 2 — merge them so there is only one. The existing separate `if (video) { ... timeupdate ... odSaveCursor }` block (~L812–820) stays as-is.

- [ ] **Step 4: Automated gate**

Run: `cargo fmt && cargo clippy -- -D warnings && cargo test`
Expected: PASS.

- [ ] **Step 5: Manual gate**

`cargo run`. Verify per type:
- **live** channel: no seek bar, no time display; play/pause works.
- **vod_loop** channel: seek bar tracks playback, time shows `m:ss / m:ss`; dragging the bar seeks the video and the time preview follows the handle.
- **vod_on_demand** channel: seek bar + time present and seek the current item.

- [ ] **Step 6: Commit**

```bash
git add templates/base.html
git commit -m "feat(player): scrubber + time display, type-aware transport (idea #51)"
```

---

## Task 4: Unified vertical navigation (continuous item→channel feed)

**Files:**
- Modify: `templates/base.html` (player JS)

- [ ] **Step 1: Add `odPrevAvailable` next to `odNextAvailable`**

After `odNextAvailable` (`base.html` ~L559–564), add:

```js
      // Last playable index at or before `from`, or -1 if none remain.
      function odPrevAvailable(from) {
        for (var i = Math.min(from, odItems.length - 1); i >= 0; i--) {
          if (odAvailable(odItems[i])) return i;
        }
        return -1;
      }
```

- [ ] **Step 2: Add the `odStartAtLast` flag and honor it in `odTune`**

Add a module-scoped flag near the other on-demand vars (`base.html` ~L524):

```js
      var odStartAtLast = false;  // navigate('up') overflow lands on the last item
```

In `odTune`, inside the `.then(function (items) {...})`, replace the start-selection block. The current code (~L646–662) computes `start`/`offset` from `startItemId` or the saved cursor. Add this branch as the FIRST check, before the `startItemId`/cursor logic:

```js
            var start = -1, offset = 0;
            if (odStartAtLast) {
              odStartAtLast = false;
              start = odPrevAvailable(odItems.length - 1);
              odRenderList();
              var box0 = document.getElementById('player-playlist');
              if (box0) box0.hidden = false;
              if (start < 0) { showPlayerError(); return; }
              odPlayIndex(start, 0);
              return;
            }
```

(Leave the existing `startItemId`/cursor logic after it unchanged — it already declares `var start`/`offset`; change its `var start = -1, offset = 0;` line to `start = -1; offset = 0;` so it does not redeclare. Verify there is exactly one `var start` in the function after editing.)

- [ ] **Step 3: Add `navigate(dir)`**

After `nextChannelId` (`base.html` ~L459), add:

```js
      // Unified vertical navigation: the whole channel list is one continuous
      // feed. On-demand channels expand into items; live/vod_loop are single
      // entries. Steps items first, then overflows to the adjacent channel.
      // Wraps at the ends (matching nextChannelId).
      function navigate(dir) {
        if (odChannelId === currentChannelId && odItems.length) {
          if (dir === 'down') {
            var n = odNextAvailable(odIndex + 1);
            if (n >= 0) { odPlayIndex(n, 0); return; }
          } else {
            var p = odPrevAvailable(odIndex - 1);
            if (p >= 0) { odPlayIndex(p, 0); return; }
          }
          // fall through: at a playlist edge → cross into the adjacent channel
        }
        var nextId = nextChannelId(dir === 'up' ? 'up' : 'down');
        if (!nextId) return;
        if (dir === 'up' && odChannelType(nextId) === 'vod_on_demand') {
          odStartAtLast = true;
        }
        tune(nextId);
      }
      window.navigate = navigate;
```

- [ ] **Step 4: Rewire keyboard ↑/↓ to `navigate`, and ungate seek**

In the `keydown` handler (`base.html` ~L851–864), replace the `ArrowUp`/`ArrowDown` block with:

```js
        if (e.key === 'ArrowUp' || e.key === 'ArrowDown') {
          e.preventDefault();
          navigate(e.key === 'ArrowUp' ? 'up' : 'down');
          return;
        }
```

and replace the `ArrowLeft`/`ArrowRight` block with (ungated for on-demand):

```js
        if (e.key === 'ArrowLeft' || e.key === 'ArrowRight') {
          if (!currentChannel) return;
          var t = currentChannel.channel_type;
          if (t !== 'vod_loop' && t !== 'vod_on_demand') return;
          if (!video || video.readyState < 1) return;
          e.preventDefault();
          video.currentTime += e.key === 'ArrowRight' ? 10 : -10;
          return;
        }
```

- [ ] **Step 5: Wire the ⏮/⏭ buttons to `navigate`**

In the toolbar wiring block (Task 2 Step 2), add after the fullscreen wiring:

```js
      var ovPrevItem = document.getElementById('ov-prev-item');
      var ovNextItem = document.getElementById('ov-next-item');
      if (ovPrevItem) ovPrevItem.addEventListener('click', function() { navigate('up'); });
      if (ovNextItem) ovNextItem.addEventListener('click', function() { navigate('down'); });
```

- [ ] **Step 6: Automated gate**

Run: `cargo fmt && cargo clippy -- -D warnings && cargo test`
Expected: PASS.

- [ ] **Step 7: Manual gate**

`cargo run`. Verify:
- On a **vod_on_demand** channel with ≥2 items: `↓` / `⏭` advances to the next item; `↑` / `⏮` goes to the previous item; the ☰ panel `▶` highlight follows.
- At the **last** item, `↓` switches to the next channel (its first item if on-demand). At the **first** item, `↑` switches to the previous channel — landing on its **last** item if that channel is on-demand.
- On **live** / **vod_loop**: `↑`/`↓` and `⏮`/`⏭` change channel (wrapping at the ends).
- `←`/`→` now seeks on **vod_on_demand** (previously did nothing) and still on **vod_loop**; does nothing on **live**.

- [ ] **Step 8: Commit**

```bash
git add templates/base.html
git commit -m "feat(player): unified vertical item→channel navigation (idea #51)"
```

---

## Task 5: Touch gesture layer (tap / double-tap-edge / vertical swipe)

**Files:**
- Modify: `templates/base.html` (player JS)

- [ ] **Step 1: Add the gesture layer bound to `#player-panel`**

After the toolbar wiring block, add:

```js
      // ── touch gestures (idea #51) ─────────────────────────────
      // Bound to the panel. Distinguishes: vertical swipe (navigate),
      // double-tap near an edge (seek), single tap (toggle chrome).
      (function() {
        var panelEl = document.getElementById('player-panel');
        if (!panelEl) return;
        var sx = 0, sy = 0, st = 0, moved = false;
        var lastTapT = 0, lastTapX = 0;
        var SWIPE = 40;      // px to count as a swipe
        var TAP_MS = 300;    // max ms between taps for a double-tap

        panelEl.addEventListener('touchstart', function(e) {
          if (e.touches.length !== 1) return;
          sx = e.touches[0].clientX; sy = e.touches[0].clientY;
          st = e.timeStamp; moved = false;
        }, {passive: true});

        panelEl.addEventListener('touchmove', function(e) {
          if (e.touches.length !== 1) return;
          var dx = e.touches[0].clientX - sx, dy = e.touches[0].clientY - sy;
          if (Math.abs(dx) > 10 || Math.abs(dy) > 10) moved = true;
          // vertical swipe: prevent the page from scrolling under us
          if (Math.abs(dy) > Math.abs(dx) && Math.abs(dy) > 10) e.preventDefault();
        }, {passive: false});

        panelEl.addEventListener('touchend', function(e) {
          var ex = (e.changedTouches[0] || {}).clientX || sx;
          var ey = (e.changedTouches[0] || {}).clientY || sy;
          var dx = ex - sx, dy = ey - sy;
          // ignore touches that land on a real control
          if (e.target.closest && e.target.closest('.ov-btn, #ov-seek, #ov-center-play, .pl-row, #player-playlist')) return;

          if (Math.abs(dy) > SWIPE && Math.abs(dy) > Math.abs(dx)) {
            navigate(dy < 0 ? 'down' : 'up');   // swipe up = next, swipe down = prev
            return;
          }
          if (!moved) {
            var now = e.timeStamp;
            var rect = panelEl.getBoundingClientRect();
            var edge = rect.width * 0.3;
            if (now - lastTapT < TAP_MS && Math.abs(ex - lastTapX) < 60) {
              // double-tap: seek if near an edge and the channel is seekable
              var t = currentChannel && currentChannel.channel_type;
              if ((t === 'vod_loop' || t === 'vod_on_demand') && video && video.readyState >= 1) {
                if (ex - rect.left < edge) { video.currentTime -= 10; }
                else if (rect.right - ex < edge) { video.currentTime += 10; }
              }
              lastTapT = 0;
              return;
            }
            lastTapT = now; lastTapX = ex;
            // single tap: toggle chrome (showControls reveals; tapping while
            // shown hides immediately)
            if (panelEl.classList.contains('show-controls')) {
              panelEl.classList.remove('show-controls');
            } else {
              showControls();
            }
          }
        }, {passive: true});
      })();
```

Note: swipe **up** = next (down the feed), swipe **down** = prev — matching the natural "pull content up to advance" direction. The existing `panel.addEventListener('touchstart', showControls, {passive:true})` at ~L762 stays; it harmlessly reveals chrome on touch start, and the tap handler above toggles on `touchend`.

- [ ] **Step 2: Resolve the double-reveal interaction**

The existing `showControls` on `touchstart` (~L762) plus the tap-toggle on `touchend` would re-reveal chrome the user just tapped to hide. Change the `touchstart` listener at ~L762 to NOT call `showControls` (the gesture layer now owns tap→chrome). Edit:

```js
        panel.addEventListener('touchstart', showControls, {passive: true});
```

to:

```js
        // touch chrome toggling is handled by the gesture layer (idea #51)
```

(Remove the line; keep the `mousemove` and `focusin` listeners.)

- [ ] **Step 3: Automated gate**

Run: `cargo fmt && cargo clippy -- -D warnings && cargo test`
Expected: PASS.

- [ ] **Step 4: Manual gate (use browser device emulation or a real phone)**

In Chrome DevTools device mode (or a phone on the LAN), tune channels and verify:
- **Single tap** on the video toggles chrome on/off (without triggering play/pause or seek).
- **Swipe up** advances (next item, then next channel at the edge); **swipe down** goes back. The page does not scroll while swiping vertically on the video.
- **Double-tap** on the left third seeks −10s; on the right third seeks +10s (on vod_loop / on-demand only).
- Tapping the actual buttons (play, ⛶, ☰, playlist rows) still works and does not trigger a swipe/seek.

- [ ] **Step 5: Commit**

```bash
git add templates/base.html
git commit -m "feat(player): touch gesture layer — tap, double-tap seek, swipe nav (idea #51)"
```

---

## Task 6: Finalize keyboard map (Esc close, help), drop stale bits

**Files:**
- Modify: `templates/base.html` (player JS keydown handler ~L829–865)

- [ ] **Step 1: Add Esc-close and `?`-help to the keydown handler**

In the `document.addEventListener('keydown', ...)` handler, after the existing `f`/`F` fullscreen block, add:

```js
        if (e.key === 'Escape') {
          if (document.fullscreenElement) return; // let native fullscreen handle Esc
          var p = document.getElementById('player-panel');
          if (p && p.style.display !== 'none') { closePlayer(); }
          return;
        }

        if (e.key === '?') {
          if (helpBox) helpBox.hidden = !helpBox.hidden;
          return;
        }
```

Also change the `f`/`F` block to call the shared helper so keyboard and button match:

```js
        if (e.key === 'f' || e.key === 'F') {
          toggleFullscreen();
          return;
        }
```

- [ ] **Step 2: Confirm no stale references**

Grep the file to confirm the removed handles are gone and nothing references them:

Run: `grep -n "ov-prev'\|ov-next'\|getElementById('ov-prev')\|getElementById('ov-next')\|\\[.*ArrowLeft.*vod_loop" templates/base.html`
Expected: no matches for `ov-prev`/`ov-next` (replaced by `ov-prev-item`/`ov-next-item`). The old toolbar used `ov-prev`/`ov-next` for channel up/down; these IDs no longer exist in the markup, so any leftover JS referencing them must be removed.

- [ ] **Step 3: Automated gate**

Run: `cargo fmt && cargo clippy -- -D warnings && cargo test`
Expected: PASS.

- [ ] **Step 4: Full manual regression across the interaction table**

`cargo run`. Walk the complete table for each channel type (live, vod_loop, vod_on_demand), desktop + emulated touch:

| Action | Touch | Keyboard | Mouse |
|---|---|---|---|
| Play/pause | tap center | Space | center / transport button |
| Seek ±10s | double-tap edge | ←/→ | drag scrubber |
| Navigate | swipe ↑/↓ | ↑/↓ | ⏮/⏭ |
| Fullscreen | ⛶ | F | ⛶ |
| Close | ✕ | Esc | ✕ |
| Help | ? | ? | ? |

Confirm: live has no seek; vod_loop seeks, no item nav; vod_on_demand does items+seek; feed boundaries cross channels and wrap at the ends.

- [ ] **Step 5: Commit**

```bash
git add templates/base.html
git commit -m "feat(player): finalize keyboard map — Esc close, ? help (idea #51)"
```

---

## Wrap-up

- [ ] Update `docs/IDEAS.md`: move idea #51 from **Open** to the **Done** count, and add a `docs/CHANGELOG.md` entry summarizing the redesign with its rationale (matching the project's idea-close-out convention).
- [ ] `cargo fmt && cargo clippy -- -D warnings && cargo test` one final time.
- [ ] Use superpowers:finishing-a-development-branch to decide how to integrate `idea-51-player-overlay-redesign` (merge / PR).

## Decisions confirmed with the user (2026-06-16)

1. **Feed ends wrap** — `nextChannelId`'s existing wrap behavior is preserved (`↓` past the last item → first channel; `↑` past the first → last). No `nextChannelId` change needed.
2. **Swipe up = next** — content scrolls up to advance (feed style); swipe down = previous. This is the plan's default (the `dy < 0` test in Task 5).
