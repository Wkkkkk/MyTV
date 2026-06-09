# Bilibili Support Implementation Plan

> **For agentic workers:** REQUIRED SUB-SKILL: Use superpowers:subagent-driven-development (recommended) or superpowers:executing-plans to implement this plan task-by-task. Steps use checkbox (`- [ ]`) syntax for tracking.

**Goal:** Copy the youtube-watcher skill into this project and patch the local copy's `get_transcript.py` to support Bilibili — adding `--lang`, `--cookies-from-browser` arguments, SRT file support, and a fix to the timestamp-stripping regex.

**Architecture:** Copy `~/.claude/skills/youtube-watcher/` into `.claude/skills/youtube-watcher/` (project-local, not global). Patch the local copy only — the global skill is left untouched. Manual CLI verification follows. MyTV Phase 2 playback is out of scope: Bilibili delivers `.m4s` streams (separate video+audio), not HLS/DASH, which MyTV's player does not support.

**Tech Stack:** Python 3, yt-dlp (with Chrome cookies for Bilibili)

**Spec:** `docs/superpowers/specs/2026-06-09-bilibili-support-test-plan-design.md`

**Test fixture:** `https://www.bilibili.com/video/BV1ceVh6tEe9` (has `ai-zh` AI subtitles)

---

## Findings from baseline (Task 1 — completed)

- Bilibili requires `--cookies-from-browser chrome` to bypass HTTP 412
- Bilibili Chinese subtitle lang code is `ai-zh`, not `zh-Hans` — `zh-Hans` returns "no subtitles"
- Subtitles download as `.srt`, not `.vtt` — the script's `glob("*.vtt")` misses them
- SRT timestamps use comma (`00:00:01,000`) not dot (`00:00:01.000`) — existing regex won't strip them
- Stream format is `.m4s` (direct HTTPS, separate video+audio) — MyTV Phase 2 playback is not feasible

## Files

- Create: `.claude/skills/youtube-watcher/` (copied from `~/.claude/skills/youtube-watcher/`)
- Modify: `.claude/skills/youtube-watcher/scripts/get_transcript.py`

---

### ~~Task 1: Baseline~~ ✓ completed

---

### Task 2: Copy skill into the project

**Files:**
- Create: `.claude/skills/youtube-watcher/`

- [ ] **Step 2.1: Copy the global skill into the project**

```bash
cp -r ~/.claude/skills/youtube-watcher /Users/kunwu/Workspace/playground/MyTV/.claude/skills/youtube-watcher
```

Expected: `.claude/skills/youtube-watcher/` appears with `SKILL.md`, `scripts/get_transcript.py`, `skill-card.md`, and `_meta.json`.

- [ ] **Step 2.2: Confirm the unpatched local script fails on missing `--lang` flag**

```bash
cd /Users/kunwu/Workspace/playground/MyTV && python3 .claude/skills/youtube-watcher/scripts/get_transcript.py --lang ai-zh https://www.bilibili.com/video/BV1ceVh6tEe9
```

Expected: `error: unrecognized arguments: --lang ai-zh`

- [ ] **Step 2.3: Confirm unpatched script finds no subtitles without cookies**

```bash
python3 .claude/skills/youtube-watcher/scripts/get_transcript.py https://www.bilibili.com/video/BV1ceVh6tEe9
```

Expected: yt-dlp exits with HTTP 412 or "Unable to download webpage" — confirms both cookie and lang fixes are needed.

---

### Task 3: Patch the local `get_transcript.py`

**Files:**
- Modify: `.claude/skills/youtube-watcher/scripts/get_transcript.py`

- [ ] **Step 3.1: Extend `clean_vtt` to also handle SRT timestamps**

The current regex only matches VTT timestamps (dot separator). Change it to match both formats:

Change:
```python
    timestamp_pattern = re.compile(r'\d{2}:\d{2}:\d{2}\.\d{3}\s-->\s\d{2}:\d{2}:\d{2}\.\d{3}')
```
To:
```python
    timestamp_pattern = re.compile(r'\d{2}:\d{2}:\d{2}[.,]\d{3}\s-->\s\d{2}:\d{2}:\d{2}[.,]\d{3}')
```

- [ ] **Step 3.2: Look for `.srt` files in addition to `.vtt`**

Change:
```python
        vtt_files = list(temp_path.glob("*.vtt"))
        
        if not vtt_files:
            print("No subtitles found.", file=sys.stderr)
            sys.exit(1)
            
        vtt_file = vtt_files[0]
        
        content = vtt_file.read_text(encoding='utf-8')
```
To:
```python
        sub_files = list(temp_path.glob("*.vtt")) + list(temp_path.glob("*.srt"))

        if not sub_files:
            print("No subtitles found.", file=sys.stderr)
            sys.exit(1)

        sub_file = sub_files[0]

        content = sub_file.read_text(encoding='utf-8')
```

- [ ] **Step 3.3: Update `get_transcript` signature to accept `lang` and `cookies_browser`**

Change:
```python
def get_transcript(url: str):
    with tempfile.TemporaryDirectory() as temp_dir:
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
def get_transcript(url: str, lang: str = "en", cookies_browser: str = None):
    with tempfile.TemporaryDirectory() as temp_dir:
        cmd = [
            "yt-dlp",
            "--write-subs",
            "--write-auto-subs",
            "--skip-download",
            "--sub-lang", lang,
            "--output", "subs",
        ]
        if cookies_browser:
            cmd += ["--cookies-from-browser", cookies_browser]
        cmd.append(url)
```

- [ ] **Step 3.4: Add `--lang` and `--cookies-from-browser` arguments to argparse and wire them up**

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
    parser.add_argument("--lang", default="en", help="Subtitle language code (e.g. en, ai-zh, ai-en)")
    parser.add_argument("--cookies-from-browser", dest="cookies_browser", default=None,
                        help="Browser to extract cookies from (e.g. chrome, safari)")
    args = parser.parse_args()

    get_transcript(args.url, lang=args.lang, cookies_browser=args.cookies_browser)
```

- [ ] **Step 3.5: Commit**

```bash
git add .claude/skills/youtube-watcher/
git commit -m "feat: add project-local youtube-watcher skill with Bilibili support"
```

---

### Task 4: Verify patch

**Files:** none

- [ ] **Step 4.1: Run patched script with `ai-zh` and Chrome cookies on Bilibili fixture**

```bash
python3 .claude/skills/youtube-watcher/scripts/get_transcript.py \
  --lang ai-zh \
  --cookies-from-browser chrome \
  https://www.bilibili.com/video/BV1ceVh6tEe9
```

Expected: readable Chinese text printed to stdout — subtitle lines only, no timestamps or SRT sequence numbers.

- [ ] **Step 4.2: Confirm English default still works for YouTube (regression check)**

```bash
python3 .claude/skills/youtube-watcher/scripts/get_transcript.py https://www.youtube.com/watch?v=dQw4w9WgXcQ
```

Expected: English transcript text printed — confirms default `en` and `.vtt` path are unbroken.

---

### Task 5: Document Bilibili limitations for MyTV

**Files:** none (observation only)

- [ ] **Step 5.1: Note stream format limitation**

Bilibili streams are delivered as `.m4s` (separate video and audio segments over direct HTTPS), not HLS `.m3u8` or DASH `.mpd`. MyTV's player and discovery resolver expect one of those two formats. Attempting to add `BV1ceVh6tEe9` via Admin → Discover → Manual URL will either fail to resolve or produce a URL the player cannot play.

Document this as a known limitation: **Bilibili VOD playback in MyTV requires a future feature to support `.m4s` muxed streams or an alternative extraction approach.**
