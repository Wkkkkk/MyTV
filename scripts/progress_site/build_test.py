import os
import shutil
import sys
import tempfile
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
        with open(os.path.join(self.site, "index.html"), encoding="utf-8") as fh:
            html = fh.read()
        self.assertNotIn('href="/', html)
        self.assertNotIn('src="/', html)


if __name__ == "__main__":
    unittest.main()
