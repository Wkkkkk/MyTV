# Bilibili Support Implementation Plan

> **For agentic workers:** REQUIRED SUB-SKILL: Use superpowers:subagent-driven-development (recommended) or superpowers:executing-plans to implement this plan task-by-task. Steps use checkbox (`- [ ]`) syntax for tracking.

**Goal:** Copy the youtube-watcher skill into this project and patch the local copy's `get_transcript.py` to accept a `--lang` argument so it fetches Chinese subtitles from Bilibili, then verify the full stack via the two-phase test plan.

**Architecture:** Copy `~/.claude/skills/youtube-watcher/` into `.claude/skills/youtube-watcher/` (project-local, not global). Patch the local copy only — the global skill is left untouched. Manual CLI and MyTV verification follow.

**Tech Stack:** Python 3, yt-dlp, MyTV (Rust/Axum, local `cargo run`)

**Spec:** `docs/superpowers/specs/2026-06-09-bilibili-support-test-plan-design.md`

---

## Files

- Create: `.claude/skills/youtube-watcher/` (copied from `~/.claude/skills/youtube-watcher/`)
- Modify: `.claude/skills/youtube-watcher/scripts/get_transcript.py`

---

### Task 1: Baseline — verify yt-dlp supports the fixture URL (no code changes)

**Files:** none

- [ ] **Step 1.1: List available formats**

```bash
yt-dlp --list-formats https://www.bilibili.com/video/BV1GpEs6gEAA/
```

Expected: table of formats including at least one DASH (`.mpd`) or HLS (`.m3u8`) entry. If the command errors with "Sign in to confirm your age" or similar, note it — the video may require a cookie.

- [ ] **Step 1.2: List available subtitles**

```bash
yt-dlp --list-subs https://www.bilibili.com/video/BV1GpEs6gEAA/
```

Expected: output includes a row with `zh-Hans` or `zh-Hant` in the language column.

- [ ] **Step 1.3: Extract stream URL**

```bash
yt-dlp -g https://www.bilibili.com/video/BV1GpEs6gEAA/
```

Expected: one or more URLs printed (Bilibili typically returns separate video and audio DASH URLs). Note the format — used in Phase 2 to set expectations for MyTV playback.

---

### Task 2: Copy skill into the project

**Files:**
- Create: `.claude/skills/youtube-watcher/`

- [ ] **Step 2.1: Copy the global skill into the project**

```bash
cp -r ~/.claude/skills/youtube-watcher /Users/kunwu/Workspace/playground/MyTV/.claude/skills/youtube-watcher
```

Expected: `.claude/skills/youtube-watcher/` appears in the project with `SKILL.md`, `scripts/get_transcript.py`, `skill-card.md`, and `_meta.json`.

- [ ] **Step 2.2: Confirm the unpatched local script fails as expected**

```bash
python3 .claude/skills/youtube-watcher/scripts/get_transcript.py --lang zh-Hans https://www.bilibili.com/video/BV1GpEs6gEAA/
```

Expected: `error: unrecognized arguments: --lang zh-Hans` — confirms the flag does not exist yet.

- [ ] **Step 2.3: Confirm English default finds no subtitles on a Chinese-only video**

```bash
python3 .claude/skills/youtube-watcher/scripts/get_transcript.py https://www.bilibili.com/video/BV1GpEs6gEAA/
```

Expected: `No subtitles found.` printed to stderr — confirms the need for the patch.

---

### Task 3: Patch the local `get_transcript.py`

**Files:**
- Modify: `.claude/skills/youtube-watcher/scripts/get_transcript.py`

- [ ] **Step 3.1: Update `get_transcript` signature to accept `lang`**

Change:
```python
def get_transcript(url: str):
```
To:
```python
def get_transcript(url: str, lang: str = "en"):
```

- [ ] **Step 3.2: Pass `lang` to yt-dlp**

Change:
```python
        cmd = [
            "yt-dlp",
            "--write-subs",
            "--write-auto-subs",
            "--skip-download",
            "--sub-lang", "en",
            "--output", "subs",
            url
        ]
```
To:
```python
        cmd = [
            "yt-dlp",
            "--write-subs",
            "--write-auto-subs",
            "--skip-download",
            "--sub-lang", lang,
            "--output", "subs",
            url
        ]
```

- [ ] **Step 3.3: Add `--lang` argument to argparse and wire it up**

Change:
```python
def main():
    parser = argparse.ArgumentParser(description="Fetch YouTube transcript.")
    parser.add_argument("url", help="YouTube video URL")
    args = parser.parse_args()

    get_transcript(args.url)
```
To:
```python
def main():
    parser = argparse.ArgumentParser(description="Fetch YouTube transcript.")
    parser.add_argument("url", help="YouTube video URL")
    parser.add_argument("--lang", default="en", help="Subtitle language code (e.g. en, zh-Hans, zh-Hant)")
    args = parser.parse_args()

    get_transcript(args.url, lang=args.lang)
```

- [ ] **Step 3.4: Commit**

```bash
git add .claude/skills/youtube-watcher/
git commit -m "feat: add project-local youtube-watcher skill with --lang support"
```

---

### Task 4: Verify patch — Phase 1 step 1.4

**Files:** none

- [ ] **Step 4.1: Run patched script with `zh-Hans`**

```bash
python3 .claude/skills/youtube-watcher/scripts/get_transcript.py --lang zh-Hans https://www.bilibili.com/video/BV1GpEs6gEAA/
```

Expected: readable Chinese text printed to stdout (subtitle lines, no VTT headers or timestamps).

- [ ] **Step 4.2: Confirm English default is unchanged (YouTube regression check)**

```bash
python3 .claude/skills/youtube-watcher/scripts/get_transcript.py https://www.youtube.com/watch?v=dQw4w9WgXcQ
```

Expected: English transcript text printed — confirms the default `en` still works for YouTube.

---

### Task 5: MyTV Phase 2 — discovery and playback

Prerequisites: Tasks 1–4 passed. Start the local server:

```bash
cargo run
```

Server starts on `http://localhost:3000`.

- [ ] **Step 5.1: Create a new VOD channel**

Navigate to `http://localhost:3000/admin/channels/new`. Fill in:
- Name: `Bilibili Test`
- Type: `vod_loop`
- Leave other fields blank

Submit. Note the channel ID from the URL (`/admin/channels/<id>`).

- [ ] **Step 5.2: Resolve the fixture URL via Manual URL discovery**

Navigate to `http://localhost:3000/admin/discover`. Click the **Manual URL** tab. Paste:

```
https://www.bilibili.com/video/BV1GpEs6gEAA/
```

Click **Resolve**.

Expected: a resolved stream URL is displayed, matching the output of step 1.3. If "Could not resolve" appears, check that `yt-dlp` is on `PATH` and reachable by the server process.

- [ ] **Step 5.3: Add the resolved item to the channel**

In the resolution result, select the `Bilibili Test` channel and click **Add to Channel**.

Expected: redirected to the channel detail page; the playlist item appears in the table with a title and duration.

- [ ] **Step 5.4: Test the item's health and budget**

On the channel detail page, click the **Test** button for the new playlist item.

Expected: health badge updates to ● green; budget badge shows ⚡ (direct) or ☁ (proxied).

- [ ] **Step 5.5: Play via the Guide**

Navigate to `http://localhost:3000/guide`. Find the `Bilibili Test` channel row. Click the program block to tune.

Expected: video plays in the player panel without error. If the stream URL from step 1.3 was a DASH URL and playback fails, document the format and mark as "DASH playback issue" rather than a failure of the patch itself.
