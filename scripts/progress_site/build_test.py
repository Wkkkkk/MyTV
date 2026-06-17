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
