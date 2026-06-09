# Story: Bilibili Support and a UI Audit — 2026-06-09

## How it started

The session opened with a UI review. Using the `ui-ux-pro-max` skill, we audited the live MyTV app at `kunstv.fly.dev` alongside all the Askama templates in the repo. The audit surfaced a full picture of where the interface stood: a custom dark-theme app with no CSS tokens, a debug panel that overlapped the EPG grid for every user, blue-tinted program blocks inconsistent with the monochrome theme, font sizes as small as 0.7rem, and no focus rings anywhere for keyboard navigation.

Rather than jump straight into fixes, we distilled everything into five new ideas in `docs/IDEAS.md` (ideas 29–33): quick wins, a CSS design-token refactor, keyboard/accessibility polish, player overlay controls, and HTMX loading skeletons. The larger items each got their own idea entry, keeping them visible without blocking other work.

## A new skill arrives

Mid-session, a link appeared: `https://clawhub.ai/michaelgathara/youtube-watcher` — a skill for fetching transcripts from YouTube videos using yt-dlp. We fetched the page, found the install command (`openclaw skills install youtube-watcher`), discovered `openclaw` wasn't on the machine, installed it via npm, ran the install, and copied the skill into `~/.claude/skills/youtube-watcher/` so Claude Code could pick it up.

The skill turned out to be simple and clean: a single Python script that calls `yt-dlp --write-subs --skip-download`, finds the downloaded `.vtt` file in a temp directory, strips timestamps and headers, and prints plain text. Ready to use for YouTube summaries immediately.

## The Bilibili question

Then came a natural question: does yt-dlp support Bilibili? A quick `yt-dlp --list-extractors | grep -i bilibili` answered it — ten dedicated extractors, covering everything from regular videos to live streams to user channel playlists.

The follow-up was just as natural: could the youtube-watcher skill work with Bilibili too? Looking at the script, it passed the URL straight to yt-dlp with no YouTube-specific logic — so technically yes, but the hardcoded `--sub-lang en` would be the problem for Chinese content. A one-line fix was all it needed.

Before writing a single line of code, we brainstormed a test plan.

## Designing the test

The brainstorming session surfaced the right shape quickly: real network calls (not mocks), two phases — yt-dlp CLI first, then MyTV discovery and playback. The fixture URL came from the user: `https://www.bilibili.com/video/BV1GpEs6gEAA/`. The plan was simple enough to fit on one page.

We wrote a spec (`docs/superpowers/specs/2026-06-09-bilibili-support-test-plan-design.md`), reviewed it, committed it, then moved to the implementation plan. The plan had five tasks: baseline verification, copy the skill locally, patch the script, verify, and run MyTV Phase 2.

One important correction before execution: rather than patching the global skill at `~/.claude/skills/`, we should copy it into the project and change only the local copy. The global skill stays untouched.

## Reality hits

Execution started with Task 1 — baseline yt-dlp checks against the fixture URL. The first command failed immediately:

```
ERROR: Unable to extract play info
```

The installed yt-dlp was from 2023. After a `pip install -U yt-dlp` update, the next attempt gave a different error:

```
HTTP Error 412: Precondition Failed
```

Bilibili's bot detection. Passing `--cookies-from-browser chrome` got through. The formats listed were MP4 over direct HTTPS — not DASH, not HLS — two separate `.m4s` streams for video and audio. And the subtitles? Only `danmaku xml` — the scrolling comment overlay, not actual text subtitles.

The fixture video had no subtitles at all. We needed a different one.

## Finding the right video

The user provided a second URL: `https://www.bilibili.com/video/BV1ceVh6tEe9`. This one had AI-generated subtitles: `ai-zh`, `ai-en`, `ai-es`, `ai-ar`, `ai-pt`. Progress.

But testing the language code revealed another gap: `--sub-lang zh-Hans` returned "no subtitles for the requested languages." Bilibili's AI subtitles use `ai-zh`, not `zh-Hans`. And when we actually downloaded them, the file came back as `.srt`, not `.vtt` — meaning the script's `glob("*.vtt")` would find nothing.

That was three problems the original plan hadn't anticipated: the cookie requirement, the `ai-zh` language code, and the `.srt` format. A fourth emerged from the `.srt` format itself — the existing timestamp-stripping regex matched VTT's dot separator (`00:00:01.000`) but not SRT's comma (`00:00:01,000`).

We updated the plan to reflect all four, switched the fixture URL, dropped MyTV Phase 2 (`.m4s` streams aren't playable in MyTV's HLS/DASH player), and committed the revised plan.

## The fix

Task 2 copied the skill into `.claude/skills/youtube-watcher/` inside the project. The `.claude/` directory was gitignored, so we updated `.gitignore` from `.claude/` to `.claude/*` with a `!.claude/skills/` exception — the right way to un-ignore a subdirectory of an ignored parent.

Task 3 made four targeted edits to `get_transcript.py`:

1. Timestamp regex: `\.` → `[.,]` — handles both VTT and SRT
2. File glob: `*.vtt` → `*.vtt` + `*.srt`
3. Function signature: added `lang="en"` and `cookies_browser=None` parameters
4. Argparse: added `--lang` and `--cookies-from-browser` flags

Everything committed in one clean commit.

## Verification

Task 4 ran the patched script against the Bilibili fixture:

```bash
python3 .claude/skills/youtube-watcher/scripts/get_transcript.py \
  --lang ai-zh --cookies-from-browser chrome \
  https://www.bilibili.com/video/BV1ceVh6tEe9
```

Clean Chinese text came back — a transcript of a short video about Tripo AI, an AI-powered 3D modeling tool. No timestamps, no sequence numbers, just the words.

The YouTube regression check passed too: the Rick Astley classic returned its English lyrics unchanged, confirming the default `en` path was untouched.

## The payoff

To close the loop, we used the skill the way it was meant to be used — paste the URL, ask for a summary. The transcript flowed through and came back as a readable English summary: a sponsored Bilibili short promoting Tripo AI, covering its one-click photo-to-3D-model feature, auto topology, skeleton rigging, and the launch of a Chinese version with a buy-one-get-one promotion.

The youtube-watcher skill, originally YouTube-only, now works for any Bilibili video with AI subtitles. One session, one fixture swap, four code changes.

## What remains

- **Bilibili playback in MyTV** — `.m4s` streams require a muxer or a different extraction approach; logged as a known limitation
- **UI improvements** — ideas 29–33 in `IDEAS.md` are ready to be picked up: quick wins, CSS tokens, accessibility polish, player controls, EPG skeletons
- **Global skill update** — the upstream `youtube-watcher` skill at `~/.claude/skills/` still has the original hardcoded `en` and `.vtt`-only logic; the project-local copy is the patched version
