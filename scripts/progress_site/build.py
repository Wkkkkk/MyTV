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
