# yt-dlp Auto-Update — Design Spec

**Date:** 2026-05-29
**Scope:** A single GitHub Actions workflow that weekly checks for a new yt-dlp release and opens a PR to bump the pinned version in the Dockerfile.

---

## Problem

The Dockerfile pins `yt-dlp==<version>` in a `RUN pip3 install` command. YouTube rotates internal APIs frequently; yt-dlp releases patches weekly. Without bumping the pin, YouTube resolution silently breaks.

---

## What it does

A scheduled workflow runs every Monday at 09:00 UTC. It:

1. Fetches the latest yt-dlp version from the PyPI JSON API.
2. Extracts the currently pinned version from the Dockerfile.
3. If the versions match → exits with no action.
4. If the latest is newer → updates the Dockerfile, pushes a branch, and opens a PR.

A `workflow_dispatch` trigger allows manual runs at any time.

---

## Workflow file

**Path:** `.github/workflows/update-ytdlp.yml`

**Trigger:**
```yaml
on:
  schedule:
    - cron: '0 9 * * 1'   # Monday 09:00 UTC
  workflow_dispatch:
```

**Permissions:** `contents: write` and `pull-requests: write` (scoped to the job).

**Steps:**

| Step | What it does |
|---|---|
| Checkout | `actions/checkout@v4` |
| Get latest version | `curl https://pypi.org/pypi/yt-dlp/json \| jq -r '.info.version'` |
| Get pinned version | `grep -oP 'yt-dlp==\K[^\s]+' Dockerfile` |
| Compare | If equal, exit 0 with "already up to date" message |
| Update Dockerfile | `sed -i` to replace old pin with new version |
| Create branch | `chore/bump-ytdlp-<new-version>` |
| Push branch | `git push origin <branch>` |
| Open PR | `gh pr create` with title `chore: bump yt-dlp to <version>` and a short body describing the change. Uses built-in `GITHUB_TOKEN`. |

---

## What is not in scope

- Auto-merging the PR (user reviews and merges manually)
- Pinning any other dependency
- Testing whether the new yt-dlp version actually works
