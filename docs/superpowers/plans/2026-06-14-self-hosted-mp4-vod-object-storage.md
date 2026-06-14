# Self-Hosted MP4 VOD via Public Object Storage — Implementation Plan

> **For agentic workers:** REQUIRED SUB-SKILL: Use superpowers:subagent-driven-development (recommended) or superpowers:executing-plans to implement this plan task-by-task. Steps use checkbox (`- [ ]`) syntax for tracking.

**Goal:** Play self-hosted MP4 files (including videos downloaded from Bilibili/YouTube) as MyTV VOD playlist items, streamed browser→bucket directly, via a tiny MyTV change plus an independent local upload tool.

**Architecture:** (1) A pure `resolver` helper makes direct media-file URLs (`.mp4`/`.webm`/`.m4v`/`.mov`) bypass `/stream-proxy` at VOD tune time. (2) A standalone Python script downloads+muxes a URL (or takes a local file), uploads to an S3-compatible public bucket, and registers a playlist item through the existing JSON admin API. The two are fully decoupled.

**Tech Stack:** Rust (Axum, SQLx), Python 3 (stdlib + boto3), yt-dlp + ffmpeg (local), Cloudflare R2 (or any S3-compatible public bucket).

**Spec:** `docs/superpowers/specs/2026-06-14-self-hosted-mp4-vod-object-storage-design.md`

---

## Execution order (reordered 2026-06-14)

Build and experiment with the **independent tool first**, defer the MyTV change:

1. **Task 3** — tool: pure helpers + unit tests
2. **Task 4** — tool: download/mux + upload + register CLI
3. **Task 5** — tool: docs
4. *(deferred)* **Task 1** — resolver `should_skip_proxy` helper
5. *(deferred)* **Task 2** — wire direct-play into VOD tune

Task numbers below are unchanged (stable references); only the order changes.
**Caveat:** until Tasks 1–2 land, a registered bucket MP4 still plays through
`/stream-proxy` (works, but not direct/zero-egress). The tool (3–5) is fully
functional on its own — it just uploads + registers.

---

## File Structure

- **Modify** `src/media/resolver.rs` — add `is_direct_media_file` + `should_skip_proxy` (pure fns) + unit tests.
- **Modify** `src/routes/player.rs` — use `should_skip_proxy` in `tune_vod_at` and `next_vod_at`.
- **Modify** `tests/http.rs` — integration test that an `.mp4` VOD item tunes with `skip_proxy=true`.
- **Create** `scripts/vod_upload.py` — the independent tool (pure helpers + I/O + CLI).
- **Create** `scripts/test_vod_upload.py` — stdlib `unittest` tests for the pure helpers.
- **Create** `scripts/README.md` — prerequisites, env vars, usage.

---

## Task 1: Resolver direct-media-file helpers (Rust)

**Files:**
- Modify: `src/media/resolver.rs` (add fns after `needs_resolution`, ~line 230; add tests in the `tests` module)

- [ ] **Step 1: Write the failing unit tests**

Add to the `#[cfg(test)] mod tests` block in `src/media/resolver.rs`:

```rust
    #[test]
    fn test_is_direct_media_file() {
        assert!(is_direct_media_file("https://b.r2.dev/k/movie.mp4"));
        assert!(is_direct_media_file("https://b/x.MP4")); // case-insensitive
        assert!(is_direct_media_file("https://b/x.webm"));
        assert!(is_direct_media_file("https://b/x.m4v"));
        assert!(is_direct_media_file("https://b/x.mov"));
        assert!(is_direct_media_file("https://b/x.mp4?sig=abc&e=1")); // query stripped
        assert!(is_direct_media_file("https://b/x.mp4#t=10")); // fragment stripped
        assert!(!is_direct_media_file("https://b/playlist.m3u8"));
        assert!(!is_direct_media_file("https://b/manifest.mpd"));
        assert!(!is_direct_media_file("https://b/readme.txt"));
        assert!(!is_direct_media_file("https://b/video")); // no extension
        assert!(!is_direct_media_file("https://www.youtube.com/watch?v=abc"));
    }

    #[test]
    fn test_should_skip_proxy() {
        // resolved-via-yt-dlp sources skip the proxy as before…
        assert!(should_skip_proxy("https://www.youtube.com/watch?v=abc"));
        // …and so do direct media files (the new case)
        assert!(should_skip_proxy("https://bucket.r2.dev/k/movie.mp4"));
        // manifests and plain IPTV still proxy
        assert!(!should_skip_proxy("https://example.com/stream.m3u8"));
        assert!(!should_skip_proxy("https://iptv.example.com/channel/1"));
    }
```

- [ ] **Step 2: Run the tests to verify they fail**

Run: `cargo test --lib resolver::tests::test_is_direct_media_file resolver::tests::test_should_skip_proxy`
Expected: FAIL — `cannot find function is_direct_media_file` / `should_skip_proxy`.

- [ ] **Step 3: Implement the helpers**

Add immediately after the `needs_resolution` function in `src/media/resolver.rs`:

```rust
/// True when the URL is a self-contained media container playable directly via
/// the browser's `<video src>` (no manifest, no proxy needed). Strips any query
/// or fragment and is case-insensitive.
pub fn is_direct_media_file(url: &str) -> bool {
    let path = url
        .split(['?', '#'])
        .next()
        .unwrap_or(url)
        .to_ascii_lowercase();
    [".mp4", ".webm", ".m4v", ".mov"]
        .iter()
        .any(|ext| path.ends_with(ext))
}

/// Whether the player should bypass `/stream-proxy` for this URL: either it is
/// resolved via yt-dlp (YouTube/Twitch) or it is a direct media file served
/// from elsewhere (e.g. self-hosted object storage).
pub fn should_skip_proxy(url: &str) -> bool {
    needs_resolution(url) || is_direct_media_file(url)
}
```

- [ ] **Step 4: Run the tests to verify they pass**

Run: `cargo test --lib resolver::tests::test_is_direct_media_file resolver::tests::test_should_skip_proxy`
Expected: PASS (2 tests).

- [ ] **Step 5: Format, lint, commit**

```bash
cargo fmt
cargo clippy --all-targets -- -D warnings
git add src/media/resolver.rs
git commit -m "feat(resolver): is_direct_media_file + should_skip_proxy helpers"
```

---

## Task 2: Direct-play wiring for VOD items (Rust)

**Files:**
- Modify: `src/routes/player.rs` (`tune_vod_at` ~line 266, `next_vod_at` ~line 290)
- Test: `tests/http.rs` (new integration test)

- [ ] **Step 1: Write the failing integration test**

Add to `tests/http.rs` (near the other VOD tune tests, after `test_tune_vod_with_playlist_returns_stream_url`):

```rust
#[tokio::test]
async fn test_tune_vod_mp4_item_skips_proxy() {
    // Channel 4's seeded items are .mp4 (vod.example.com/epN.mp4): direct media
    // files should now play without the stream proxy.
    let response = app().await.oneshot(req("/channel/4/tune")).await.unwrap();
    assert_eq!(response.status(), StatusCode::OK);
    let json = body_json(response).await;
    assert_eq!(json["skip_proxy"].as_bool().unwrap(), true);
}
```

- [ ] **Step 2: Run the test to verify it fails**

Run: `cargo test --test http test_tune_vod_mp4_item_skips_proxy`
Expected: FAIL — `skip_proxy` is currently `false` for `.mp4` items (panics on `assert_eq!`, left `false`).

- [ ] **Step 3: Switch the skip_proxy source at both VOD call sites**

In `src/routes/player.rs`, in `tune_vod_at`, change the `skip_proxy` argument passed to `tune_response`:

```rust
        Ok(url) => Ok(tune_response(
            ch,
            url,
            offset,
            resolver::should_skip_proxy(&item.url),
            None,
            Some(item.id),
        )),
```

And make the identical change in `next_vod_at` (the `offset` there is `0`):

```rust
        Ok(url) => Ok(tune_response(
            ch,
            url,
            0,
            resolver::should_skip_proxy(&item.url),
            None,
            Some(item.id),
        )),
```

(Both previously passed `resolver::needs_resolution(&item.url)`.)

- [ ] **Step 4: Run the test, then the full suite**

Run: `cargo test --test http test_tune_vod_mp4_item_skips_proxy`
Expected: PASS.

Run: `cargo test`
Expected: PASS — no regressions (the existing `test_tune_vod_with_playlist_returns_stream_url` only asserts the URL, not `skip_proxy`).

- [ ] **Step 5: Format, lint, commit**

```bash
cargo fmt
cargo clippy --all-targets -- -D warnings
git add src/routes/player.rs tests/http.rs
git commit -m "feat(player): play direct-media VOD items without the stream proxy"
```

---

## Task 3: Upload tool — pure helpers + unit tests (Python)

**Files:**
- Create: `scripts/vod_upload.py` (module docstring + imports + pure helpers only)
- Test: `scripts/test_vod_upload.py`

- [ ] **Step 1: Write the failing unit tests**

Create `scripts/test_vod_upload.py`:

```python
import unittest

import vod_upload as v


class Helpers(unittest.TestCase):
    def test_is_url(self):
        self.assertTrue(v.is_url("https://x/y.mp4"))
        self.assertTrue(v.is_url("http://x"))
        self.assertFalse(v.is_url("./movie.mp4"))
        self.assertFalse(v.is_url("/tmp/a.mp4"))

    def test_sanitize_filename(self):
        self.assertEqual(v.sanitize_filename("my movie!.mp4"), "my_movie_.mp4")
        self.assertEqual(v.sanitize_filename("/path/to/Ep 1.mp4"), "Ep_1.mp4")

    def test_object_key(self):
        self.assertEqual(v.object_key("vod", "a b.mp4", "ID"), "vod/ID-a_b.mp4")
        self.assertEqual(v.object_key("", "a.mp4", "ID"), "ID-a.mp4")
        self.assertEqual(v.object_key("/p/", "a.mp4", "ID"), "p/ID-a.mp4")

    def test_build_ytdlp_cmd_with_cookies(self):
        cmd = v.build_ytdlp_cmd("URL", "/tmp/o.mp4", "chrome", "vcodec:h264,acodec:aac")
        self.assertEqual(cmd[0], "yt-dlp")
        self.assertIn("--cookies-from-browser", cmd)
        self.assertIn("chrome", cmd)
        self.assertIn("bv*[vcodec~=avc1]+ba[ext=m4a]/b[vcodec~=avc1]", cmd)
        self.assertEqual(cmd[-1], "URL")  # url after the "--" guard

    def test_build_ytdlp_cmd_without_cookies(self):
        cmd = v.build_ytdlp_cmd("URL", "/tmp/o.mp4", None, "vcodec:h264")
        self.assertNotIn("--cookies-from-browser", cmd)

    def test_build_register_url(self):
        self.assertEqual(
            v.build_register_url("https://h.fly.dev/", 7),
            "https://h.fly.dev/api/admin/channels/7/playlist",
        )

    def test_build_payload(self):
        self.assertEqual(
            v.build_payload("T", "U", 5),
            {"title": "T", "url": "U", "duration_secs": 5},
        )

    def test_public_url(self):
        self.assertEqual(
            v.public_url("https://b.r2.dev/", "/k/x.mp4"),
            "https://b.r2.dev/k/x.mp4",
        )


if __name__ == "__main__":
    unittest.main()
```

- [ ] **Step 2: Run the tests to verify they fail**

Run: `cd scripts && python3 -m unittest test_vod_upload -v`
Expected: FAIL — `ModuleNotFoundError: No module named 'vod_upload'`.

- [ ] **Step 3: Create the module with the pure helpers**

Create `scripts/vod_upload.py`:

```python
#!/usr/bin/env python3
"""Upload a local MP4 (or download+mux a video URL) to public object storage and
register it as a MyTV VOD playlist item via the JSON admin API.

Independent of the MyTV server. See scripts/README.md for prerequisites, env
vars, and usage.
"""
import argparse
import base64
import json
import os
import re
import shutil
import subprocess
import sys
import tempfile
import urllib.error
import urllib.request
import uuid

# yt-dlp format selector: a video stream encoded with H.264 (avc1) merged with an
# m4a audio stream, falling back to any avc1 combined format. Forces a
# browser-playable MP4 (Bilibili's default "best" is AV1, which Safari can't play).
AVC1_FORMAT = "bv*[vcodec~=avc1]+ba[ext=m4a]/b[vcodec~=avc1]"


def is_url(source: str) -> bool:
    return source.startswith("http://") or source.startswith("https://")


def sanitize_filename(name: str) -> str:
    return re.sub(r"[^A-Za-z0-9._-]", "_", os.path.basename(name))


def object_key(prefix: str, filename: str, uid: str) -> str:
    safe = sanitize_filename(filename)
    p = prefix.strip("/")
    return f"{p}/{uid}-{safe}" if p else f"{uid}-{safe}"


def build_ytdlp_cmd(url: str, out_path: str, cookies_from_browser, format_sort: str):
    cmd = [
        "yt-dlp",
        "--no-playlist",
        "-f",
        AVC1_FORMAT,
        "-S",
        format_sort,
        "--merge-output-format",
        "mp4",
        "-o",
        out_path,
    ]
    if cookies_from_browser:
        cmd += ["--cookies-from-browser", cookies_from_browser]
    cmd += ["--", url]
    return cmd


def build_register_url(base: str, channel: int) -> str:
    return f"{base.rstrip('/')}/api/admin/channels/{channel}/playlist"


def build_payload(title: str, url: str, duration_secs: int) -> dict:
    return {"title": title, "url": url, "duration_secs": duration_secs}


def public_url(base: str, key: str) -> str:
    return f"{base.rstrip('/')}/{key.lstrip('/')}"
```

- [ ] **Step 4: Run the tests to verify they pass**

Run: `cd scripts && python3 -m unittest test_vod_upload -v`
Expected: PASS (8 tests).

- [ ] **Step 5: Commit**

```bash
git add scripts/vod_upload.py scripts/test_vod_upload.py
git commit -m "feat(scripts): vod_upload pure helpers + unit tests"
```

---

## Task 4: Upload tool — I/O and CLI (Python)

**Files:**
- Modify: `scripts/vod_upload.py` (append I/O functions + `main`)

- [ ] **Step 1: Append the I/O functions and CLI**

Add to the end of `scripts/vod_upload.py`:

```python
def require_env(*names):
    missing = [n for n in names if not os.environ.get(n)]
    if missing:
        sys.exit("error: missing required env vars: " + ", ".join(missing))


def require_tool(name: str):
    if shutil.which(name) is None:
        sys.exit(f"error: required tool not found on PATH: {name}")


def probe_duration(path: str) -> int:
    out = subprocess.run(
        [
            "ffprobe", "-v", "error",
            "-show_entries", "format=duration",
            "-of", "default=nokey=1:noprint_wrappers=1",
            path,
        ],
        capture_output=True,
        text=True,
    )
    if out.returncode != 0:
        sys.exit(f"error: ffprobe failed: {out.stderr.strip()}")
    try:
        return round(float(out.stdout.strip()))
    except ValueError:
        sys.exit(f"error: could not parse ffprobe duration: {out.stdout!r}")


def fetch_title(url: str, cookies) -> str | None:
    cmd = ["yt-dlp", "--no-playlist", "--print", "title"]
    if cookies:
        cmd += ["--cookies-from-browser", cookies]
    cmd += ["--", url]
    out = subprocess.run(cmd, capture_output=True, text=True)
    title = out.stdout.strip()
    return title if out.returncode == 0 and title else None


def download_and_mux(url: str, out_path: str, cookies, format_sort: str):
    cmd = build_ytdlp_cmd(url, out_path, cookies, format_sort)
    print("+ " + " ".join(cmd), file=sys.stderr)
    if subprocess.run(cmd).returncode != 0:
        sys.exit("error: yt-dlp download/mux failed (see output above)")


def upload_to_bucket(path: str, endpoint: str, bucket: str, key: str):
    try:
        import boto3
    except ImportError:
        sys.exit("error: boto3 is required for upload (pip install boto3)")
    client = boto3.client("s3", endpoint_url=endpoint)
    client.upload_file(path, bucket, key, ExtraArgs={"ContentType": "video/mp4"})


def register_item(base: str, channel: int, password: str, payload: dict) -> dict:
    url = build_register_url(base, channel)
    token = base64.b64encode(f"user:{password}".encode()).decode()
    req = urllib.request.Request(
        url,
        data=json.dumps(payload).encode(),
        method="POST",
        headers={
            "Content-Type": "application/json",
            "Authorization": f"Basic {token}",
        },
    )
    try:
        with urllib.request.urlopen(req) as resp:
            return json.loads(resp.read().decode())
    except urllib.error.HTTPError as e:
        sys.exit(f"error: API returned {e.code}: {e.read().decode()[:300]}")
    except urllib.error.URLError as e:
        sys.exit(f"error: could not reach {url}: {e.reason}")


def main():
    p = argparse.ArgumentParser(
        description="Upload/download a video to object storage and register it as a MyTV VOD item."
    )
    p.add_argument("source", help="Local MP4 path OR a video URL (yt-dlp-supported, e.g. Bilibili/YouTube)")
    p.add_argument("--channel", type=int, required=True, help="MyTV channel id to add the item to")
    p.add_argument("--title", help="Item title (default: yt-dlp title for URLs, filename for local files)")
    p.add_argument("--key-prefix", default="vod", help="Object key prefix (default: vod)")
    p.add_argument("--cookies-from-browser", dest="cookies", default="chrome",
                   help="Browser for yt-dlp cookies on URL sources (default: chrome)")
    p.add_argument("--format-sort", default="vcodec:h264,acodec:aac",
                   help="yt-dlp -S sort string (default prefers H.264/AAC)")
    p.add_argument("--dry-run", action="store_true",
                   help="Print planned actions without downloading, uploading, or registering")
    args = p.parse_args()

    require_env("MYTV_BASE_URL", "MYTV_ADMIN_PASSWORD",
                "VOD_S3_ENDPOINT", "VOD_S3_BUCKET", "VOD_PUBLIC_BASE_URL")
    base = os.environ["MYTV_BASE_URL"]
    password = os.environ["MYTV_ADMIN_PASSWORD"]
    endpoint = os.environ["VOD_S3_ENDPOINT"]
    bucket = os.environ["VOD_S3_BUCKET"]
    public_base = os.environ["VOD_PUBLIC_BASE_URL"]

    tmp = None
    if is_url(args.source):
        require_tool("yt-dlp")
        require_tool("ffprobe")
        title = args.title or fetch_title(args.source, args.cookies) or "Untitled"
        filename = sanitize_filename(title) + ".mp4"
        tmp = tempfile.mkdtemp(prefix="vod_upload_")
        local_path = os.path.join(tmp, "video.mp4")
    else:
        require_tool("ffprobe")
        local_path = args.source
        if not os.path.isfile(local_path):
            sys.exit(f"error: file not found: {local_path}")
        title = args.title or os.path.splitext(os.path.basename(local_path))[0]
        filename = os.path.basename(local_path)

    key = object_key(args.key_prefix, filename, uuid.uuid4().hex)
    final_url = public_url(public_base, key)

    if args.dry_run:
        print(json.dumps({
            "dry_run": True,
            "source": args.source,
            "title": title,
            "object_key": key,
            "public_url": final_url,
            "register_url": build_register_url(base, args.channel),
        }, indent=2))
        if tmp:
            shutil.rmtree(tmp, ignore_errors=True)
        return

    try:
        if tmp:
            download_and_mux(args.source, local_path, args.cookies, args.format_sort)
        duration = probe_duration(local_path)
        upload_to_bucket(local_path, endpoint, bucket, key)
        item = register_item(base, args.channel, password,
                             build_payload(title, final_url, duration))
        print(json.dumps({"registered": item, "public_url": final_url}, indent=2))
    finally:
        if tmp:
            shutil.rmtree(tmp, ignore_errors=True)


if __name__ == "__main__":
    main()
```

- [ ] **Step 2: Re-run the unit tests (no regressions from the new code)**

Run: `cd scripts && python3 -m unittest test_vod_upload -v`
Expected: PASS (8 tests) — the module still imports cleanly (boto3 is imported lazily inside `upload_to_bucket`, so it is not needed to import the module).

- [ ] **Step 3: Verify `--dry-run` with a local file (no network/creds needed)**

Run:
```bash
cd /Users/kunwu/Workspace/playground/MyTV
MYTV_BASE_URL=https://kunstv.fly.dev MYTV_ADMIN_PASSWORD=x \
VOD_S3_ENDPOINT=https://acct.r2.cloudflarestorage.com VOD_S3_BUCKET=media \
VOD_PUBLIC_BASE_URL=https://media.example.com \
python3 scripts/vod_upload.py ./README.md --channel 7 --title "Demo" --dry-run
```
Expected: JSON printed with `"dry_run": true`, `"object_key": "vod/<hex>-Demo.mp4"`, `"public_url": "https://media.example.com/vod/<hex>-Demo.mp4"`, `"register_url": "https://kunstv.fly.dev/api/admin/channels/7/playlist"`. No upload, no HTTP call. (Using `./README.md` is fine for a dry run — it never reads the file.)

- [ ] **Step 4: (Manual, optional — requires creds + a public bucket) real end-to-end run**

With the env vars set to real values and `boto3`/`yt-dlp`/`ffmpeg` installed:
```bash
python3 scripts/vod_upload.py https://www.bilibili.com/video/BV1ceVh6tEe9 --channel 7
```
Expected: downloads+muxes an H.264 MP4, uploads it, prints `{"registered": {...}, "public_url": "..."}`. Then in MyTV: tuning channel 7 plays the file directly (network tab shows the request going to the bucket, not `/stream-proxy`). Document the result; if cookies are missing/expired yt-dlp prints HTTP 412 — re-auth in the browser.

- [ ] **Step 5: Commit**

```bash
git add scripts/vod_upload.py
git commit -m "feat(scripts): vod_upload download/mux + upload + register CLI"
```

---

## Task 5: Documentation

**Files:**
- Create: `scripts/README.md`

- [ ] **Step 1: Write the usage doc**

Create `scripts/README.md`:

```markdown
# scripts/vod_upload.py — self-hosted MP4 VOD uploader

Uploads a video to a public object-storage bucket and registers it as a MyTV
VOD playlist item. Accepts a local MP4 **or** a video URL (Bilibili, YouTube,
anything yt-dlp supports) — URLs are downloaded and muxed to a browser-playable
H.264/AAC MP4 first. Independent of the MyTV server.

## Prerequisites

- `python3` with `boto3` (`pip install boto3`)
- `yt-dlp` and `ffmpeg`/`ffprobe` on PATH (only needed for URL sources / duration)
- A **public-read** S3-compatible bucket (e.g. Cloudflare R2). Enabling public
  access is a one-time bucket setup (R2: enable the r2.dev managed domain or a
  custom domain) and is out of scope for this tool.

## Environment variables

| Var | Purpose |
|-----|---------|
| `AWS_ACCESS_KEY_ID` / `AWS_SECRET_ACCESS_KEY` | Bucket credentials (boto3) |
| `VOD_S3_ENDPOINT` | S3 endpoint URL (e.g. `https://<acct>.r2.cloudflarestorage.com`) |
| `VOD_S3_BUCKET` | Bucket name |
| `VOD_PUBLIC_BASE_URL` | Public base URL of the bucket (e.g. `https://media.example.com`) |
| `MYTV_BASE_URL` | MyTV base URL (e.g. `https://kunstv.fly.dev`) |
| `MYTV_ADMIN_PASSWORD` | Admin password (same as `mytvctl`) |

## Usage

```bash
# Download a Bilibili video, mux to H.264 MP4, upload, register on channel 7:
python3 scripts/vod_upload.py https://www.bilibili.com/video/BV1... --channel 7

# Upload a local file:
python3 scripts/vod_upload.py ./movie.mp4 --channel 7 --title "Movie"

# Preview without uploading:
python3 scripts/vod_upload.py ./movie.mp4 --channel 7 --dry-run
```

Flags: `--title`, `--key-prefix` (default `vod`), `--cookies-from-browser`
(default `chrome`, URL sources), `--format-sort` (default `vcodec:h264,acodec:aac`),
`--dry-run`.

## Notes

- Bilibili is DASH-only and serves AV1 by default; the tool forces H.264 so the
  file plays in all browsers (Safari included).
- Cookies for Bilibili come from your local browser via yt-dlp; nothing about
  Bilibili runs on the MyTV server.
- MyTV plays these items directly from the bucket (no `/stream-proxy`), because
  the URL ends in a direct media extension (`resolver::is_direct_media_file`).

## Tests

```bash
cd scripts && python3 -m unittest test_vod_upload -v
```
```

- [ ] **Step 2: Commit**

```bash
git add scripts/README.md
git commit -m "docs(scripts): document vod_upload prerequisites and usage"
```

---

## Self-Review notes (for the implementer)

- **Spec coverage:** Task 1+2 = the MyTV `should_skip_proxy` heuristic + wiring. Task 3+4 = the independent tool (URL download+mux *and* local-file paths, upload, register). Task 5 = prereqs/env docs. The H.264 forcing (spec's critical finding) is encoded in `AVC1_FORMAT` + `--format-sort` default. The public-bucket-setup-is-manual non-goal is documented, not built.
- **Type/name consistency:** helper names (`is_url`, `sanitize_filename`, `object_key`, `build_ytdlp_cmd`, `build_register_url`, `build_payload`, `public_url`) are identical across the test file, the module, and `main`. Rust: `is_direct_media_file` / `should_skip_proxy` match between definition (Task 1), tests (Task 1), and call sites (Task 2).
- **API contract:** `POST /api/admin/channels/{id}/playlist` with `{title, url, duration_secs}` matches `CreatePlaylistItemRequest` in `src/routes/api/playlist.rs`; Basic auth checks the password only (username `user`), matching `check_basic_auth` / `mytvctl`.
```
