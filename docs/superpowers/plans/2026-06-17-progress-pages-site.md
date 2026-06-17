# Weekly Progress Pages Site Implementation Plan

> **For agentic workers:** REQUIRED SUB-SKILL: Use superpowers:subagent-driven-development (recommended) or superpowers:executing-plans to implement this plan task-by-task. Steps use checkbox (`- [ ]`) syntax for tracking.

**Goal:** Build a static, self-regenerating GitHub Pages microsite that publishes the weekly editorial progress cards from `docs/progress/`.

**Architecture:** A Python 3 stdlib build script (`scripts/progress_site/build.py`) discovers the cards, extracts display metadata from each card's HTML, renders a designed `index.html` plus one reflow "detail" page per week from HTML templates, and assembles a `site/` tree. A GitHub Action runs the script on push under `docs/progress/**` and deploys `site/` via `actions/deploy-pages`. The local card-generation job is unchanged.

**Tech Stack:** Python 3 (stdlib only — `html.parser`, `re`, `pathlib`, `shutil`, `unittest`), HTML/CSS templates, GitHub Actions (`upload-pages-artifact`, `deploy-pages`).

**Spec:** `docs/superpowers/specs/2026-06-17-progress-pages-site-design.md`

## Global Constraints

- Python build uses **stdlib only** — no `pip install`, no third-party imports.
- The site is served from the **`/MyTV/` project-Pages sub-path** — every asset/iframe/anchor path within the site must be **relative** (no leading `/`).
- **Do not modify** the card-generation job or any existing `docs/progress/*-week-card.html` / `*-week.md` file.
- Cards are machine-generated with stable class names: `.kicker`, `.title`, `.deck`, and `.statblock .big`. The card's responsive switch is `@media (max-width:1000px)`.
- Editorial palette (match in chrome): paper `#f5f3ed`, ink `#1a1a1a`, muted `#6b6358`, accent `#b5341f`, plane `#e9e5db`, backdrop `#2a2a2a`. Fonts: Noto Serif SC (serif), Inter (sans), Oswald (condensed).
- Repo: `github.com/Wkkkkk/MyTV`, default branch `main`. GitHub blob base for doc links: `https://github.com/Wkkkkk/MyTV/blob/main/`.
- All commits end with the project's `Co-Authored-By` trailer.

---

### Task 1: Card discovery + metadata extraction

**Files:**
- Create: `scripts/progress_site/build.py`
- Create: `scripts/progress_site/build_test.py`
- Create: `scripts/progress_site/fixtures/progress/2026-06-15-week-card.html`
- Create: `scripts/progress_site/fixtures/progress/2026-06-08-week-card.html`
- Create: `scripts/progress_site/fixtures/progress/2026-06-01-week-card.html`
- Create: `scripts/progress_site/fixtures/progress/2026-06-01-week.md`

**Interfaces:**
- Produces: `class Week` (dataclass) with fields `date: str`, `range_label: str`, `headline: str`, `deck: str`, `commits: str`, `card_file: str`, `detail_file: str`. `discover(progress_dir: str) -> list[Week]` returns weeks sorted by `date` descending. `extract(html_text: str, classname: str) -> str` returns the cleaned inner text of the first element carrying that class.

- [ ] **Step 1: Create the three fixture card files**

`scripts/progress_site/fixtures/progress/2026-06-15-week-card.html`:
```html
<!DOCTYPE html><html><head><title>Weekly Progress — 2026-06-15</title></head><body>
<div class="kicker">MyTV · Weekly Progress · 09 → 15 Jun 2026</div>
<h1 class="title">Two campaigns shipped,<br>one fire put out.</h1>
<p class="deck">Two structured campaigns landed, a <b>production incident</b> was contained.</p>
<div class="statblock"><div class="big">277<small>commits this week</small></div></div>
</body></html>
```

`scripts/progress_site/fixtures/progress/2026-06-08-week-card.html`:
```html
<!DOCTYPE html><html><head><title>Weekly Progress — 2026-06-08</title></head><body>
<div class="kicker">MyTV · Weekly Progress · 02 → 08 Jun 2026</div>
<h1 class="title">Steady refactors and tests.</h1>
<p class="deck">A quieter week of cleanup.</p>
<div class="statblock"><div class="big">120<small>commits this week</small></div></div>
</body></html>
```

`scripts/progress_site/fixtures/progress/2026-06-01-week-card.html` (no `.title`, no `.deck`, no `.statblock` — exercises fallbacks):
```html
<!DOCTYPE html><html><head><title>Weekly Progress — 2026-06-01</title></head><body>
<div class="kicker">MyTV · Weekly Progress · 26 May → 01 Jun 2026</div>
</body></html>
```

`scripts/progress_site/fixtures/progress/2026-06-01-week.md`:
```markdown
# Fallback headline from markdown

Some body text.
```

- [ ] **Step 2: Write the failing test**

`scripts/progress_site/build_test.py`:
```python
import os
import sys
import unittest

sys.path.insert(0, os.path.dirname(os.path.abspath(__file__)))
import build  # noqa: E402

FIXTURES = os.path.join(os.path.dirname(os.path.abspath(__file__)), "fixtures", "progress")


class DiscoverTest(unittest.TestCase):
    def setUp(self):
        self.weeks = build.discover(FIXTURES)

    def test_sorted_descending(self):
        self.assertEqual([w.date for w in self.weeks],
                         ["2026-06-15", "2026-06-08", "2026-06-01"])

    def test_extracts_headline_and_strips_br(self):
        self.assertEqual(self.weeks[0].headline, "Two campaigns shipped, one fire put out.")

    def test_extracts_range_label_from_kicker(self):
        self.assertEqual(self.weeks[0].range_label, "09 → 15 Jun 2026")

    def test_extracts_commit_count(self):
        self.assertEqual(self.weeks[0].commits, "277")

    def test_card_and_detail_paths(self):
        self.assertEqual(self.weeks[0].card_file, "cards/2026-06-15-week-card.html")
        self.assertEqual(self.weeks[0].detail_file, "week/2026-06-15.html")

    def test_headline_falls_back_to_markdown_heading(self):
        self.assertEqual(self.weeks[2].headline, "Fallback headline from markdown")

    def test_tolerates_missing_deck_and_commits(self):
        self.assertEqual(self.weeks[2].deck, "")
        self.assertEqual(self.weeks[2].commits, "")


if __name__ == "__main__":
    unittest.main()
```

- [ ] **Step 3: Run test to verify it fails**

Run: `python3 scripts/progress_site/build_test.py -v`
Expected: FAIL — `ModuleNotFoundError: No module named 'build'` (build.py not created yet).

- [ ] **Step 4: Write minimal implementation**

`scripts/progress_site/build.py`:
```python
"""Build the weekly-progress GitHub Pages site from docs/progress/ cards."""
import os
import pathlib
import re
from dataclasses import dataclass
from html.parser import HTMLParser

CARD_RE = re.compile(r"^(\d{4}-\d{2}-\d{2})-week-card\.html$")
_VOID = {"br", "img", "hr", "input", "meta", "link", "source", "wbr"}


@dataclass
class Week:
    date: str
    range_label: str
    headline: str
    deck: str
    commits: str
    card_file: str
    detail_file: str


class _ClassText(HTMLParser):
    """Capture cleaned inner text of the first element carrying `target` class."""

    def __init__(self, target):
        super().__init__()
        self.target = target
        self.depth = 0
        self.found = False
        self.parts = []

    def handle_starttag(self, tag, attrs):
        if self.depth == 0:
            if not self.found and self.target in dict(attrs).get("class", "").split():
                self.depth = 1
                self.found = True
            return
        if tag in _VOID:
            if tag == "br":
                self.parts.append(" ")
            return
        self.depth += 1

    def handle_startendtag(self, tag, attrs):
        if self.depth > 0 and tag == "br":
            self.parts.append(" ")

    def handle_endtag(self, tag):
        if self.depth > 0 and tag not in _VOID:
            self.depth -= 1

    def handle_data(self, data):
        if self.depth > 0:
            self.parts.append(data)

    def text(self):
        return re.sub(r"\s+", " ", "".join(self.parts)).strip()


def extract(html_text, classname):
    parser = _ClassText(classname)
    parser.feed(html_text)
    return parser.text()


def _first_heading(md_text):
    for line in md_text.splitlines():
        stripped = line.strip()
        if stripped.startswith("#"):
            return stripped.lstrip("#").strip()
    return ""


def discover(progress_dir):
    progress = pathlib.Path(progress_dir)
    weeks = []
    for name in os.listdir(progress):
        match = CARD_RE.match(name)
        if not match:
            continue
        date = match.group(1)
        card = (progress / name).read_text(encoding="utf-8")

        headline = extract(card, "title")
        if not headline:
            md_path = progress / f"{date}-week.md"
            if md_path.exists():
                headline = _first_heading(md_path.read_text(encoding="utf-8"))
            headline = headline or f"Week ending {date}"

        kicker = extract(card, "kicker")
        range_label = kicker.split("·")[-1].strip() if kicker else date

        commit_match = re.match(r"\d[\d,]*", extract(card, "big"))
        commits = commit_match.group(0).replace(",", "") if commit_match else ""

        weeks.append(Week(
            date=date,
            range_label=range_label,
            headline=headline,
            deck=extract(card, "deck"),
            commits=commits,
            card_file=f"cards/{name}",
            detail_file=f"week/{date}.html",
        ))
    weeks.sort(key=lambda w: w.date, reverse=True)
    return weeks
```

- [ ] **Step 5: Run test to verify it passes**

Run: `python3 scripts/progress_site/build_test.py -v`
Expected: PASS — 7 tests OK.

- [ ] **Step 6: Commit**

```bash
git add scripts/progress_site/build.py scripts/progress_site/build_test.py scripts/progress_site/fixtures
git commit -m "feat(progress-site): card discovery + metadata extraction

Co-Authored-By: Claude Opus 4.8 (1M context) <noreply@anthropic.com>"
```

---

### Task 2: Render index + per-week detail pages

**Files:**
- Create: `scripts/progress_site/template.html`
- Create: `scripts/progress_site/detail-template.html`
- Modify: `scripts/progress_site/build.py` (add `render`, standing-section data, helpers)
- Modify: `scripts/progress_site/build_test.py` (add `RenderTest`)

**Interfaces:**
- Consumes: `Week`, `discover` from Task 1.
- Produces: `render(weeks: list[Week], template_dir: str) -> tuple[str, dict[str, str]]` returning `(index_html, {date: detail_html})`. Module constants `ARCH_DIAGRAM: dict`, `ARCH_DOCS: list[dict]`, `INCIDENTS: list[dict]` (each doc/incident dict has `title`, `blurb`, `href`).

- [ ] **Step 1: Create `scripts/progress_site/template.html`**

```html
<!DOCTYPE html>
<html lang="en">
<head>
<meta charset="UTF-8">
<meta name="viewport" content="width=device-width, initial-scale=1.0">
<title>MyTV — Weekly Progress</title>
<link rel="icon" href="favicon.svg" type="image/svg+xml">
<link href="https://fonts.googleapis.com/css2?family=Noto+Serif+SC:wght@700;900&family=Oswald:wght@500;700&family=Inter:wght@400;600;700&display=swap" rel="stylesheet">
<style>
  :root{--paper:#f5f3ed;--ink:#1a1a1a;--muted:#6b6358;--accent:#b5341f;--plane:#e9e5db;--bg:#2a2a2a;
    --serif:"Noto Serif SC",Georgia,serif;--sans:"Inter",-apple-system,"Segoe UI",sans-serif;--cond:"Oswald","Inter",sans-serif;}
  *{box-sizing:border-box;} body{margin:0;background:var(--bg);font-family:var(--sans);color:var(--ink);}
  .wrap{max-width:900px;margin:0 auto;padding:28px 20px 60px;}
  .band{background:var(--paper);border-radius:5px;padding:24px 28px;margin-bottom:18px;}
  a{color:#9c2a18;} .tag{font-family:var(--cond);font-weight:700;letter-spacing:.22em;text-transform:uppercase;font-size:12px;color:var(--accent);}
  .mast{display:flex;align-items:center;gap:14px;} .mast .logo{width:46px;height:46px;}
  .mast h1{font-family:var(--serif);font-weight:900;font-size:34px;margin:2px 0 0;line-height:1;}
  .about p{font-size:14px;line-height:1.6;margin:8px 0 0;} .about .lead{border-left:5px solid var(--accent);padding-left:14px;}
  .meta{display:flex;gap:20px;flex-wrap:wrap;margin-top:14px;font-size:12.5px;} .meta b{font-family:var(--cond);color:var(--accent);letter-spacing:.05em;}
  .secth{font-family:var(--cond);font-weight:700;text-transform:uppercase;letter-spacing:.16em;font-size:14px;color:var(--accent);margin:0 0 12px;}
  .label{font-family:var(--cond);font-weight:700;text-transform:uppercase;letter-spacing:.14em;font-size:12px;color:var(--accent);margin-bottom:10px;}
  .poster-frame{position:relative;width:100%;aspect-ratio:4/3;overflow:hidden;border:1px solid #d8d2c5;border-radius:3px;box-shadow:0 8px 30px rgba(0,0,0,.18);background:var(--paper);}
  .poster-frame iframe{position:absolute;top:0;left:0;width:2000px;height:1500px;border:0;transform-origin:top left;}
  .recent{display:grid;grid-template-columns:1fr 1fr;gap:16px;}
  .cardlink{display:block;color:inherit;text-decoration:none;} .cardlink .cap{margin-top:8px;}
  .cap .hl{font-family:var(--serif);font-weight:700;font-size:15px;line-height:1.2;} .cap .rg{font-size:11.5px;color:var(--muted);}
  .tl{list-style:none;margin:0;padding:0;} .tl a{display:grid;grid-template-columns:96px 1fr 60px;gap:14px;align-items:baseline;
    padding:11px 0;border-bottom:1px solid #ddd6c8;color:inherit;text-decoration:none;}
  .tl a:hover{background:#efebe1;} .tl .date{font-family:var(--cond);font-weight:700;font-size:12.5px;color:var(--accent);}
  .tl .hl{font-weight:700;font-size:14px;} .tl .hl small{display:block;font-weight:400;color:var(--muted);font-size:12px;margin-top:2px;}
  .tl .ct{font-family:var(--cond);font-size:14px;text-align:right;} .tl .ct small{display:block;font-size:9px;color:var(--muted);letter-spacing:.1em;}
  .diagcard{display:flex;justify-content:space-between;align-items:center;background:var(--ink);color:#fff;border-radius:3px;padding:14px 18px;margin-bottom:12px;text-decoration:none;}
  .diagcard b{font-family:var(--cond);letter-spacing:.04em;color:#fff;} .diagcard small{color:#bdb6a8;}
  .docgrid{display:grid;grid-template-columns:1fr 1fr;gap:6px 20px;}
  .doc{display:block;padding:9px 0;border-bottom:1px solid #e2dccf;font-size:13.5px;color:inherit;text-decoration:none;}
  .doc:hover b{color:var(--accent);} .doc b{color:var(--ink);} .doc small{display:block;color:var(--muted);font-size:11.5px;margin-top:1px;}
  .incl{border-left:4px solid var(--accent);padding-left:13px;margin-bottom:10px;display:block;color:inherit;text-decoration:none;}
  .incl .it{font-weight:700;font-size:14px;} .incl small{display:block;font-weight:400;color:var(--muted);font-size:12px;margin-top:2px;}
  .footer{background:#222;color:#cfc8ba;border-radius:5px;padding:16px 28px;font-size:12.5px;display:flex;justify-content:space-between;flex-wrap:wrap;gap:8px;}
  .footer a{color:#ff9c83;}
  @media(max-width:640px){.recent,.docgrid{grid-template-columns:1fr;}}
</style>
</head>
<body>
<div class="wrap">

  <div class="band mast">
    <img class="logo" src="favicon.svg" alt="MyTV">
    <div><div class="tag">MyTV · build log</div><h1>Weekly Progress</h1></div>
  </div>

  <div class="band about">
    <div class="lead">
      <p><b>MyTV</b> is a single-user IPTV web app — Rust · Axum · SQLite · HTMX. It tunes live channels and VOD playlists, proxies streams, and resolves YouTube/HLS sources, all on one 256&nbsp;MB box.</p>
      <p>Every Friday an automated job reads the week's git history and renders the editorial poster below — what shipped, what broke, what got refactored. Read a poster top-down: headline and commit count, then shipped features, architecture, and any incident.</p>
    </div>
    <div class="meta">
      <span><b>Live</b> <a href="https://kunstv.fly.dev/">kunstv.fly.dev ↗</a></span>
      <span><b>Source</b> <a href="https://github.com/Wkkkkk/MyTV">github.com/Wkkkkk/MyTV ↗</a></span>
      <span><b>Tests</b> 399</span>
      <span><b>Since</b> 2026</span>
    </div>
  </div>

  {{HERO}}

  {{RECENT}}

  <div class="band">
    <div class="secth">The archive — every week</div>
    <ul class="tl">{{TIMELINE}}</ul>
  </div>

  <div class="band">
    <div class="secth">How it's built</div>
    {{ARCHITECTURE}}
  </div>

  <div class="band">
    <div class="secth">Incidents &amp; bug log</div>
    {{INCIDENTS}}
  </div>

  <div class="footer">
    <span>Generated weekly from git history · idea #55</span>
    <span><a href="https://kunstv.fly.dev/">Live app ↗</a> &nbsp; <a href="https://github.com/Wkkkkk/MyTV">GitHub ↗</a></span>
  </div>

</div>
<script>
  // Scale fixed-size poster iframes (2000x1500) to their container width.
  function scalePosters(){
    document.querySelectorAll('.poster-frame').forEach(function(f){
      var ifr = f.querySelector('iframe');
      if (ifr) ifr.style.transform = 'scale(' + (f.clientWidth / 2000) + ')';
    });
  }
  window.addEventListener('load', scalePosters);
  window.addEventListener('resize', scalePosters);
</script>
</body>
</html>
```

- [ ] **Step 2: Create `scripts/progress_site/detail-template.html`**

```html
<!DOCTYPE html>
<html lang="en">
<head>
<meta charset="UTF-8">
<meta name="viewport" content="width=device-width, initial-scale=1.0">
<title>MyTV — {{RANGE_LABEL}}</title>
<link rel="icon" href="../favicon.svg" type="image/svg+xml">
<link href="https://fonts.googleapis.com/css2?family=Noto+Serif+SC:wght@700;900&family=Oswald:wght@500;700&family=Inter:wght@400;600;700&display=swap" rel="stylesheet">
<style>
  :root{--paper:#f5f3ed;--ink:#1a1a1a;--muted:#6b6358;--accent:#b5341f;--bg:#2a2a2a;
    --serif:"Noto Serif SC",Georgia,serif;--sans:"Inter",-apple-system,sans-serif;--cond:"Oswald",sans-serif;}
  *{box-sizing:border-box;} body{margin:0;background:var(--bg);font-family:var(--sans);color:var(--ink);}
  .wrap{max-width:900px;margin:0 auto;padding:24px 20px 60px;}
  .bar{display:flex;justify-content:space-between;align-items:baseline;margin-bottom:14px;}
  .bar a{color:#ff9c83;text-decoration:none;font-family:var(--cond);letter-spacing:.08em;text-transform:uppercase;font-size:12px;}
  .head{background:var(--paper);border-radius:5px;padding:22px 26px;margin-bottom:16px;}
  .head .tag{font-family:var(--cond);font-weight:700;letter-spacing:.2em;text-transform:uppercase;font-size:11px;color:var(--accent);}
  .head h1{font-family:var(--serif);font-weight:900;font-size:30px;line-height:1.05;margin:8px 0;}
  .head p{font-size:14px;line-height:1.55;color:#3a352e;margin:0;}
  .reflow{width:100%;border:0;background:var(--paper);border-radius:5px;display:block;}
</style>
</head>
<body>
<div class="wrap">
  <div class="bar">
    <a href="../index.html">← back to index</a>
    <a href="../{{CARD_FILE}}" target="_blank">view as poster ↗</a>
  </div>
  <div class="head">
    <div class="tag">MyTV · Weekly Progress · {{RANGE_LABEL}}</div>
    <h1>{{HEADLINE}}</h1>
    <p>{{DECK}}</p>
  </div>
  <iframe class="reflow" src="../{{CARD_FILE}}" title="Weekly card — {{RANGE_LABEL}}" scrolling="no"></iframe>
</div>
<script>
  // The card reflows to a single column below 1000px; keep the iframe under that
  // width and size its height to the card's content (same-origin, so readable).
  var ifr = document.querySelector('.reflow');
  function sizeReflow(){
    try { ifr.style.height = ifr.contentWindow.document.body.scrollHeight + 'px'; } catch (e) {}
  }
  ifr.addEventListener('load', sizeReflow);
  window.addEventListener('resize', sizeReflow);
</script>
</body>
</html>
```

- [ ] **Step 3: Write the failing test** — append to `scripts/progress_site/build_test.py`:

```python
TEMPLATE_DIR = os.path.dirname(os.path.abspath(__file__))


class RenderTest(unittest.TestCase):
    def setUp(self):
        self.weeks = build.discover(FIXTURES)
        self.index, self.details = build.render(self.weeks, TEMPLATE_DIR)

    def test_hero_is_newest_week(self):
        self.assertIn("cards/2026-06-15-week-card.html", self.index)
        self.assertIn("Two campaigns shipped, one fire put out.", self.index)

    def test_no_unfilled_placeholders(self):
        for token in ("{{HERO}}", "{{RECENT}}", "{{TIMELINE}}", "{{ARCHITECTURE}}", "{{INCIDENTS}}"):
            self.assertNotIn(token, self.index)

    def test_timeline_lists_all_weeks(self):
        for date in ("2026-06-15", "2026-06-08", "2026-06-01"):
            self.assertIn(f"week/{date}.html", self.index)

    def test_recent_holds_two_weeks(self):
        self.assertEqual(self.index.count('class="poster-frame"'),
                         1 + min(2, len(self.weeks) - 1))  # hero + recent

    def test_standing_sections_present(self):
        self.assertIn("architecture-diagram.html", self.index)
        self.assertIn(build.INCIDENTS[0]["href"], self.index)

    def test_one_detail_page_per_week(self):
        self.assertEqual(set(self.details), {"2026-06-15", "2026-06-08", "2026-06-01"})
        self.assertIn("cards/2026-06-01-week-card.html", self.details["2026-06-01"])
        self.assertNotIn("{{", self.details["2026-06-01"])
```

- [ ] **Step 4: Run test to verify it fails**

Run: `python3 scripts/progress_site/build_test.py -v`
Expected: FAIL — `AttributeError: module 'build' has no attribute 'render'`.

- [ ] **Step 5: Add render + standing-section data to `build.py`**

Append to `scripts/progress_site/build.py`:
```python
GITHUB_BLOB = "https://github.com/Wkkkkk/MyTV/blob/main/"

ARCH_DIAGRAM = {
    "title": "System architecture diagram",
    "blurb": "interactive · architecture-diagram.html",
    "href": "architecture-diagram.html",
}
ARCH_DOCS = [
    {"title": "Tune flow", "blurb": "live → VOD, ended-broadcast conversion",
     "href": GITHUB_BLOB + "docs/architecture/tune-flow.md"},
    {"title": "yt-dlp resolution", "blurb": "capped concurrency, live-status probe",
     "href": GITHUB_BLOB + "docs/architecture/ytdlp-resolution.md"},
    {"title": "Health checker", "blurb": "auto-disable / cooldown loop",
     "href": GITHUB_BLOB + "docs/architecture/health-checker.md"},
    {"title": "Database ER", "blurb": "channels · sources · playlist items",
     "href": GITHUB_BLOB + "docs/architecture/database-er.md"},
    {"title": "Request route map", "blurb": "player · guide · admin · api",
     "href": GITHUB_BLOB + "docs/architecture/request-route-map.md"},
    {"title": "Source status", "blurb": "active / health / live aggregation",
     "href": GITHUB_BLOB + "docs/architecture/source-status.md"},
]
INCIDENTS = [
    {"title": "Live-status badge yt-dlp OOM", "blurb": "2026-06-10 · fan-out OOM'd the 256 MB VM → capped 2-permit semaphore",
     "href": GITHUB_BLOB + "docs/bug-logs/2026-06-10-live-status-badge-ytdlp-oom.md"},
    {"title": "Stream-proxy hop-by-hop headers", "blurb": "2026-06-03 · proxied Connection/Transfer-Encoding broke playback",
     "href": GITHUB_BLOB + "docs/bug-logs/2026-06-03-stream-proxy-hop-by-hop-headers.md"},
]


def _esc(text):
    return (text.replace("&", "&amp;").replace("<", "&lt;").replace(">", "&gt;"))


def _poster(week, klass):
    return (
        f'<a class="cardlink" href="{week.detail_file}">'
        f'<div class="poster-frame"><iframe src="{week.card_file}" '
        f'title="Weekly card — {_esc(week.range_label)}" scrolling="no" loading="lazy"></iframe></div>'
        f'<div class="cap"><div class="hl">{_esc(week.headline)}</div>'
        f'<div class="rg">{_esc(week.range_label)}</div></div></a>'
    )


def _hero(week):
    full = (f'<a href="{week.card_file}" target="_blank">view full card ↗</a>')
    return (
        '<div class="band">'
        f'<div class="label">Latest · {_esc(week.range_label)} &nbsp;·&nbsp; {full}</div>'
        '<a class="cardlink" href="' + week.detail_file + '">'
        f'<div class="poster-frame"><iframe src="{week.card_file}" '
        f'title="Weekly card — {_esc(week.range_label)}" scrolling="no" loading="lazy"></iframe></div></a>'
        '</div>'
    )


def _recent(weeks):
    if not weeks:
        return ""
    cells = "".join(_poster(w, "mini") for w in weeks)
    return f'<div class="band"><div class="label">Recent weeks</div><div class="recent">{cells}</div></div>'


def _timeline_rows(weeks):
    rows = []
    for w in weeks:
        count = (f'{w.commits}<small>commits</small>') if w.commits else "—"
        sub = f'<small>{_esc(w.deck)}</small>' if w.deck else ""
        rows.append(
            f'<a href="{w.detail_file}"><span class="date">{_esc(w.range_label)}</span>'
            f'<span class="hl">{_esc(w.headline)}{sub}</span>'
            f'<span class="ct">{count}</span></a>'
        )
    return "".join(rows)


def _architecture():
    diag = (f'<a class="diagcard" href="{ARCH_DIAGRAM["href"]}">'
            f'<span><b>{ARCH_DIAGRAM["title"]}</b><br><small>{ARCH_DIAGRAM["blurb"]}</small></span>'
            f'<span style="color:#ff9c83;font-size:12px">view ↗</span></a>')
    docs = "".join(
        f'<a class="doc" href="{d["href"]}"><b>{_esc(d["title"])}</b><small>{_esc(d["blurb"])}</small></a>'
        for d in ARCH_DOCS)
    return diag + f'<div class="docgrid">{docs}</div>'


def _incidents():
    return "".join(
        f'<a class="incl" href="{i["href"]}"><div class="it">{_esc(i["title"])}</div>'
        f'<small>{_esc(i["blurb"])}</small></a>'
        for i in INCIDENTS)


def render(weeks, template_dir):
    tdir = pathlib.Path(template_dir)
    index_tpl = (tdir / "template.html").read_text(encoding="utf-8")
    detail_tpl = (tdir / "detail-template.html").read_text(encoding="utf-8")

    hero = _hero(weeks[0]) if weeks else '<div class="band">No weeks yet.</div>'
    index_html = (index_tpl
                  .replace("{{HERO}}", hero)
                  .replace("{{RECENT}}", _recent(weeks[1:3]))
                  .replace("{{TIMELINE}}", _timeline_rows(weeks))
                  .replace("{{ARCHITECTURE}}", _architecture())
                  .replace("{{INCIDENTS}}", _incidents()))

    details = {}
    for w in weeks:
        details[w.date] = (detail_tpl
                           .replace("{{RANGE_LABEL}}", _esc(w.range_label))
                           .replace("{{HEADLINE}}", _esc(w.headline))
                           .replace("{{DECK}}", _esc(w.deck))
                           .replace("{{CARD_FILE}}", w.card_file))
    return index_html, details
```

- [ ] **Step 6: Run test to verify it passes**

Run: `python3 scripts/progress_site/build_test.py -v`
Expected: PASS — all DiscoverTest + RenderTest cases OK.

- [ ] **Step 7: Commit**

```bash
git add scripts/progress_site/build.py scripts/progress_site/build_test.py scripts/progress_site/template.html scripts/progress_site/detail-template.html
git commit -m "feat(progress-site): render index + per-week detail pages

Co-Authored-By: Claude Opus 4.8 (1M context) <noreply@anthropic.com>"
```

---

### Task 3: Assemble the `site/` tree + `main()` entrypoint

**Files:**
- Modify: `scripts/progress_site/build.py` (add `assemble`, `main`, `__main__` guard)
- Modify: `scripts/progress_site/build_test.py` (add `AssembleTest`)
- Create: `scripts/progress_site/fixtures/static/favicon.svg`
- Create: `scripts/progress_site/fixtures/architecture-diagram.html`

**Interfaces:**
- Consumes: `Week`, `render` from Tasks 1–2.
- Produces: `assemble(site_dir, weeks, index_html, detail_pages, progress_dir, static_dir, arch_diagram_src) -> None` writes the deployable tree. `main() -> None` wires real repo paths and prints a summary.

- [ ] **Step 1: Create assemble fixtures**

`scripts/progress_site/fixtures/static/favicon.svg`:
```svg
<svg xmlns="http://www.w3.org/2000/svg" viewBox="0 0 32 32"><rect x="3" y="6" width="26" height="19" rx="6" fill="none" stroke="#e94560" stroke-width="2.6"/></svg>
```

`scripts/progress_site/fixtures/architecture-diagram.html`:
```html
<!DOCTYPE html><html><body><h1>Architecture diagram fixture</h1></body></html>
```

- [ ] **Step 2: Write the failing test** — append to `build_test.py`:

```python
import shutil
import tempfile


class AssembleTest(unittest.TestCase):
    def setUp(self):
        self.weeks = build.discover(FIXTURES)
        index, details = build.render(self.weeks, TEMPLATE_DIR)
        self.site = tempfile.mkdtemp()
        build.assemble(
            self.site, self.weeks, index, details,
            progress_dir=FIXTURES,
            static_dir=os.path.join(TEMPLATE_DIR, "fixtures", "static"),
            arch_diagram_src=os.path.join(TEMPLATE_DIR, "fixtures", "architecture-diagram.html"),
        )

    def tearDown(self):
        shutil.rmtree(self.site, ignore_errors=True)

    def _exists(self, rel):
        return os.path.isfile(os.path.join(self.site, rel))

    def test_index_written(self):
        self.assertTrue(self._exists("index.html"))

    def test_favicon_and_diagram_copied(self):
        self.assertTrue(self._exists("favicon.svg"))
        self.assertTrue(self._exists("architecture-diagram.html"))

    def test_card_and_detail_per_week(self):
        for date in ("2026-06-15", "2026-06-08", "2026-06-01"):
            self.assertTrue(self._exists(f"cards/{date}-week-card.html"))
            self.assertTrue(self._exists(f"week/{date}.html"))

    def test_no_absolute_paths_in_index(self):
        html = open(os.path.join(self.site, "index.html"), encoding="utf-8").read()
        self.assertNotIn('href="/', html)
        self.assertNotIn('src="/', html)
```

- [ ] **Step 3: Run test to verify it fails**

Run: `python3 scripts/progress_site/build_test.py -v`
Expected: FAIL — `AttributeError: module 'build' has no attribute 'assemble'`.

- [ ] **Step 4: Add assemble + main to `build.py`**

Append to `scripts/progress_site/build.py`:
```python
import shutil


def assemble(site_dir, weeks, index_html, detail_pages, progress_dir, static_dir, arch_diagram_src):
    site = pathlib.Path(site_dir)
    if site.exists():
        shutil.rmtree(site)
    (site / "cards").mkdir(parents=True)
    (site / "week").mkdir(parents=True)

    (site / "index.html").write_text(index_html, encoding="utf-8")
    for date, html_text in detail_pages.items():
        (site / "week" / f"{date}.html").write_text(html_text, encoding="utf-8")

    progress = pathlib.Path(progress_dir)
    for week in weeks:
        name = pathlib.Path(week.card_file).name
        shutil.copy(progress / name, site / "cards" / name)

    shutil.copy(pathlib.Path(static_dir) / "favicon.svg", site / "favicon.svg")
    shutil.copy(arch_diagram_src, site / "architecture-diagram.html")


def main():
    root = pathlib.Path(__file__).resolve().parents[2]
    template_dir = pathlib.Path(__file__).resolve().parent
    progress_dir = root / "docs" / "progress"

    weeks = discover(progress_dir)
    index_html, detail_pages = render(weeks, template_dir)
    assemble(
        root / "site", weeks, index_html, detail_pages,
        progress_dir=progress_dir,
        static_dir=root / "static",
        arch_diagram_src=root / "docs" / "architecture" / "architecture-diagram.html",
    )
    print(f"Built site/ with {len(weeks)} week(s)")


if __name__ == "__main__":
    main()
```

- [ ] **Step 5: Run unit tests to verify they pass**

Run: `python3 scripts/progress_site/build_test.py -v`
Expected: PASS — all tests OK.

- [ ] **Step 6: Run the build against the real repo and inspect output**

Run: `python3 scripts/progress_site/build.py && find site -type f | sort`
Expected: prints `Built site/ with 1 week(s)` and lists `site/index.html`, `site/favicon.svg`, `site/architecture-diagram.html`, `site/cards/2026-06-15-week-card.html`, `site/week/2026-06-15.html`.

- [ ] **Step 7: Open the built site and visually confirm**

Run: `open site/index.html`
Expected: masthead + About render in editorial style; the latest poster shows scaled in the hero; archive lists the one week; clicking it opens the reflow detail page; architecture/incident links resolve to GitHub.

- [ ] **Step 8: Ignore the build output and commit**

Add `site/` to `.gitignore` (the artifact is built in CI, never committed):
```bash
grep -qxF 'site/' .gitignore || printf 'site/\n' >> .gitignore
git add scripts/progress_site/build.py scripts/progress_site/build_test.py scripts/progress_site/fixtures .gitignore
git commit -m "feat(progress-site): assemble site/ tree + main entrypoint

Co-Authored-By: Claude Opus 4.8 (1M context) <noreply@anthropic.com>"
```

---

### Task 4: GitHub Action — build & deploy to Pages

**Files:**
- Create: `.github/workflows/pages.yml`

**Interfaces:**
- Consumes: `scripts/progress_site/build.py` (run as `python3 scripts/progress_site/build.py`) and `build_test.py`.

- [ ] **Step 1: Create `.github/workflows/pages.yml`**

```yaml
name: Pages

on:
  push:
    branches: [main]
    paths:
      - "docs/progress/**"
      - "docs/architecture/architecture-diagram.html"
      - "scripts/progress_site/**"
      - ".github/workflows/pages.yml"
  workflow_dispatch:

permissions:
  contents: read
  pages: write
  id-token: write

concurrency:
  group: pages
  cancel-in-progress: true

jobs:
  build-deploy:
    runs-on: ubuntu-latest
    environment:
      name: github-pages
      url: ${{ steps.deploy.outputs.page_url }}
    steps:
      - uses: actions/checkout@v5
      - uses: actions/setup-python@v5
        with:
          python-version: "3.x"
      - name: Test build script
        run: python3 scripts/progress_site/build_test.py -v
      - name: Build site
        run: python3 scripts/progress_site/build.py
      - uses: actions/configure-pages@v5
      - uses: actions/upload-pages-artifact@v3
        with:
          path: site
      - id: deploy
        uses: actions/deploy-pages@v4
```

- [ ] **Step 2: Validate the workflow YAML locally**

Run: `python3 -c "import yaml,sys; yaml.safe_load(open('.github/workflows/pages.yml')); print('yaml ok')"`
Expected: `yaml ok`. (If PyYAML is absent, skip — GitHub validates on push.)

- [ ] **Step 3: Commit**

```bash
git add .github/workflows/pages.yml
git commit -m "ci(progress-site): build & deploy weekly progress site to Pages

Co-Authored-By: Claude Opus 4.8 (1M context) <noreply@anthropic.com>"
```

---

### Task 5: Manual-prerequisites doc + close idea #55

**Files:**
- Create: `scripts/progress_site/README.md`
- Modify: `docs/IDEAS.md` (move #55 from Open toward Done once deployed)

**Interfaces:** none (documentation only).

- [ ] **Step 1: Write `scripts/progress_site/README.md`**

```markdown
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
```

- [ ] **Step 2: Commit the doc**

```bash
git add scripts/progress_site/README.md
git commit -m "docs(progress-site): builder README + manual Pages setup

Co-Authored-By: Claude Opus 4.8 (1M context) <noreply@anthropic.com>"
```

- [ ] **Step 3: After the site is live, record #55 as done**

In `docs/IDEAS.md`, remove the #55 block from `## Open` and bump the `## Done`
count line (and add a CHANGELOG entry if that is the project habit). Commit:
```bash
git add docs/IDEAS.md
git commit -m "docs: close idea #55 — weekly progress Pages site

Co-Authored-By: Claude Opus 4.8 (1M context) <noreply@anthropic.com>"
```

---

## Self-Review

**1. Spec coverage:**
- All-weeks publish, latest + 2 recent inline, full archive → Task 2 (`_hero`, `_recent`, `_timeline_rows`). ✓
- Hybrid poster/reflow embedding → Task 2 templates (poster scale script; detail reflow iframe). ✓
- GitHub Action on `docs/progress/**` + deploy-pages → Task 4. ✓
- Rich chrome (masthead, About, timeline, architecture, incidents, footer) → Task 2 `template.html`. ✓
- Standing sections link to GitHub-rendered docs; diagram embedded locally → Task 2 (`ARCH_DOCS`, `INCIDENTS`, `_architecture`) + Task 3 (copy diagram). ✓
- Python stdlib build, three units (discover/render/assemble) → Tasks 1–3. ✓
- Tests for discover/render/assemble → Tasks 1–3 test steps. ✓
- Relative paths for `/MyTV/` sub-path → Task 3 `test_no_absolute_paths_in_index`; templates use relative `href`/`src`. ✓
- Manual prerequisites documented → Task 5 README. ✓
- Card job unchanged → no task touches it. ✓

**2. Placeholder scan:** No TBD/TODO/"handle edge cases"; every code step shows full code; every command lists expected output. ✓

**3. Type consistency:** `Week` fields (`date`, `range_label`, `headline`, `deck`, `commits`, `card_file`, `detail_file`) are used identically across `discover`, `render`, `assemble`. `render(weeks, template_dir)` and `assemble(site_dir, weeks, index_html, detail_pages, progress_dir, static_dir, arch_diagram_src)` signatures match their call sites in `main()` and tests. Template placeholders `{{HERO}}/{{RECENT}}/{{TIMELINE}}/{{ARCHITECTURE}}/{{INCIDENTS}}` and `{{RANGE_LABEL}}/{{HEADLINE}}/{{DECK}}/{{CARD_FILE}}` match the `.replace(...)` calls in `render`. ✓
