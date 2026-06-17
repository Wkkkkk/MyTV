# Progress site builder

Generates the weekly-progress GitHub Pages site from `docs/progress/` cards.

## Build locally
    python3 scripts/progress_site/build.py   # writes ./site
    open site/index.html

## Test
    python3 scripts/progress_site/build_test.py -v

## Deploy
Automatic: `.github/workflows/pages.yml` rebuilds and deploys on every push to
`main` that touches `docs/progress/**`, the architecture diagram, or this folder.
Manual: run the **Pages** workflow via *Actions → Pages → Run workflow*.

## One-time setup (cannot be committed)
1. Repo **Settings → Pages → Source = GitHub Actions**.
2. Site URL is `https://wkkkkk.github.io/MyTV/`; all in-site paths are relative.

## How it works
`build.py` discovers `<date>-week-card.html` files, extracts headline / date
range / commit count from each card's HTML (`.title`, `.kicker`, `.statblock .big`),
renders `index.html` + one reflow detail page per week from the `*.html` templates,
and assembles `site/` (index, `cards/`, `week/`, favicon, architecture diagram).
Hero & recent weeks embed the card as a scaled 4:3 poster; detail pages embed it
in a narrow column so the card's own `@media(max-width:1000px)` reflow makes it
readable.
