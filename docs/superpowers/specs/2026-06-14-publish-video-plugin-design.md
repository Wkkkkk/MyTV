# publish-video — Distributable Plugin / Agent Skill — Design

**Date:** 2026-06-14
**Status:** Approved (brainstorming complete; pending plan)

## Goal

Extract MyTV's `scripts/vod_upload.py` into a **standalone, MyTV-independent,
agent-invocable capability**, packaged as a **distributable Claude Code plugin**
containing one skill (`publish-video`). An agent, in any project, can invoke the
skill on request to take a video — a local file, a direct media URL, a
yt-dlp-supported site URL, or a folder of files — normalize it to a
browser-playable H.264/AAC MP4, upload it to S3-compatible public object
storage, and return the public URL. MyTV becomes merely *one consumer* via an
optional sink.

## Background

- The existing tool (`scripts/vod_upload.py`, on the unmerged
  `feat/vod-upload-tool` branch) already: classifies source as local file vs URL,
  downloads+muxes URLs to H.264/AAC via yt-dlp, probes duration, uploads to an
  S3-compatible bucket via boto3, and registers a MyTV playlist item. It prints a
  JSON result and has 8 unit tests over pure helpers + a `--dry-run`.
- Its generic core ("source → browser-playable MP4 → public URL") is welded to a
  MyTV-specific tail (register a playlist item, `MYTV_*` env). Decoupling that
  tail is what makes the core reusable.
- Target use is **agent invocation**: other agents call this on request. That
  makes a **Claude Code skill** the right interface (a `SKILL.md` an agent reads
  to know when/how to use it), and a **distributable plugin** the right package
  (installable via `/plugin marketplace add` + `/plugin install`, like
  superpowers/agentmemory).

### Decisions locked during brainstorming (2026-06-14)

- **Reuse target:** "video → public URL" core; MyTV registration is an optional
  sink, not the tool's purpose.
- **New input types to support now:** direct media URLs, batch/lists of URLs,
  local directories, already-playable passthrough.
- **Packaging:** distributable plugin (own repo), **built and validated locally
  first** (local-path marketplace), then published to GitHub.
- **Transcode policy:** for non-H.264/AAC local/direct inputs, **default = warn +
  upload as-is**; `--transcode` is an opt-in flag that re-encodes via ffmpeg.
- **Seeding:** the plugin's `publish_video.py` is seeded from the existing
  `vod_upload.py`; the `feat/vod-upload-tool` branch is then dropped. MyTV retains
  only the deferred direct-play change (Tasks 1–2 of the
  `2026-06-14-self-hosted-mp4-vod-object-storage` plan).
- **Location:** a new sibling folder/repo, planned `~/Workspace/playground/publish-video-plugin/`.

## Non-goals

- In-app/upload UI; private/signed buckets (public-read only).
- Bucket provisioning + public-access setup (one-time manual operator step).
- A source-extractor *plugin framework* — the four named input types are built
  directly via a simple dispatch; no abstract registry (YAGNI).
- Auto-installing Python deps from the plugin (documented prereqs + loud failure
  instead).
- HLS/DASH manifest hosting; any change to MyTV's live path or player.

## Architecture — three layers

```
publish-video-plugin/                 # new local git repo (source of truth)
  .claude-plugin/
    plugin.json                       # name, version, description, author
    marketplace.json                  # single-repo marketplace listing this plugin
  skills/publish-video/
    SKILL.md                          # agent-facing: when + how to invoke, how to read result
    REFERENCE.md                      # full flag/env table (keeps SKILL.md lean)
    scripts/
      publish_video.py                # the engine (seeded from vod_upload.py)
      test_publish_video.py           # unittest over pure helpers
  README.md
```

## Layer 1 — Engine (`publish_video.py`)

Self-contained single script (dependency-light, portable). Pipeline:

```
inputs → resolve to job list → acquire each → playability gate → probe duration → upload → sink(s)
```

### Input resolution & classification

Accepts one or more positional `SOURCE` args plus `--from-file LIST` (text file,
one source per line, `#` comments, blank lines ignored). Each source string is
classified by a pure `classify_source(s)`:

1. `os.path.isdir(s)`  → **DIRECTORY**
2. `os.path.isfile(s)` → **LOCAL_FILE**
3. `is_url(s)` and the URL path ends in a progressive media ext
   (`.mp4/.webm/.mov/.m4v`) → **DIRECT_URL**
4. `is_url(s)` (anything else `http(s)`) → **YTDLP_URL**
5. otherwise → error (`not a file, directory, or URL: …`)

**DIRECTORY** expands to one **LOCAL_FILE** job per contained video file
(extension test via pure `is_video_file(name)`; set:
`.mp4 .webm .mov .m4v .mkv .avi`); non-recursive by default, `--recursive`
descends. The full input set flattens to an ordered **job list**; batch =
processing that list sequentially with per-item results.

### Acquisition → local file

- **YTDLP_URL** → `download_and_mux` (existing `AVC1_FORMAT`,
  `-S vcodec:h264,acodec:aac`, `--merge-output-format mp4`, optional
  `--cookies-from-browser`). Already H.264/AAC by construction.
- **DIRECT_URL** → stream-download to a temp file via `urllib` (no extractor).
- **LOCAL_FILE** → used in place (never deleted by the tool).

### Playability gate (already-playable passthrough + transcode policy)

`ffprobe` the acquired file → container + video codec + audio codec. Pure
`is_browser_playable(container, vcodec, acodec)` returns true when container is
`mp4`, video is `h264`, audio is `aac` or absent.

- **Playable** → upload as-is (`passthrough = true`).
- **Not playable** and **no `--transcode`** → **warn on stderr** ("may not play
  in all browsers"), upload as-is (`passthrough = false, transcoded = false`).
- **Not playable** and **`--transcode`** → ffmpeg re-encode to H.264/AAC MP4,
  then upload (`transcoded = true`).

yt-dlp outputs always pass the gate (forced H.264/AAC).

### Duration

`ffprobe` `format=duration` → rounded integer `duration_secs` (existing
`probe_duration`).

### Upload (S3-compatible)

boto3 `upload_file` with `ContentType=video/mp4` to the configured endpoint;
object key `{prefix}/{uuid}-{sanitized-name}.mp4` (default prefix `video`).
Works for any S3-compatible store (R2/B2/S3/Spaces). **Env renamed** to a neutral
namespace:

- `PUBLISH_VIDEO_S3_ENDPOINT`, `PUBLISH_VIDEO_S3_BUCKET`,
  `PUBLISH_VIDEO_PUBLIC_BASE_URL`
- `AWS_ACCESS_KEY_ID`, `AWS_SECRET_ACCESS_KEY` (boto3 standard)

Public URL = `f"{PUBLISH_VIDEO_PUBLIC_BASE_URL}/{key}"`.

### Sinks (the decoupling from MyTV)

- **Default `print`** — emit a JSON envelope to **stdout** (all logs/progress to
  **stderr**, so an agent can parse stdout cleanly):

  ```json
  {
    "ok": 2,
    "failed": 1,
    "results": [
      {"source": "...", "type": "ytdlp_url", "title": "...",
       "public_url": "https://<base>/video/<uuid>-<name>.mp4",
       "object_key": "video/<uuid>-<name>.mp4", "duration_secs": 193,
       "passthrough": false, "transcoded": false},
      {"source": "...", "type": "local_file", "error": "ffprobe failed: ..."}
    ]
  }
  ```

  The envelope is **always** the batch shape (even for a single source) so agents
  parse one stable contract. A failed item carries an `error` string and no
  `public_url`.
- **Optional `--sink mytv --channel N`** — additionally POSTs each successful
  item to `{MYTV_BASE_URL}/api/admin/channels/{N}/playlist` with Basic auth
  (existing `register_item`). Reads `MYTV_BASE_URL` / `MYTV_ADMIN_PASSWORD`
  **only when this sink is selected**; the registered item id is added to that
  result object as `mytv_item`.

### CLI surface

```
publish_video.py [SOURCE ...] [options]

  SOURCE                       yt-dlp URL | direct media URL | local file | local directory (repeatable)
  --from-file FILE             read additional sources, one per line (# comments)
  --recursive                  descend into subdirectories for DIRECTORY sources
  --title TITLE                override title (single-source runs only)
  --key-prefix PREFIX          object key prefix (default: video)
  --cookies-from-browser B     browser for yt-dlp cookies (default: chrome; URL sources)
  --format-sort SORT           yt-dlp -S string (default: vcodec:h264,acodec:aac)
  --transcode                  re-encode non-H.264/AAC inputs to H.264/AAC (default: warn + upload as-is)
  --sink {print,mytv}          output sink (default: print)
  --channel N                  MyTV channel id (required with --sink mytv)
  --dry-run                    print planned actions; no download/upload/register
```

### Exit codes

`0` when every item succeeded; `1` when any item failed (batch still processes
all items, collecting per-item `error`); `2` for usage/config errors (missing
env, missing tool, bad arguments) — fail fast before processing.

## Layer 2 — Skill (`SKILL.md`)

Frontmatter:

- `name: publish-video`
- `description:` "Use when you need to publish a local or remote video — a file,
  a direct media URL, a yt-dlp-supported site URL, or a folder of videos — to a
  public URL. Downloads/normalizes to a browser-playable H.264/AAC MP4, uploads
  to S3-compatible object storage, and returns the public URL. Optionally
  registers it as a MyTV VOD playlist item."

Body (lean; details delegated to `REFERENCE.md`):

- **When to use / not use** (use: need a hosted, browser-playable video URL; not:
  HLS/DASH manifests, private/signed delivery).
- **Prerequisites:** `python3` + `boto3`, `yt-dlp`, `ffmpeg`/`ffprobe`; the script
  fails with a clear message if any are missing.
- **Required env:** the `PUBLISH_VIDEO_S3_*` + `AWS_*` vars (and `MYTV_*` only for
  the `mytv` sink).
- **How to invoke:** the exact command —
  `python3 ${CLAUDE_PLUGIN_ROOT}/skills/publish-video/scripts/publish_video.py <source> [options]`.
- **How to read the result:** parse the JSON envelope on **stdout**; for each
  entry in `results`, read `public_url` (or `error`).
- **One example per input type** + a batch example + a `--dry-run` example.

`REFERENCE.md` holds the full flag table, env table, JSON-output schema, and exit
codes — generated/maintained to match the script.

## Layer 3 — Plugin + marketplace (local-first)

- **`plugin.json`** — `{ "name": "publish-video", "version": "0.1.0",
  "description": "...", "author": {...} }`.
- **`marketplace.json`** — a single-repo marketplace listing one plugin whose
  source is this repo, so `/plugin marketplace add <path-or-repo>` exposes it.

**Build & validate locally, then publish:**

1. Scaffold the repo (layout above); write `plugin.json` + `marketplace.json`.
2. `/plugin marketplace add /abs/path/to/publish-video-plugin` (local path).
3. `/plugin install publish-video`.
4. **End-to-end validation:** an agent invokes the skill — `--dry-run` for each
   input type, then one real run against an R2 bucket; confirm it parses
   `public_url` from stdout.
5. `gh repo create` + push; re-point the marketplace to the remote; re-add and
   re-install to confirm the published path works.

## Seeding & migration

- Seed `skills/publish-video/scripts/publish_video.py` from the current
  `vod_upload.py` (the new repo is the source of truth going forward).
- Drop the `feat/vod-upload-tool` branch once seeded — its work lives on in the
  plugin. MyTV keeps **only** the deferred direct-play change (Tasks 1–2 of the
  object-storage plan), so registered bucket MP4s play without the proxy.

## Testing strategy

- **Engine (unittest, in the plugin repo):** extend the existing pure-helper
  suite — `classify_source`, `is_video_file`, `is_url`, `sanitize_filename`,
  `object_key`, `build_ytdlp_cmd`, `is_browser_playable`, sink-payload builders
  (`build_payload`, `build_register_url`, `public_url`), and the JSON-envelope
  builder. I/O (download, ffprobe, ffmpeg, upload, API) stays behind thin wrappers
  so the pure logic is fully testable. `--dry-run` exercises every input type
  with no tools/network.
- **Skill/plugin:** manual validation per the local-first flow — install locally,
  agent dry-run each input type, one real R2 run. Keeps the project's
  "documented, verified commands + dry-run" culture rather than a CI suite.

## Error handling

- Missing tool / env / bad args → clear stderr message + exit `2`, before any
  processing.
- Per-item failures in a batch are isolated: the item gets an `error` string, the
  run continues, overall exit `1`.
- yt-dlp / ffmpeg / download / upload / MyTV-API errors are surfaced with context
  (e.g. yt-dlp's stderr for cookie/412/format issues).

## Risks / open items

- **Prereq friction:** a distributable plugin can't install python/boto3/yt-dlp/
  ffmpeg; mitigated by documented prereqs + loud failure.
- **Transcode cost:** `--transcode` re-encodes (slow/lossy); off by default, so
  the caller opts in only when broad playback matters.
- **Public bucket:** a leaked object URL is publicly fetchable (accepted
  trade-off, same as the object-storage design).
- **Credentials in env:** the calling agent/operator supplies `AWS_*` /
  `PUBLISH_VIDEO_S3_*`; the plugin never bundles secrets.

## Rollout

- Layers 1–2 land in the new repo; Layer 3 wires the plugin + local marketplace.
- Publish to GitHub only after the local end-to-end validation passes.
- MyTV is untouched except for the separately-planned direct-play Tasks 1–2.
