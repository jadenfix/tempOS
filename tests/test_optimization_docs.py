"""Tests for scripts/check-optimization-docs.py."""

from __future__ import annotations

import importlib.util
import sys
import unittest
from pathlib import Path

REPO_ROOT = Path(__file__).resolve().parent.parent
_SCRIPT = REPO_ROOT / "scripts" / "check-optimization-docs.py"

_spec = importlib.util.spec_from_file_location("check_optimization_docs", _SCRIPT)
assert _spec and _spec.loader
check_optimization_docs = importlib.util.module_from_spec(_spec)
sys.modules["check_optimization_docs"] = check_optimization_docs
_spec.loader.exec_module(check_optimization_docs)


class OptimizationDocsTest(unittest.TestCase):
    def test_required_markers_are_present(self) -> None:
        errors: list[str] = []
        for relative_path, markers in check_optimization_docs.REQUIRED_MARKERS.items():
            errors.extend(check_optimization_docs.validate_file(relative_path, markers))
        self.assertEqual(errors, [])


if __name__ == "__main__":
    unittest.main()
