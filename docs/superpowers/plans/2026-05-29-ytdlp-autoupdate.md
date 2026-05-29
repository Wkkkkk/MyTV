# yt-dlp Auto-Update Implementation Plan

> **For agentic workers:** REQUIRED SUB-SKILL: Use superpowers:subagent-driven-development (recommended) or superpowers:executing-plans to implement this plan task-by-task. Steps use checkbox (`- [ ]`) syntax for tracking.

**Goal:** Add a weekly GitHub Actions workflow that opens a PR to bump the pinned yt-dlp version in the Dockerfile whenever a newer version is available on PyPI.

**Architecture:** A single workflow file (`.github/workflows/update-ytdlp.yml`) with four steps: fetch latest version from PyPI JSON API, extract current pinned version from Dockerfile, compare, and if different create a branch + open a PR using the built-in `GITHUB_TOKEN`.

**Tech Stack:** GitHub Actions, bash, `curl`, `jq`, `sed`, `gh` CLI (pre-installed on `ubuntu-latest`)

---

### Task 1: Create the workflow file

**Files:**
- Create: `.github/workflows/update-ytdlp.yml`

- [ ] **Step 1: Create the workflow file**

Create `.github/workflows/update-ytdlp.yml` with this exact content:

```yaml
name: Update yt-dlp

on:
  schedule:
    - cron: '0 9 * * 1'   # Monday 09:00 UTC
  workflow_dispatch:

jobs:
  update:
    name: Check and bump yt-dlp
    runs-on: ubuntu-latest
    permissions:
      contents: write
      pull-requests: write

    steps:
      - uses: actions/checkout@v4

      - name: Get latest yt-dlp version
        id: latest
        run: |
          VERSION=$(curl -fsSL https://pypi.org/pypi/yt-dlp/json | jq -r '.info.version')
          echo "version=$VERSION" >> "$GITHUB_OUTPUT"

      - name: Get pinned yt-dlp version
        id: pinned
        run: |
          VERSION=$(grep -oP 'yt-dlp==\K[^\s]+' Dockerfile)
          echo "version=$VERSION" >> "$GITHUB_OUTPUT"

      - name: Check if update needed
        id: check
        run: |
          if [ "${{ steps.latest.outputs.version }}" = "${{ steps.pinned.outputs.version }}" ]; then
            echo "needed=false" >> "$GITHUB_OUTPUT"
            echo "yt-dlp is already at ${{ steps.pinned.outputs.version }}, nothing to do."
          else
            echo "needed=true" >> "$GITHUB_OUTPUT"
            echo "Update available: ${{ steps.pinned.outputs.version }} → ${{ steps.latest.outputs.version }}"
          fi

      - name: Update Dockerfile
        if: steps.check.outputs.needed == 'true'
        run: |
          sed -i "s/yt-dlp==${{ steps.pinned.outputs.version }}/yt-dlp==${{ steps.latest.outputs.version }}/" Dockerfile

      - name: Create branch and open PR
        if: steps.check.outputs.needed == 'true'
        env:
          GH_TOKEN: ${{ secrets.GITHUB_TOKEN }}
        run: |
          BRANCH="chore/bump-ytdlp-${{ steps.latest.outputs.version }}"
          git config user.name "github-actions[bot]"
          git config user.email "github-actions[bot]@users.noreply.github.com"
          git checkout -b "$BRANCH"
          git add Dockerfile
          git commit -m "chore: bump yt-dlp to ${{ steps.latest.outputs.version }}"
          git push origin "$BRANCH"
          gh pr create \
            --title "chore: bump yt-dlp to ${{ steps.latest.outputs.version }}" \
            --body "Automated weekly update of the pinned yt-dlp version in the Dockerfile.

**Previous version:** \`${{ steps.pinned.outputs.version }}\`
**New version:** \`${{ steps.latest.outputs.version }}\`

[yt-dlp changelog](https://github.com/yt-dlp/yt-dlp/releases)" \
            --base main \
            --head "$BRANCH"
```

- [ ] **Step 2: Validate YAML syntax**

```bash
python3 -c "import yaml; yaml.safe_load(open('.github/workflows/update-ytdlp.yml'))" && echo "YAML valid"
```

Expected: `YAML valid` with no errors.

- [ ] **Step 3: Commit and push**

```bash
git add .github/workflows/update-ytdlp.yml
git commit -m "ci: add weekly yt-dlp auto-update workflow"
git push
```

Expected: pre-push hook passes (fmt, clippy, tests), push succeeds.

---

### Task 2: Verify the workflow runs

- [ ] **Step 1: Trigger the workflow manually**

```bash
gh workflow run update-ytdlp.yml --repo Wkkkkk/MyTV
```

Expected output: `Created workflow_dispatch event for update-ytdlp.yml at main`

- [ ] **Step 2: Watch the run**

```bash
gh run list --workflow=update-ytdlp.yml --repo Wkkkkk/MyTV --limit 1
```

Wait ~30 seconds, then check the status. Run again until status is `completed`.

Expected: `completed  success  Check and bump yt-dlp`

- [ ] **Step 3: Check the outcome**

The current pinned version is `2026.3.17`. If a newer yt-dlp exists on PyPI (likely), a PR will have been opened. Check:

```bash
gh pr list --repo Wkkkkk/MyTV
```

Expected: a PR titled `chore: bump yt-dlp to <new-version>` is listed.

If no newer version exists yet, the run will log `yt-dlp is already at 2026.3.17, nothing to do.` — that is also a success.
