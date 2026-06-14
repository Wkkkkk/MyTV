# Self-Hosted MP4 VOD via Public Object Storage — Design

**Date:** 2026-06-14
**Status:** Approved (brainstorming complete; pending plan)

## Goal

Play self-hosted MP4 files — including videos pulled from Bilibili/YouTube — as
MyTV VOD playlist items, streamed **browser → object-storage bucket directly**
(zero Fly egress, no proxy). The feature is two decoupled pieces:

1. **MyTV** gets one tiny, tested change so a direct media-file URL plays without
   the stream proxy.
2. An **independent local tool** (`scripts/vod_upload.py`) owns everything else —
   optional download+mux from a URL, upload to the bucket, and registration via
   MyTV's existing JSON admin API. It is decoupled from the Rust server.

## Background

- A `vod_loop` channel has `playlist_items` (`url` + `duration_secs`). At tune
  time, `tune_vod_at` / `next_vod_at` (`src/routes/player.rs`) call
  `resolver::resolve_url(item.url)` and return a `TuneResponse` whose
  `skip_proxy` field is currently `resolver::needs_resolution(item.url)`.
- `needs_resolution` is true only for `youtube.com` / `youtu.be` / `twitch.tv`.
  So a plain external MP4 URL gets `skip_proxy = false` → the player routes it
  through `/stream-proxy` (CDN → Fly → browser: double egress, plus the proxy's
  body handling on large files). We want such items to play **direct**.
- The player's `_loadSource` (`templates/base.html`) already has a direct-MP4
  branch that uses the unproxied URL for `<video src>` when `skip_proxy` is true.
  A direct `<video src>` plays a cross-origin MP4 **without** needing CORS (CORS
  only matters for hls.js/dash.js and the budget probe) and needs no SSRF proxy
  (the browser fetches, not the server).

### Bilibili spike findings (2026-06-14, yt-dlp 2026.03.17 + Chrome cookies)

- Bilibili is **DASH-only, no combined progressive format**: every yt-dlp format
  is `audio only` or `video only`; a combined-format selector errors.
- Default "best" picks **AV1** video, which is **not reliably browser-playable**
  (Safari especially). We must force **H.264 (avc1) + AAC**.
- yt-dlp downloads the separate streams and **muxes** them with ffmpeg into one
  MP4 in a single command (stream copy, no re-encode).
- This makes Bilibili a purely **local** step that feeds the object-storage
  pipeline; MyTV never touches Bilibili (no cookies on Fly, no Referer proxy, no
  DASH synthesis). Idea #43 (Bilibili *inside* MyTV) stays shelved — this
  sidesteps it.

Validated locally: a 3:13 video downloaded+muxed to an 11 MB MP4 with
`codec_name=h264` + `aac` using the format selector in Part 2.

## Non-goals

- In-app upload UI (uploads happen via the local tool).
- Private/signed buckets (public-read only; out of scope to add credentials +
  presigning to MyTV).
- Bucket provisioning and enabling public access — a **one-time manual setup**
  (R2 enables public access per bucket, not via per-object ACL).
- HLS/DASH served from object storage (those still proxy as today).
- Any change to the live path or the player.

## Part 1 — MyTV: direct-play heuristic

**New pure function** in `src/media/resolver.rs`:

```rust
/// True when the URL is a self-contained media container playable directly via
/// `<video src>` (no manifest, no proxy). Strips query/fragment, case-insensitive.
pub fn is_direct_media_file(url: &str) -> bool {
    let path = url.split(['?', '#']).next().unwrap_or(url).to_ascii_lowercase();
    [".mp4", ".webm", ".m4v", ".mov"].iter().any(|ext| path.ends_with(ext))
}

/// Whether the player should bypass `/stream-proxy` for this URL.
pub fn should_skip_proxy(url: &str) -> bool {
    needs_resolution(url) || is_direct_media_file(url)
}
```

**Wiring:** in `tune_vod_at` and `next_vod_at` (`src/routes/player.rs`), replace
the `skip_proxy` argument `resolver::needs_resolution(&item.url)` with
`resolver::should_skip_proxy(&item.url)`. The live path is unchanged. Manifests
(`.m3u8` / `.mpd`) still proxy because they are not direct media files.

**Why a heuristic, not a flag:** zero migration, no admin UI, automatic for any
bucket file. Safe because a direct media file needs neither CORS nor SSRF
proxying for client-side playback.

**Tests:**
- Unit (`resolver`): `is_direct_media_file` true for `.mp4/.webm/.m4v/.mov`,
  including uppercase extensions and URLs with `?query`/`#frag`; false for
  `.m3u8`, `.mpd`, `.txt`, no-extension, and YouTube watch URLs.
  `should_skip_proxy` true for both a YouTube URL and an `.mp4` URL.
- Integration (`tests/http.rs`): a `vod_loop` channel whose playlist item URL is
  a plain `https://…/x.mp4` returns HTTP 200 with `skip_proxy = true` and the URL
  passed through unchanged.

## Part 2 — Independent tool: `scripts/vod_upload.py`

A standalone Python script (not part of the Rust crate). Drives the existing
public JSON API; nothing in MyTV depends on it.

### Usage

```bash
# A Bilibili / YouTube / any yt-dlp-supported URL — downloads + muxes first:
python3 scripts/vod_upload.py https://www.bilibili.com/video/BV1... --channel 7

# A local file already on disk — skips download:
python3 scripts/vod_upload.py ./movie.mp4 --channel 7 --title "Movie"
```

### Flow

1. **Classify `SOURCE`**: an `http(s)://` URL vs a local file path.
2. **If URL → download+mux** to a temp dir, forcing browser-compatible codecs:
   ```
   yt-dlp [--cookies-from-browser chrome] --no-playlist \
     -f 'bv*[vcodec~=avc1]+ba[ext=m4a]/b[vcodec~=avc1]' \
     -S 'vcodec:h264,acodec:aac' \
     --merge-output-format mp4 -o <tmp>.mp4 SOURCE
   ```
   Default `--title` derived from `yt-dlp --print title` (URL) or the filename.
3. **`ffprobe`** the resulting (or local) file → `duration_secs`; warn if the
   video stream is not `h264` or audio not `aac` (won't play everywhere).
4. **Upload** via `boto3` `upload_file` to the S3-compatible bucket; key
   `{prefix}/{uuid}-{sanitized-name}.mp4`, `ContentType=video/mp4`.
5. **Public URL** = `f"{VOD_PUBLIC_BASE_URL}/{key}"`.
6. **Register**: `POST {MYTV_BASE_URL}/api/admin/channels/{channel}/playlist`
   (HTTP basic auth, password from env), body
   `{"title": …, "url": public_url, "duration_secs": …}`.
   *(channel id is in the path; create DTO is `title` / `url` / `duration_secs` /
   optional `sort_order` — verified against `src/routes/api/playlist.rs`.)*
7. **Print** the created item JSON + public URL; remove the temp download.

### Flags

`SOURCE` (positional), `--channel` (required), `--title` (optional override),
`--key-prefix` (optional), `--cookies-from-browser` (default `chrome`, URL
sources only), `--format-sort`/`--quality` (optional, default best avc1),
`--dry-run` (print planned actions; no upload/register).

### Config (env)

- `AWS_ACCESS_KEY_ID`, `AWS_SECRET_ACCESS_KEY` (boto3 standard)
- `VOD_S3_ENDPOINT` (R2 S3 endpoint), `VOD_S3_BUCKET`, `VOD_PUBLIC_BASE_URL`
  (the bucket's public base — `r2.dev` managed domain or a custom domain)
- `MYTV_BASE_URL`, `MYTV_ADMIN_PASSWORD` (reused from `mytvctl`)

### Operator prerequisites

`python3`, `boto3`, `yt-dlp`, `ffmpeg`/`ffprobe`. The tool checks for these and
fails with a clear message if any are missing.

### Error handling

Missing env/deps, yt-dlp failure (cookie/412/format-unavailable — surface
yt-dlp's stderr), `ffprobe` failure, upload failure, or API non-2xx → clear
stderr message + nonzero exit. Each run uses a unique key (no dedupe); re-running
creates a new item — acceptable for a personal tool.

## Retention (rolling window) — added 2026-06-14

The intended library is **ephemeral**: videos under ~30 min, up to ~10/day, kept
for at most a week (~300 MB/video → ~20 GB steady state). Retention is handled in
two halves:

- **Storage side — provider lifecycle rule.** A one-time R2 **object lifecycle
  rule** ("delete objects older than 7 days") expires files automatically, with
  no cron or script. This is the entire retention mechanism on the storage side
  and is set up manually alongside the bucket (consistent with the manual
  public-access setup already noted as out of scope to automate).
- **MyTV side — tolerate 404s (no code).** A `playlist_item` row outlives its
  deleted file and ends up pointing at a dead URL. The player already handles
  this: the direct-`<video src>` branch in `templates/base.html` wires
  `video.onerror` → `GET /channel/:id/next`, and `next_vod_at`
  (`src/routes/player.rs`) returns the next item by playlist position. So a dead
  item **auto-advances** instead of freezing the channel. Worst case, if several
  consecutive items expired together, the player chains `onerror → /next` until
  it lands on a live one — acceptable for a personal rolling library. Stale rows
  accumulate in the guide and are pruned by hand occasionally (e.g.
  `mytvctl playlist delete`) if they become noticeable. No retention sweeper, no
  `created_at` migration, no new server code.

This keeps the rolling window fully automatic on the storage side and free on
the MyTV side, at the cost of transient stale guide entries.

## Data flow at play time

```
tune → item.url = https://<bucket>/<key>.mp4
     → resolve_url passes it through (not YouTube/Twitch)
     → should_skip_proxy(url) == true   (direct media file)
     → player sets <video src> to the bucket URL directly
     → browser streams from R2 with Range requests (seeking works)
     → zero Fly egress
```

## Testing strategy

- **MyTV (Rust):** unit + integration tests above; `cargo fmt`/`clippy`/`test`
  all green before merge (CI gate).
- **Tool (Python):** primary validation is the spike (commands proven on
  2026-06-14) plus a manual end-to-end run. Keep the script thin by factoring
  pure helpers (`is_url`, key generation, format-selector string, API-payload
  builder) so they *could* be pytest-checked; the project's automated-test
  culture is Rust-side, so the tool relies on documented, verified commands +
  `--dry-run` rather than a CI suite.

## Risks / open items

- **Cookie expiry / premium gating:** high-res Bilibili formats may be
  login-gated; the tool surfaces yt-dlp's stderr so failures are legible.
- **Public bucket setup is manual** and out of scope to automate.
- **Quality default** is best avc1 (Bilibili offers avc1 up to 1080p); H.264 is
  larger than the AV1/HEVC variants at equal resolution — an accepted trade for
  compatibility.

## Considered and rejected alternatives

- **catbox.moe / Litterbox (2026-06-14).** A free, donation-funded file host.
  *Catbox* (permanent, ~200 MB/file) and *Litterbox* (temporary, up to 1 GB,
  expiry options 1h/12h/1day/**3 days max**). Attractive for zero-setup uploads
  (a single multipart POST, no credentials or bucket). **Rejected as the backing
  store** for three reasons:
  1. **Retention mismatch** — Catbox is permanent (defeats the rolling window);
     Litterbox caps at **3 days**, so it cannot honor the ~7-day window at all.
  2. **Wrong tool for streaming** — a `vod_loop` re-fetches the file on every
     loop, which is exactly the "use as a CDN / hotlinking" pattern Catbox's FAQ
     prohibits; Litterbox is built for one-off shares, not repeated playback off
     a free single-operator service with no SLA.
  3. **No S3 API** — would need a separate uploader path in `vod_upload.py`,
     diverging from the boto3 flow.
  Its one legitimate use is a **zero-setup escape hatch**: Litterbox (3-day
  window, accepting the streaming caveat) can prove the end-to-end pipeline
  before an R2 bucket exists. Not the design's storage layer.
- **Backblaze B2 + Cloudflare CDN** — supports lifecycle rules, but free egress
  requires fronting it with Cloudflare, i.e. two services to wire for no
  advantage over R2.
- **Bunny.net Storage + CDN** — cheapest dedicated video delivery, but **no
  native object-TTL**, so weekly deletion would become a maintained cron/script
  — against the zero-maintenance priority.
- **Fly volume (self-hosted serving)** — adds Fly egress, makes the 256 MB VM
  serve ~300 MB files, and needs manual volume sizing. Rejected against the
  zero-maintenance priority.

## Rollout

- Part 1 ships as a normal MyTV change (PR, tests, fmt/clippy).
- Part 2 lands under `scripts/` with usage notes + documented prerequisites and
  env vars; independent of the server binary and its deploy.
