"""Tests for PR optimization packet validation."""

from __future__ import annotations

import importlib.util
import unittest
from pathlib import Path

SPEC = importlib.util.spec_from_file_location(
    "check_pr_review_packet",
    str(Path(__file__).resolve().parent.parent / "scripts" / "check-pr-review-packet.py"),
)
assert SPEC and SPEC.loader
MODULE = importlib.util.module_from_spec(SPEC)
SPEC.loader.exec_module(MODULE)


def _body(section: str) -> str:
    return f"""## What does this PR do?

Runtime work.

## Type of change

- [ ] Performance, language boundary, compiler/runtime, accelerator, or close-to-metal
- [ ] Docs / process only

## Optimization packet
{section}

## Tests
- [x] unit
"""


class PrReviewPacketTests(unittest.TestCase):
    def test_accepts_complete_checked_packet(self) -> None:
        errors = MODULE.validate_pr_body(
            _body(
                """- [x] Hot path and cold path are named.
- [x] Bottleneck class is identified (contract, algorithm).
- [x] Baseline, target budget, replay command, workload/fixture, and regression gate are included.
- [x] Compiler/runtime/backend versions are recorded.
- [x] Authority boundary, receipt/audit replay, macOS path, fallback, and rollback story are preserved.
- [x] Source links and dates are included."""
            )
        )
        self.assertEqual(errors, [])


    def test_single_checked_item_cannot_satisfy_later_items(self) -> None:
        errors = MODULE.validate_pr_body(
            _body(
                """- [x] Hot path and cold path are named.
- [ ] Bottleneck class is identified.
- [ ] Baseline, target budget, replay command, workload/fixture, and regression gate are included.
- [ ] Compiler/runtime/backend versions are recorded.
- [ ] Authority boundary, receipt/audit replay, macOS path, fallback, and rollback story are preserved.
- [ ] Source links and dates are included."""
            )
        )
        self.assertIn("optimization packet missing checked item: bottleneck class", errors)
        self.assertIn("optimization packet missing checked item: source links/dates", errors)


    def test_incidental_na_does_not_bypass_packet(self) -> None:
        errors = MODULE.validate_pr_body(
            _body(
                """- [x] Hot path and cold path are named.
- [x] Rollback: N/A."""
            )
        )
        self.assertIn("optimization packet missing checked item: bottleneck class", errors)


    def test_explicit_na_bypasses_packet(self) -> None:
        self.assertEqual(MODULE.validate_pr_body(_body("N/A")), [])

    def test_sensitive_change_cannot_use_na(self) -> None:
        body = _body("N/A").replace(
            "- [ ] Performance, language boundary, compiler/runtime, accelerator, or close-to-metal",
            "- [x] Performance, language boundary, compiler/runtime, accelerator, or close-to-metal",
        )
        errors = MODULE.validate_pr_body(body)
        self.assertEqual(errors, ["optimization packet cannot be N/A for performance-sensitive changes"])


    def test_missing_section_fails(self) -> None:
        errors = MODULE.validate_pr_body("## What does this PR do?\nNo packet.\n")
        self.assertEqual(errors, ["missing '## Optimization packet' section"])
