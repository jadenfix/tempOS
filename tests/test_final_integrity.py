"""Tests for the final.md integrity guard."""
from __future__ import annotations

import sys
import tempfile
import unittest
from pathlib import Path

REPO_ROOT = Path(__file__).resolve().parent.parent
sys.path.insert(0, str(REPO_ROOT / "tools"))

import final_integrity as fi  # noqa: E402


class ScanTest(unittest.TestCase):
    def test_scan_reports_headings_and_lines(self) -> None:
        snapshot = fi.scan(fi.FINAL_MD)
        self.assertGreater(snapshot["line_count"], 100)
        self.assertIn("# beaterOS Final Plan", snapshot["headings"])
        self.assertEqual(len(snapshot["sha256"]), 64)


class RegressionDetectionTest(unittest.TestCase):
    def _scan_text(self, text: str) -> dict:
        with tempfile.NamedTemporaryFile("w", suffix=".md", delete=False) as handle:
            handle.write(text)
            path = Path(handle.name)
        try:
            return fi.scan(path)
        finally:
            path.unlink()

    def test_removed_heading_is_detected(self) -> None:
        locked = self._scan_text("# A\ncontent\n## B\nmore\n")
        current = self._scan_text("# A\ncontent\n")
        missing = [h for h in locked["headings"] if h not in current["headings"]]
        self.assertEqual(missing, ["## B"])

    def test_growth_is_allowed(self) -> None:
        locked = self._scan_text("# A\n")
        current = self._scan_text("# A\n## B\nnew section\n")
        missing = [h for h in locked["headings"] if h not in current["headings"]]
        self.assertEqual(missing, [])
        self.assertGreaterEqual(current["line_count"], locked["line_count"])

    def test_section_hollowing_is_detected(self) -> None:
        # Total line count is preserved (padding under A), but B is gutted.
        locked = self._scan_text("# A\nx\n## B\nkeep1\nkeep2\nkeep3\n")
        current = self._scan_text("# A\nx\npad1\npad2\npad3\n## B\n")
        self.assertGreaterEqual(current["line_count"], locked["line_count"])
        hollowed = [
            h
            for h, pinned in locked["section_lines"].items()
            if h in current["headings"] and current["section_lines"].get(h, 0) < pinned
        ]
        self.assertIn("## B", hollowed)


class LockConsistencyTest(unittest.TestCase):
    def test_lock_file_matches_or_is_subset_of_current(self) -> None:
        """The committed lock must never be ahead of final.md (no phantom rules)."""
        if not fi.LOCK_FILE.exists():
            self.skipTest("lock file not yet created")
        self.assertEqual(fi.check(), 0)


if __name__ == "__main__":
    unittest.main()
