# Swedish News Workflow — Design Spec

**Date:** 2026-05-29
**Project:** New standalone GitHub repo (`swedish-news`)

## Overview

A Python script that fetches today's Swedish-language news articles from 8sidor.se, summarizes each in one English sentence via the Claude API, and delivers the result either to the terminal (manual runs) or via email (scheduled daily run at 9 AM).

## Repository Structure

```
swedish-news/
  news.py              # all logic: scrape, summarize, output
  requirements.txt     # anthropic, requests, beautifulsoup4
  .github/
    workflows/
      daily-news.yml   # cron schedule + manual dispatch
```

## Data Flow

```
1. Parse CLI args: --date (default today), --email flag
2. fetch_8sidor(date) → list of (title, url, body)
3. Fetch all article pages in parallel (ThreadPoolExecutor)
4. Single Claude API call: all article bodies → N one-sentence English summaries
5. Format output as bullet list: "• [summary] → [url]"
6a. --email flag → send via Gmail SMTP
6b. no flag      → print to terminal
```

## CLI Interface

```bash
python news.py                        # today, terminal
python news.py --date 2026-05-20      # past date, terminal
python news.py --email                # today, email (used by cron)
python news.py --date 2026-05-20 --email  # past date, email
```

## GitHub Actions

**Scheduled run** (`daily-news.yml`):
- Cron: `0 7 * * *` (9:00 AM Stockholm time — CET is UTC+1, CEST is UTC+2; `0 7` covers summer/CEST; adjust to `0 8` in winter if needed, or use a fixed offset)
- Runs: `python news.py --email`

**Manual dispatch**:
- `workflow_dispatch` with optional `date` input (format: `YYYY-MM-DD`)
- If date input is empty, defaults to today
- Runs: `python news.py [--date DATE] --email`

## Email Format

```
Subject: 8sidor.se — Swedish News [2026-05-29]

• Sweden launches new climate initiative → https://8sidor.se/...
• Heavy rain expected across the country → https://8sidor.se/...
• New record in Swedish exports → https://8sidor.se/...
```

Plain text. Recipient is the same Gmail account used to send.

## Configuration

All secrets passed via environment variables (locally via `.env`, in Actions via GitHub Secrets):

| Variable | Description |
|---|---|
| `ANTHROPIC_API_KEY` | Claude API key |
| `GMAIL_USER` | Gmail address used to send |
| `GMAIL_APP_PASSWORD` | Gmail app password (not account password) |
| `EMAIL_TO` | Recipient address (can equal `GMAIL_USER`) |

## Error Handling

| Scenario | Behaviour |
|---|---|
| No articles found for date | Print/email "No articles found for [date]" and exit 0 |
| Individual article fetch fails | Skip that article, continue with the rest |
| Claude API error | Exit with error message, non-zero exit code |
| Email send fails | Print content to terminal as fallback |

## Extensibility

The scraper is a named function with a consistent return shape:

```python
def fetch_8sidor(date: str) -> list[tuple[str, str, str]]:
    # returns [(title, url, body), ...]
```

Adding a new source later (e.g. SVT, SR) means adding one new function with the same signature and wiring it into the CLI via a `--source` flag. No restructuring required.

## Out of Scope

- Storing historical summaries
- Multiple output destinations simultaneously
- Difficulty filtering or content ranking
- Podcast or video sources (future, once more fluent)
