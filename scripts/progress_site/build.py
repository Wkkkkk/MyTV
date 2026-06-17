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
