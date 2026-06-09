# Story: Bilibili Transcript Support — 2026-06-09

## A new skill arrives

The session opened with a new skill: `https://clawhub.ai/michaelgathara/youtube-watcher` — a tool for fetching transcripts from YouTube videos using yt-dlp. We fetched the page, found the install command (`openclaw skills install youtube-watcher`), discovered `openclaw` wasn't on the machine, installed it via npm, ran the install, and copied the skill into `~/.claude/skills/youtube-watcher/` so Claude Code could pick it up.

The skill is simple and clean: a single Python script that calls `yt-dlp --write-subs --skip-download`, finds the downloaded `.vtt` file in a temp directory, strips timestamps and headers, and prints plain text.

## The Bilibili question

A quick check confirmed yt-dlp has ten dedicated Bilibili extractors. The follow-up was just as natural: could the youtube-watcher skill work with Bilibili too? The script passed the URL straight to yt-dlp with no YouTube-specific logic — so technically yes, but the hardcoded `--sub-lang en` would be a problem for Chinese content.

Before writing a single line of code, we brainstormed a test plan.

## Designing the test

Two phases: yt-dlp CLI verification first, then MyTV discovery and playback. The fixture URL came from the user: `https://www.bilibili.com/video/BV1GpEs6gEAA/`. We wrote a spec, reviewed it, committed it, and produced an implementation plan with five tasks: baseline verification, copy the skill locally, patch the script, verify, and run MyTV Phase 2.

One important design decision: rather than patching the global skill at `~/.claude/skills/`, copy it into the project and change only the local copy. The global skill stays untouched.

## Reality hits

Task 1 — baseline yt-dlp checks — failed immediately:

```
ERROR: Unable to extract play info
```

The installed yt-dlp was from 2023. After `pip install -U yt-dlp`, the next attempt gave:

```
HTTP Error 412: Precondition Failed
```

Bilibili's bot detection. Passing `--cookies-from-browser chrome` got through. The formats listed were MP4 over direct HTTPS — not DASH, not HLS — two separate `.m4s` streams. And the subtitles? Only `danmaku xml` — scrolling comment overlay, not text subtitles. The fixture had no usable subtitles at all.

## Finding the right video

The user provided a second URL: `https://www.bilibili.com/video/BV1ceVh6tEe9`. This one had AI-generated subtitles: `ai-zh`, `ai-en`, `ai-es`, `ai-ar`, `ai-pt`.

But `--sub-lang zh-Hans` returned "no subtitles for the requested languages" — Bilibili uses `ai-zh`, not `zh-Hans`. And the downloaded file came back as `.srt`, not `.vtt`, meaning the script's `glob("*.vtt")` would find nothing. That surfaced a fourth problem: the timestamp regex matched VTT's dot separator (`00:00:01.000`) but not SRT's comma (`00:00:01,000`).

Four problems the original plan hadn't anticipated. We updated the plan, switched the fixture, dropped MyTV Phase 2 (`.m4s` streams aren't playable in MyTV's player), and committed the revised plan.

## The fix

The `.claude/` directory was gitignored, so we updated `.gitignore` from `.claude/` to `.claude/*` with a `!.claude/skills/` exception before committing the copied skill.

Four targeted edits to `get_transcript.py`:

1. Timestamp regex: `\.` → `[.,]` — handles both VTT and SRT
2. File glob: `*.vtt` → `*.vtt` + `*.srt`
3. Function signature: added `lang="en"` and `cookies_browser=None` parameters
4. Argparse: added `--lang` and `--cookies-from-browser` flags

## Verification

```bash
python3 .claude/skills/youtube-watcher/scripts/get_transcript.py \
  --lang ai-zh --cookies-from-browser chrome \
  https://www.bilibili.com/video/BV1ceVh6tEe9
```

Clean Chinese text. The YouTube regression check also passed — the default `en` path untouched.

## The payoff

We fed the transcript through the skill for a summary. The video turned out to be a sponsored short promoting Tripo AI — an AI-powered 3D modeling tool that turns photos into printable 3D models with one click.

The youtube-watcher skill, originally YouTube-only, now works for any Bilibili video with AI subtitles. One session, one fixture swap, four code changes.

## What remains

- **Bilibili playback in MyTV** — `.m4s` streams require a muxer or a different extraction approach; logged as a known limitation
- **Global skill** — the upstream `~/.claude/skills/youtube-watcher/` still has the original hardcoded `en` and `.vtt`-only logic; the project-local copy is the patched version
