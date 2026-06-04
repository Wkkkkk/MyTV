# DASH Stream Support

**Date:** 2026-06-04

## Goal

Play MPEG-DASH (`.mpd`) sources in the MyTV player without transcoding. Currently the player uses hls.js + native HLS fallback; DASH requires a JS player library. The stream proxy must rewrite MPD manifest URLs so DASH segments are routed through `/stream-proxy` the same way HLS segments are.

---

## Decisions

| Question | Decision |
|---|---|
| JS player library | dash.js (not Shaka) |
| Format detection on frontend | URL sniff — `.mpd` in URL |
| MPD URL rewriting | Yes, via `quick-xml` streaming rewriter |
| New `SourceKind::Dash` | No — DASH sources stored as `Iptv`, no DB migration needed |
| CORS probe extended to DASH | No — CORS cache starts cold for DASH hosts, `direct` defaults to `false` (always proxy) |

---

## Backend

### `Cargo.toml`

Add `quick-xml = "0.40"`.

### `src/media/mod.rs`

Add `pub mod mpd;`.

### `src/media/mpd.rs`

New module, mirroring `src/media/hls.rs`.

```rust
pub fn rewrite_mpd_urls(xml: &str, base_url: &str, direct: bool) -> String
```

Uses `quick-xml`'s streaming reader/writer: reads XML events, passes them through unchanged except for three rewrite targets:

| Target | What changes |
|---|---|
| `<BaseURL>` text content | If absolute `http(s)://` URL, wrap in `/stream-proxy?url=…` |
| `<SegmentTemplate media="">` attribute | If absolute URL, replace with `/stream-proxy?url=…{original_template}` — dash.js expands `$Number$`/`$RepresentationID$` variables after substitution, so the proxy receives fully-resolved segment URLs |
| `<SegmentTemplate initialization="">` attribute | Same as `media` |
| `<SegmentURL media="">` attribute | If absolute URL, wrap in proxy |

Relative URLs are left untouched — they resolve against the (already proxied) `<BaseURL>`.

When `direct=true`, the function returns `xml` unchanged (future-proofing for when the CORS probe is extended to DASH).

### `src/routes/player.rs` — stream proxy

Extend the `is_playlist` detection to include DASH:

```rust
let is_dash = ct.contains("dash+xml") || url.contains(".mpd");
let is_playlist = is_dash || ct.contains("mpegurl") || url.contains(".m3u8") || url.contains(".m3u");
```

When `is_playlist` is true, a new `is_dash` branch dispatches to `mpd::rewrite_mpd_urls` instead of `hls::rewrite_hls_urls`. The response `Content-Type` is set to `application/dash+xml` for DASH manifests (same as the existing `application/vnd.apple.mpegurl` override for HLS).

---

## Frontend

### `templates/base.html`

**Script tag** — add dash.js from CDN alongside the existing hls.js tag:

```html
<script src="https://cdn.jsdelivr.net/npm/dashjs@4.7.4/dist/dash.all.min.js"></script>
```

**Player state** — add `let dash = null;` alongside the existing `let hls = null;`.

**`_loadSource(url, offset)`** — branch on `.mpd` in the URL:

- **DASH path**: detach hls (`hls.detachMedia()`), create a new `dashjs.MediaPlayer().create()`, initialize it on the `<video>` element with the proxied URL. Set `currentTime = offset` after the `MANIFEST_LOADED` event fires.
- **HLS path**: if a dash player is active, call `dash.reset(); dash = null;` then re-attach hls (`hls.attachMedia(video)`), then existing hls.js logic unchanged.

**DASH failover** — `dash.on(dashjs.MediaPlayer.events.ERROR, fn)`: dash.js fires repeated ERROR events for a single failure, so a `let dashErrorFired = false;` flag (reset at the top of `_loadSource`, set to `true` on first error) gates the failover to one attempt. On first error, fetch `/channel/:id/next` and call `_loadSource` with the new URL. Same shape as the existing HLS fatal error handler.

**Teardown on new tune** — before loading any new source, call `hls.stopLoad()` and `if (dash) { dash.reset(); dash = null; }` to clean state.

Keyboard shortcuts (`space`, `f`, arrow keys) operate on the `<video>` element directly — no changes needed.

---

## Testing

### Unit tests — `src/media/mpd.rs`

Fixture: `tests/fixtures/bbb_30fps.mpd` — fetch once from `https://dash.akamaized.net/akamai/bbb_30fps/bbb_30fps.mpd` and commit. Tests run against real MPD structure (multi-period, `<SegmentTemplate>`, `<BaseURL>`).

| Test | Assertion |
|---|---|
| `rewrite_base_url_absolute` | `<BaseURL>https://cdn.example.com/</BaseURL>` → proxied |
| `rewrite_segment_template_media` | Absolute `media` attr proxied; `$Number$`/`$RepresentationID$` survive intact |
| `rewrite_segment_template_initialization` | Absolute `initialization` attr proxied |
| `rewrite_segment_url_media` | `<SegmentURL media="https://…">` proxied |
| `relative_urls_untouched` | Relative paths unchanged |
| `direct_mode_returns_unchanged` | `direct=true` → output equals input |
| `bbb_fixture_rewrite` | Load `tests/fixtures/bbb_30fps.mpd`, run rewriter, assert all `https://` URLs in output are `/stream-proxy?url=…` prefixed |

### Integration test — `tests/http.rs`

One test marked `#[ignore = "requires network access — run manually"]`:

- `GET /stream-proxy?url=https://dash.akamaized.net/akamai/bbb_30fps/bbb_30fps.mpd`
- Assert: status 200, `Content-Type: application/dash+xml`, body contains `/stream-proxy?url=` rewrites

Same pattern as the existing `#[ignore]` yt-dlp network tests.

---

## Out of scope

- CORS probe extended to DASH hosts (can be added later as a follow-on to the existing health checker)
- `SourceKind::Dash` and DB migration
- Adaptive bitrate or DRM support
