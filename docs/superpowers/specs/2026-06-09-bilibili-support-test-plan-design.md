# Bilibili VOD Support — Test Plan Design

## Goal

Verify that yt-dlp can fetch streams and Chinese-language transcripts from Bilibili VOD, and that MyTV can discover, add, health-check, and play a Bilibili video as a VOD playlist item.

## Test Fixture

```
https://www.bilibili.com/video/BV1GpEs6gEAA/
```

## Phase 1 — yt-dlp CLI

Run these four commands in order. Each step validates a capability required by the next.

| Step | Command | Pass Criteria |
|------|---------|---------------|
| 1.1 | `yt-dlp --list-formats https://www.bilibili.com/video/BV1GpEs6gEAA/` | At least one DASH or HLS format is listed |
| 1.2 | `yt-dlp --list-subs https://www.bilibili.com/video/BV1GpEs6gEAA/` | `zh-Hans` or `zh-Hant` subtitles are listed |
| 1.3 | `yt-dlp -g https://www.bilibili.com/video/BV1GpEs6gEAA/` | Prints a stream URL without error; note the format (DASH `.mpd` or HLS `.m3u8`) — this determines Phase 2 expectations |
| 1.4 | `python3 ~/.claude/skills/youtube-watcher/scripts/get_transcript.py --lang zh-Hans https://www.bilibili.com/video/BV1GpEs6gEAA/` | Prints readable Chinese text (**requires the `--lang` patch to `get_transcript.py`**) |

## Phase 2 — MyTV Discovery and Playback

Prerequisites: local MyTV server running (`cargo run`), Phase 1 steps 1.1–1.3 passed.

| Step | Action | Pass Criteria |
|------|--------|---------------|
| 2.1 | Admin → Discover → Manual URL → paste fixture URL → Resolve | Resolves to the same stream URL as step 1.3 |
| 2.2 | Add resolved item to a new VOD channel | Item appears in channel detail with title and duration |
| 2.3 | Click Test button on the item | Health badge turns green ●; budget badge shows ⚡ or ☁ |
| 2.4 | Guide → navigate to the VOD channel → click to play | Video plays in the player without error |

**Skip condition for 2.4:** if step 1.3 returned a format other than DASH or HLS, document it as "format not supported by MyTV player" and skip playback.

## Patch Required

Step 1.4 will fail until `get_transcript.py` is patched to accept a `--lang` argument (default `en`). The patch is a prerequisite for Phase 1 to pass completely; Phases 1.1–1.3 and all of Phase 2 can be run against the unpatched script.
