# DASH Stream Support Implementation Plan

> **For agentic workers:** REQUIRED SUB-SKILL: Use superpowers:subagent-driven-development (recommended) or superpowers:executing-plans to implement this plan task-by-task. Steps use checkbox (`- [ ]`) syntax for tracking.

**Goal:** Add MPEG-DASH playback to MyTV — proxy rewrites MPD manifests, dash.js plays them in the browser.

**Architecture:** A new `src/media/mpd.rs` module (mirroring `hls.rs`) rewrites DASH MPD XML using `quick-xml`: it resolves relative `<BaseURL>` elements to absolute URLs and wraps absolute `<SegmentTemplate>`/`<SegmentURL>` attributes in `/stream-proxy`. The stream proxy in `player.rs` detects `.mpd` URLs and dispatches to this rewriter. The frontend detects `.mpd` in the resolved URL and uses a `dashjs.MediaPlayer` instance instead of hls.js.

**Tech Stack:** `quick-xml = "0.40"` (Rust XML streaming), `dashjs@4.7.4` (CDN script), existing Axum stream proxy.

> **Implementation note (corrected from spec):** The BBB test stream uses `<BaseURL>./</BaseURL>` with relative `<SegmentTemplate>` URLs. Wrapping a relative BaseURL in `/stream-proxy?url=…` would break dash.js URL resolution (RFC 3986 relative resolution ignores query parameters). The correct behavior: **always resolve relative BaseURL to absolute** using the actual MPD URL; **leave absolute BaseURL unchanged**; only wrap absolute SegmentTemplate/SegmentURL URLs in the proxy. Streams with relative SegmentTemplate (like BBB) rely on CDN CORS for segment delivery — the BBB CDN has `Access-Control-Allow-Origin: *`.

---

## File Map

| File | Action | Responsibility |
|---|---|---|
| `Cargo.toml` | Modify | Add `quick-xml = "0.40"` |
| `tests/fixtures/bbb_30fps.mpd` | Create | Real BBB DASH fixture for unit tests |
| `src/media/mod.rs` | Modify | Add `pub mod mpd;` |
| `src/media/mpd.rs` | Create | `rewrite_mpd_urls`, `pct_encode_template`, `resolve_relative_url`, `rewrite_url_attrs` |
| `src/routes/player.rs` | Modify | Add `is_dash` detection + dispatch to `mpd::rewrite_mpd_urls` |
| `templates/base.html` | Modify | Add dash.js, DASH player branch in `_loadSource`, failover handler |
| `tests/http.rs` | Modify | Add `#[ignore]` network test for `/stream-proxy` + BBB MPD |

---

## Task 1: Add `quick-xml` dependency and BBB fixture

**Files:**
- Modify: `Cargo.toml`
- Create: `tests/fixtures/bbb_30fps.mpd`

- [ ] **Step 1: Add `quick-xml` to `Cargo.toml`**

  Open `Cargo.toml` and add to `[dependencies]`:

  ```toml
  quick-xml = "0.40"
  ```

- [ ] **Step 2: Download the BBB fixture**

  ```bash
  curl -o tests/fixtures/bbb_30fps.mpd \
    "https://dash.akamaized.net/akamai/bbb_30fps/bbb_30fps.mpd"
  ```

  Verify it starts with `<MPD`:

  ```bash
  head -3 tests/fixtures/bbb_30fps.mpd
  ```

  Expected first line: `<MPD mediaPresentationDuration=...`

- [ ] **Step 3: Verify dependency resolves**

  ```bash
  cargo fetch
  ```

  Expected: exits 0 with no errors.

- [ ] **Step 4: Commit**

  ```bash
  git add Cargo.toml Cargo.lock tests/fixtures/bbb_30fps.mpd
  git commit -m "chore: add quick-xml dep and BBB DASH fixture"
  ```

---

## Task 2: Create `src/media/mpd.rs` with TDD

**Files:**
- Modify: `src/media/mod.rs`
- Create: `src/media/mpd.rs`

- [ ] **Step 1: Register the module**

  In `src/media/mod.rs`, add `pub mod mpd;` at line 1 (after existing `pub mod hls;`):

  ```rust
  pub mod hls;
  pub mod m3u;
  pub mod mpd;
  pub mod resolver;
  ```

- [ ] **Step 2: Create `src/media/mpd.rs` with failing tests**

  Create the file with the test module only (no implementation yet):

  ```rust
  use crate::media::hls::pct_encode;
  use quick_xml::events::{BytesStart, BytesText, Event};
  use quick_xml::{Reader, Writer};
  use std::io::Cursor;

  pub fn rewrite_mpd_urls(_xml: &str, _base_url: &str, _direct: bool) -> String {
      unimplemented!()
  }

  fn pct_encode_template(_s: &str) -> String {
      unimplemented!()
  }

  fn resolve_relative_url(_url: &str, _base_url: &str) -> String {
      unimplemented!()
  }

  fn rewrite_url_attrs(_e: BytesStart<'_>, _url_attr_names: &[&[u8]]) -> BytesStart<'static> {
      unimplemented!()
  }

  #[cfg(test)]
  mod tests {
      use super::*;

      #[test]
      fn relative_base_url_resolved_to_absolute() {
          let xml = r#"<?xml version="1.0"?><MPD><BaseURL>./</BaseURL></MPD>"#;
          let out = rewrite_mpd_urls(xml, "https://origin.example.com/path/stream.mpd", false);
          assert!(
              out.contains("<BaseURL>https://origin.example.com/path/</BaseURL>"),
              "got: {out}"
          );
      }

      #[test]
      fn absolute_base_url_left_unchanged() {
          let xml = r#"<?xml version="1.0"?><MPD><BaseURL>https://cdn.example.com/</BaseURL></MPD>"#;
          let out = rewrite_mpd_urls(xml, "https://origin.example.com/stream.mpd", false);
          assert!(
              out.contains("<BaseURL>https://cdn.example.com/</BaseURL>"),
              "got: {out}"
          );
          assert!(!out.contains("/stream-proxy"), "absolute BaseURL must not be proxied");
      }

      #[test]
      fn rewrite_segment_template_media_absolute() {
          let xml = r#"<?xml version="1.0"?><MPD><SegmentTemplate media="https://cdn.example.com/video/$RepresentationID$/seg-$Number$.m4s" initialization="https://cdn.example.com/video/$RepresentationID$/init.mp4"/></MPD>"#;
          let out = rewrite_mpd_urls(xml, "https://origin.example.com/stream.mpd", false);
          // URL is proxied; $ template variables preserved (not percent-encoded)
          assert!(
              out.contains("media=\"/stream-proxy?url=https%3A%2F%2Fcdn.example.com%2Fvideo%2F$RepresentationID$%2Fseg-$Number$.m4s\""),
              "got: {out}"
          );
          assert!(
              out.contains("initialization=\"/stream-proxy?url=https%3A%2F%2Fcdn.example.com%2Fvideo%2F$RepresentationID$%2Finit.mp4\""),
              "got: {out}"
          );
      }

      #[test]
      fn relative_segment_template_left_unchanged() {
          let xml = r#"<?xml version="1.0"?><MPD><SegmentTemplate media="video/$RepresentationID$/seg-$Number$.m4s"/></MPD>"#;
          let out = rewrite_mpd_urls(xml, "https://origin.example.com/stream.mpd", false);
          assert!(
              out.contains(r#"media="video/$RepresentationID$/seg-$Number$.m4s""#),
              "got: {out}"
          );
          assert!(!out.contains("/stream-proxy"), "relative template must not be proxied");
      }

      #[test]
      fn rewrite_segment_url_media_absolute() {
          let xml = r#"<?xml version="1.0"?><MPD><SegmentList><SegmentURL media="https://cdn.example.com/video/seg-1.m4s"/></SegmentList></MPD>"#;
          let out = rewrite_mpd_urls(xml, "https://origin.example.com/stream.mpd", false);
          assert!(
              out.contains("media=\"/stream-proxy?url=https%3A%2F%2Fcdn.example.com%2Fvideo%2Fseg-1.m4s\""),
              "got: {out}"
          );
      }

      #[test]
      fn direct_mode_does_not_proxy_segments_but_still_resolves_base_url() {
          let xml = r#"<?xml version="1.0"?><MPD><BaseURL>./</BaseURL><SegmentTemplate media="https://cdn.example.com/seg-$Number$.m4s"/></MPD>"#;
          let out = rewrite_mpd_urls(xml, "https://origin.example.com/path/stream.mpd", true);
          // BaseURL resolved even in direct mode
          assert!(
              out.contains("<BaseURL>https://origin.example.com/path/</BaseURL>"),
              "got: {out}"
          );
          // SegmentTemplate NOT proxied (direct=true)
          assert!(
              !out.contains("/stream-proxy"),
              "direct mode must not proxy segment URLs"
          );
      }

      #[test]
      fn bbb_fixture_resolves_relative_base_url() {
          let xml = include_str!("../../tests/fixtures/bbb_30fps.mpd");
          let out = rewrite_mpd_urls(
              xml,
              "https://dash.akamaized.net/akamai/bbb_30fps/bbb_30fps.mpd",
              false,
          );
          // Relative BaseURL "./" resolved to absolute CDN path
          assert!(
              out.contains("<BaseURL>https://dash.akamaized.net/akamai/bbb_30fps/</BaseURL>"),
              "got BaseURL section: {}", &out[..out.find("</BaseURL>").unwrap_or(200) + 10]
          );
          // BBB uses relative SegmentTemplate — no proxy URLs injected
          assert!(
              !out.contains("/stream-proxy?url="),
              "relative templates must not be proxied"
          );
          // Output is still valid XML (contains MPD root)
          assert!(out.contains("<MPD"));
      }
  }
  ```

- [ ] **Step 3: Run tests to confirm they fail**

  ```bash
  cargo test -p mytv media::mpd 2>&1 | tail -20
  ```

  Expected: multiple `panicked at 'not yet implemented'` errors. All mpd tests fail.

- [ ] **Step 4: Implement `src/media/mpd.rs`**

  Replace the entire file with the full implementation:

  ```rust
  use crate::media::hls::pct_encode;
  use quick_xml::events::{BytesStart, BytesText, Event};
  use quick_xml::{Reader, Writer};
  use std::io::Cursor;

  /// Rewrites a DASH MPD manifest:
  /// - Relative <BaseURL> text → resolved to absolute (always, even when direct=true)
  /// - Absolute <BaseURL> text → left unchanged
  /// - Absolute <SegmentTemplate media/initialization> attrs → wrapped in /stream-proxy (unless direct=true)
  /// - Absolute <SegmentURL media> attrs → wrapped in /stream-proxy (unless direct=true)
  /// - Relative segment URLs → left unchanged (resolve against BaseURL on CDN)
  pub fn rewrite_mpd_urls(xml: &str, base_url: &str, direct: bool) -> String {
      let mut reader = Reader::from_str(xml);
      let mut writer = Writer::new(Cursor::new(Vec::new()));
      let mut in_base_url = false;

      loop {
          match reader.read_event() {
              Ok(Event::Start(e)) => match e.local_name().as_ref() {
                  b"BaseURL" => {
                      in_base_url = true;
                      writer.write_event(Event::Start(e)).unwrap();
                  }
                  b"SegmentTemplate" if !direct => {
                      let rewritten = rewrite_url_attrs(e, &[b"media", b"initialization"]);
                      writer.write_event(Event::Start(rewritten)).unwrap();
                  }
                  _ => {
                      writer.write_event(Event::Start(e)).unwrap();
                  }
              },
              Ok(Event::Empty(e)) => match e.local_name().as_ref() {
                  b"SegmentTemplate" if !direct => {
                      let rewritten = rewrite_url_attrs(e, &[b"media", b"initialization"]);
                      writer.write_event(Event::Empty(rewritten)).unwrap();
                  }
                  b"SegmentURL" if !direct => {
                      let rewritten = rewrite_url_attrs(e, &[b"media"]);
                      writer.write_event(Event::Empty(rewritten)).unwrap();
                  }
                  _ => {
                      writer.write_event(Event::Empty(e)).unwrap();
                  }
              },
              Ok(Event::Text(e)) => {
                  if in_base_url {
                      let text = e.unescape().unwrap_or_default();
                      let trimmed = text.trim();
                      if trimmed.starts_with("http://") || trimmed.starts_with("https://") {
                          // Absolute BaseURL — leave unchanged
                          writer.write_event(Event::Text(e)).unwrap();
                      } else {
                          // Relative BaseURL — resolve to absolute
                          let resolved = resolve_relative_url(trimmed, base_url);
                          writer
                              .write_event(Event::Text(BytesText::new(&resolved)))
                              .unwrap();
                      }
                  } else {
                      writer.write_event(Event::Text(e)).unwrap();
                  }
              }
              Ok(Event::End(e)) => {
                  if e.local_name().as_ref() == b"BaseURL" {
                      in_base_url = false;
                  }
                  writer.write_event(Event::End(e)).unwrap();
              }
              Ok(Event::Eof) => break,
              Ok(e) => {
                  writer.write_event(e).unwrap();
              }
              Err(_) => break,
          }
      }

      String::from_utf8(writer.into_inner().into_inner())
          .unwrap_or_else(|_| xml.to_string())
  }

  /// Like `pct_encode` but preserves `$` so DASH template variables
  /// (`$Number$`, `$RepresentationID$`, etc.) survive proxy URL wrapping.
  fn pct_encode_template(s: &str) -> String {
      s.bytes()
          .map(|b| match b {
              b'A'..=b'Z'
              | b'a'..=b'z'
              | b'0'..=b'9'
              | b'-'
              | b'_'
              | b'.'
              | b'~'
              | b'$' => (b as char).to_string(),
              _ => format!("%{:02X}", b),
          })
          .collect()
  }

  /// Resolves a URL against a base URL.
  /// Absolute URLs are returned unchanged.
  /// Relative URLs (including `./`) are combined with the base URL's directory.
  fn resolve_relative_url(url: &str, base_url: &str) -> String {
      if url.starts_with("http://") || url.starts_with("https://") {
          return url.to_string();
      }
      if url.starts_with('/') {
          let after_scheme = base_url.find("://").map(|i| i + 3).unwrap_or(0);
          let host_len = base_url[after_scheme..]
              .find('/')
              .unwrap_or(base_url[after_scheme..].len());
          let origin = &base_url[..after_scheme + host_len];
          return format!("{}{}", origin, url);
      }
      let base_dir = base_url
          .rsplit_once('/')
          .map(|(b, _)| b)
          .unwrap_or(base_url);
      let stripped = url.trim_start_matches("./");
      if stripped.is_empty() {
          format!("{}/", base_dir)
      } else {
          format!("{}/{}", base_dir, stripped)
      }
  }

  /// Rewrites named URL attributes on a start/empty element.
  /// Absolute HTTP(S) URLs are wrapped in `/stream-proxy?url=…` using
  /// `pct_encode_template` (which preserves `$` for DASH template variables).
  /// Relative URLs and non-URL attributes are passed through unchanged.
  fn rewrite_url_attrs(e: BytesStart<'_>, url_attr_names: &[&[u8]]) -> BytesStart<'static> {
      let name = std::str::from_utf8(e.name().as_ref())
          .unwrap_or_default()
          .to_owned();
      let mut new = BytesStart::new(name);
      for attr in e.attributes().flatten() {
          let key = attr.key.as_ref().to_owned();
          let val = attr.unescape_value().unwrap_or_default().into_owned();
          let new_val = if url_attr_names.iter().any(|n| *n == key.as_slice())
              && (val.starts_with("http://") || val.starts_with("https://"))
          {
              format!("/stream-proxy?url={}", pct_encode_template(&val))
          } else {
              val
          };
          new.push_attribute((key.as_slice(), new_val.as_bytes()));
      }
      new
  }

  #[cfg(test)]
  mod tests {
      use super::*;

      #[test]
      fn relative_base_url_resolved_to_absolute() {
          let xml = r#"<?xml version="1.0"?><MPD><BaseURL>./</BaseURL></MPD>"#;
          let out = rewrite_mpd_urls(xml, "https://origin.example.com/path/stream.mpd", false);
          assert!(
              out.contains("<BaseURL>https://origin.example.com/path/</BaseURL>"),
              "got: {out}"
          );
      }

      #[test]
      fn absolute_base_url_left_unchanged() {
          let xml = r#"<?xml version="1.0"?><MPD><BaseURL>https://cdn.example.com/</BaseURL></MPD>"#;
          let out = rewrite_mpd_urls(xml, "https://origin.example.com/stream.mpd", false);
          assert!(
              out.contains("<BaseURL>https://cdn.example.com/</BaseURL>"),
              "got: {out}"
          );
          assert!(!out.contains("/stream-proxy"), "absolute BaseURL must not be proxied");
      }

      #[test]
      fn rewrite_segment_template_media_absolute() {
          let xml = r#"<?xml version="1.0"?><MPD><SegmentTemplate media="https://cdn.example.com/video/$RepresentationID$/seg-$Number$.m4s" initialization="https://cdn.example.com/video/$RepresentationID$/init.mp4"/></MPD>"#;
          let out = rewrite_mpd_urls(xml, "https://origin.example.com/stream.mpd", false);
          assert!(
              out.contains("media=\"/stream-proxy?url=https%3A%2F%2Fcdn.example.com%2Fvideo%2F$RepresentationID$%2Fseg-$Number$.m4s\""),
              "got: {out}"
          );
          assert!(
              out.contains("initialization=\"/stream-proxy?url=https%3A%2F%2Fcdn.example.com%2Fvideo%2F$RepresentationID$%2Finit.mp4\""),
              "got: {out}"
          );
      }

      #[test]
      fn relative_segment_template_left_unchanged() {
          let xml = r#"<?xml version="1.0"?><MPD><SegmentTemplate media="video/$RepresentationID$/seg-$Number$.m4s"/></MPD>"#;
          let out = rewrite_mpd_urls(xml, "https://origin.example.com/stream.mpd", false);
          assert!(
              out.contains(r#"media="video/$RepresentationID$/seg-$Number$.m4s""#),
              "got: {out}"
          );
          assert!(!out.contains("/stream-proxy"), "relative template must not be proxied");
      }

      #[test]
      fn rewrite_segment_url_media_absolute() {
          let xml = r#"<?xml version="1.0"?><MPD><SegmentList><SegmentURL media="https://cdn.example.com/video/seg-1.m4s"/></SegmentList></MPD>"#;
          let out = rewrite_mpd_urls(xml, "https://origin.example.com/stream.mpd", false);
          assert!(
              out.contains("media=\"/stream-proxy?url=https%3A%2F%2Fcdn.example.com%2Fvideo%2Fseg-1.m4s\""),
              "got: {out}"
          );
      }

      #[test]
      fn direct_mode_does_not_proxy_segments_but_still_resolves_base_url() {
          let xml = r#"<?xml version="1.0"?><MPD><BaseURL>./</BaseURL><SegmentTemplate media="https://cdn.example.com/seg-$Number$.m4s"/></MPD>"#;
          let out = rewrite_mpd_urls(xml, "https://origin.example.com/path/stream.mpd", true);
          assert!(
              out.contains("<BaseURL>https://origin.example.com/path/</BaseURL>"),
              "got: {out}"
          );
          assert!(
              !out.contains("/stream-proxy"),
              "direct mode must not proxy segment URLs"
          );
      }

      #[test]
      fn bbb_fixture_resolves_relative_base_url() {
          let xml = include_str!("../../tests/fixtures/bbb_30fps.mpd");
          let out = rewrite_mpd_urls(
              xml,
              "https://dash.akamaized.net/akamai/bbb_30fps/bbb_30fps.mpd",
              false,
          );
          assert!(
              out.contains("<BaseURL>https://dash.akamaized.net/akamai/bbb_30fps/</BaseURL>"),
              "got BaseURL section: {}",
              &out[..out.find("</BaseURL>").unwrap_or(200) + 10]
          );
          assert!(
              !out.contains("/stream-proxy?url="),
              "relative templates must not be proxied"
          );
          assert!(out.contains("<MPD"));
      }
  }
  ```

- [ ] **Step 5: Run tests to confirm they pass**

  ```bash
  cargo test -p mytv media::mpd 2>&1 | tail -20
  ```

  Expected: `test media::mpd::tests::... ok` for all 6 tests. No failures.

- [ ] **Step 6: Run full test suite**

  ```bash
  cargo test 2>&1 | tail -10
  ```

  Expected: all existing tests still pass.

- [ ] **Step 7: Format and lint**

  ```bash
  cargo fmt && cargo clippy -- -D warnings
  ```

  Expected: no warnings, no errors.

- [ ] **Step 8: Commit**

  ```bash
  git add src/media/mod.rs src/media/mpd.rs
  git commit -m "feat: add MPD URL rewriter (quick-xml streaming)"
  ```

---

## Task 3: Wire the stream proxy to handle DASH manifests

**Files:**
- Modify: `src/routes/player.rs:12` (import)
- Modify: `src/routes/player.rs:256` (`is_playlist` detection)
- Modify: `src/routes/player.rs:311-318` (rewrite dispatch)

- [ ] **Step 1: Add `mpd` to imports**

  In `src/routes/player.rs`, change line 12:

  ```rust
  // before
  use crate::{
      media::{hls, resolver},
  ```

  ```rust
  // after
  use crate::{
      media::{hls, mpd, resolver},
  ```

- [ ] **Step 2: Add DASH detection to `is_playlist` (line 256)**

  ```rust
  // before (line 256)
  let is_playlist = ct.contains("mpegurl") || url.contains(".m3u8") || url.contains(".m3u");
  ```

  ```rust
  // after
  let is_dash = ct.contains("dash+xml") || url.contains(".mpd");
  let is_playlist =
      is_dash || ct.contains("mpegurl") || url.contains(".m3u8") || url.contains(".m3u");
  ```

- [ ] **Step 3: Branch on `is_dash` for rewriting (lines 311-318)**

  ```rust
  // before
  let text = String::from_utf8_lossy(&body_bytes);
  let direct = resolve_direct_segments(&state, &url).await;
  let rewritten = hls::rewrite_hls_urls(&text, &url, direct);
  headers.insert(
      axum::http::header::CONTENT_TYPE,
      HeaderValue::from_static("application/vnd.apple.mpegurl"),
  );
  (status, headers, rewritten).into_response()
  ```

  ```rust
  // after
  let text = String::from_utf8_lossy(&body_bytes);
  let direct = resolve_direct_segments(&state, &url).await;
  let (rewritten, content_type) = if is_dash {
      (
          mpd::rewrite_mpd_urls(&text, &url, direct),
          "application/dash+xml",
      )
  } else {
      (
          hls::rewrite_hls_urls(&text, &url, direct),
          "application/vnd.apple.mpegurl",
      )
  };
  headers.insert(
      axum::http::header::CONTENT_TYPE,
      HeaderValue::from_static(content_type),
  );
  (status, headers, rewritten).into_response()
  ```

- [ ] **Step 4: Run full test suite**

  ```bash
  cargo test 2>&1 | tail -10
  ```

  Expected: all tests pass. No regressions.

- [ ] **Step 5: Format and lint**

  ```bash
  cargo fmt && cargo clippy -- -D warnings
  ```

  Expected: clean.

- [ ] **Step 6: Commit**

  ```bash
  git add src/routes/player.rs
  git commit -m "feat: stream proxy detects and rewrites DASH MPD manifests"
  ```

---

## Task 4: Frontend — add dash.js and DASH player support

**Files:**
- Modify: `templates/base.html`

- [ ] **Step 1: Add dash.js CDN script**

  In `templates/base.html`, after the existing hls.js script tag (line 11):

  ```html
  <!-- before -->
  <script src="https://cdn.jsdelivr.net/npm/hls.js@1.5.13"></script>
  ```

  ```html
  <!-- after -->
  <script src="https://cdn.jsdelivr.net/npm/hls.js@1.5.13"></script>
  <script src="https://cdn.jsdelivr.net/npm/dashjs@4.7.4/dist/dash.all.min.js"></script>
  ```

- [ ] **Step 2: Add DASH player state variables**

  In `templates/base.html`, inside `DOMContentLoaded`, after `let currentChannel = null;` (around line 124):

  ```javascript
  // before
  let currentUrl = null;
  let currentChannel = null;
  ```

  ```javascript
  // after
  let currentUrl = null;
  let currentChannel = null;
  let dash = null;
  let dashErrorFired = false;
  ```

- [ ] **Step 3: Replace `_loadSource` with the DASH-aware version**

  Find the existing `function _loadSource(url, offset) { ... }` block (starting around line 223) and replace the entire function:

  ```javascript
  function _loadSource(url, offset) {
    currentUrl = url;
    dashErrorFired = false;
    var isDash = url.indexOf('.mpd') >= 0;
    url = proxyUrl(url);

    if (isDash) {
      if (hls) { hls.stopLoad(); hls.detachMedia(); }
      if (dash) { dash.reset(); dash = null; }
      dash = dashjs.MediaPlayer().create();
      dash.on(dashjs.MediaPlayer.events.ERROR, function() {
        if (dashErrorFired || !currentChannelId) return;
        dashErrorFired = true;
        if (typeof debugLog === 'function') debugLog('warn', 'DASH error, trying next source');
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
        });
      }
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
  }
  ```

- [ ] **Step 4: Add DASH teardown to `tune()`**

  Find `function tune(channelId) {` (around line 270) and add teardown after `hidePlayerError();`:

  ```javascript
  // before
  function tune(channelId) {
    currentChannelId = channelId;
    currentUrl = null;
    hidePlayerError();
    document.getElementById('player-panel').style.display = 'block';
  ```

  ```javascript
  // after
  function tune(channelId) {
    currentChannelId = channelId;
    currentUrl = null;
    hidePlayerError();
    if (hls) hls.stopLoad();
    if (dash) { dash.reset(); dash = null; }
    document.getElementById('player-panel').style.display = 'block';
  ```

- [ ] **Step 5: Verify `cargo test` still passes (templates are not compiled, but sanity check)**

  ```bash
  cargo test 2>&1 | tail -5
  ```

  Expected: all pass.

- [ ] **Step 6: Commit**

  ```bash
  git add templates/base.html
  git commit -m "feat: add DASH playback via dash.js with failover"
  ```

---

## Task 5: Add `#[ignore]` network integration test

**Files:**
- Modify: `tests/http.rs`

- [ ] **Step 1: Add the `#[ignore]` test**

  At the end of `tests/http.rs`, add:

  ```rust
  #[tokio::test]
  #[ignore = "requires network access — run manually"]
  async fn test_stream_proxy_rewrites_dash_bbb_manifest() {
      use http_body_util::BodyExt;
      let app = app().await;
      let encoded_url =
          "https%3A%2F%2Fdash.akamaized.net%2Fakamai%2Fbbb_30fps%2Fbbb_30fps.mpd";
      let response = app
          .oneshot(req(&format!("/stream-proxy?url={encoded_url}")))
          .await
          .unwrap();
      assert_eq!(response.status(), StatusCode::OK);
      let ct = response
          .headers()
          .get("content-type")
          .and_then(|v| v.to_str().ok())
          .unwrap_or("");
      assert!(ct.contains("dash+xml"), "expected dash+xml content-type, got: {ct}");
      let bytes = response.into_body().collect().await.unwrap().to_bytes();
      let body = std::str::from_utf8(&bytes).unwrap();
      // BaseURL "./" resolved to absolute CDN path
      assert!(
          body.contains("https://dash.akamaized.net/akamai/bbb_30fps/"),
          "expected resolved absolute BaseURL in body"
      );
      // Valid DASH XML
      assert!(body.contains("<MPD"), "expected MPD root element");
  }
  ```

- [ ] **Step 2: Run it manually to verify against the live stream**

  ```bash
  cargo test test_stream_proxy_rewrites_dash_bbb_manifest -- --ignored --nocapture
  ```

  Expected: `test test_stream_proxy_rewrites_dash_bbb_manifest ... ok`

- [ ] **Step 3: Confirm it is skipped in normal test runs**

  ```bash
  cargo test 2>&1 | grep -E "ignored|test result"
  ```

  Expected: output includes `1 ignored` and all non-ignored tests pass.

- [ ] **Step 4: Format and lint**

  ```bash
  cargo fmt && cargo clippy -- -D warnings
  ```

  Expected: clean.

- [ ] **Step 5: Commit**

  ```bash
  git add tests/http.rs
  git commit -m "test: add ignored network test for DASH proxy (BBB stream)"
  ```
