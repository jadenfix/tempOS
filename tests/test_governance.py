import subprocess
import tempfile
import unittest
from pathlib import Path


ROOT = Path(__file__).resolve().parents[1]
CHECK = ROOT / "scripts" / "check-governance.py"


def ledger_with(row: str) -> str:
    return "\n".join(
        [
            "# Test Ledger",
            "",
            "| PR | Author agent | Reviewer agent | Merger agent | Status |",
            "| --- | --- | --- | --- | --- |",
            row,
            "",
        ]
    )


class GovernanceCheckTests(unittest.TestCase):
    def run_checker(self, text: str) -> subprocess.CompletedProcess[str]:
        with tempfile.TemporaryDirectory() as tmp:
            path = Path(tmp) / "ledger.md"
            path.write_text(text, encoding="utf-8")
            return subprocess.run(
                ["python3", str(CHECK), str(path)],
                cwd=ROOT,
                text=True,
                stdout=subprocess.PIPE,
                stderr=subprocess.PIPE,
                check=False,
            )

    def test_same_non_author_reviewer_and_merger_is_allowed(self):
        result = self.run_checker(
            ledger_with("| #1 | agent/author | agent/reviewer | agent/reviewer | merged |")
        )

        self.assertEqual(result.returncode, 0, result.stderr)
        self.assertIn("reviewer/merger differ from author", result.stdout)

    def test_author_cannot_review_or_merge_own_pr(self):
        result = self.run_checker(
            ledger_with("| #1 | agent/author | agent/author | agent/author | merged |")
        )

        self.assertEqual(result.returncode, 1)
        self.assertIn("Reviewer 'agent/author' is the Author", result.stderr)
        self.assertIn("Merger 'agent/author' is the Author", result.stderr)

    def test_unknown_status_fails_closed(self):
        result = self.run_checker(
            ledger_with("| #1 | agent/author | agent/reviewer | agent/reviewer | done |")
        )

        self.assertEqual(result.returncode, 1)
        self.assertIn("unrecognized status", result.stderr)


if __name__ == "__main__":
    unittest.main()
