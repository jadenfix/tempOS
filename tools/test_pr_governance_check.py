#!/usr/bin/env python3
"""Unit tests for the beaterOS PR governance checker (stdlib unittest only).

Run with:  python3 -m unittest tools.test_pr_governance_check
       or:  python3 tools/test_pr_governance_check.py
"""

from __future__ import annotations

import os
import tempfile
import unittest

import pr_governance_check as gov


def _write(root: str, rel: str, text: str) -> None:
    path = os.path.join(root, rel)
    os.makedirs(os.path.dirname(path) or root, exist_ok=True)
    with open(path, "w", encoding="utf-8") as handle:
        handle.write(text)


def _make_healthy_repo(root: str, final_lines: int = 3200) -> None:
    _write(root, "final.md", "\n".join(f"line {i}" for i in range(final_lines)))
    for rel in gov.REQUIRED_PROCESS_FILES:
        _write(root, rel, f"# stub for {rel}\n")


class RepoInvariantTests(unittest.TestCase):
    def test_healthy_repo_has_no_errors(self) -> None:
        with tempfile.TemporaryDirectory() as root:
            _make_healthy_repo(root)
            findings = gov.check_repo_invariants(root)
            self.assertEqual(gov.worst_level(findings), gov.OK)

    def test_missing_final_md_is_error(self) -> None:
        with tempfile.TemporaryDirectory() as root:
            findings = gov.check_repo_invariants(root)
            codes = {f.code for f in findings}
            self.assertIn("final-md-missing", codes)
            self.assertEqual(gov.worst_level(findings), gov.ERROR)

    def test_gutted_final_md_is_error(self) -> None:
        with tempfile.TemporaryDirectory() as root:
            _make_healthy_repo(root, final_lines=10)
            findings = gov.check_repo_invariants(root)
            codes = {f.code for f in findings}
            self.assertIn("final-md-weakened", codes)
            self.assertEqual(gov.worst_level(findings), gov.ERROR)

    def test_missing_process_file_is_warning_not_error(self) -> None:
        with tempfile.TemporaryDirectory() as root:
            _make_healthy_repo(root)
            os.remove(os.path.join(root, "AGENTS.md"))
            findings = gov.check_repo_invariants(root)
            codes = {f.code for f in findings}
            self.assertIn("process-file-missing", codes)
            # Missing coordination doc must not hard-fail an unrelated PR.
            self.assertEqual(gov.worst_level(findings), gov.WARN)

    def test_env_override_of_floor(self) -> None:
        with tempfile.TemporaryDirectory() as root:
            _make_healthy_repo(root, final_lines=200)
            os.environ["FINAL_MD_MIN_LINES"] = "100"
            try:
                findings = gov.check_repo_invariants(root)
            finally:
                del os.environ["FINAL_MD_MIN_LINES"]
            self.assertNotIn("final-md-weakened", {f.code for f in findings})

    def test_invalid_env_floor_falls_back_to_default(self) -> None:
        with tempfile.TemporaryDirectory() as root:
            # 3100 lines is above the default 3000 floor but the env var is junk;
            # the fallback must keep the default and not raise.
            _make_healthy_repo(root, final_lines=3100)
            os.environ["FINAL_MD_MIN_LINES"] = "not-a-number"
            try:
                findings = gov.check_repo_invariants(root)
            finally:
                del os.environ["FINAL_MD_MIN_LINES"]
            self.assertNotIn("final-md-weakened", {f.code for f in findings})
            self.assertEqual(gov.worst_level(findings), gov.OK)


class PrBodyTests(unittest.TestCase):
    GOOD_BODY = (
        "## Review routing\n"
        "- [x] Reviewed by an agent/person who did not author the PR.\n"
        "- [ ] Merge performed by an agent/person who did not author the PR.\n"
    )

    def test_good_body_passes(self) -> None:
        findings = gov.check_pr_body(self.GOOD_BODY)
        self.assertEqual(gov.worst_level(findings), gov.OK)

    def test_missing_routing_language_is_warning(self) -> None:
        findings = gov.check_pr_body("no routing here")
        self.assertIn("routing-section-incomplete", {f.code for f in findings})
        self.assertEqual(gov.worst_level(findings), gov.WARN)

    def test_author_merging_own_pr_is_error(self) -> None:
        findings = gov.check_pr_body(self.GOOD_BODY, author="codex", merged_by="Codex")
        self.assertIn("author-merged-own-pr", {f.code for f in findings})
        self.assertEqual(gov.worst_level(findings), gov.ERROR)

    def test_different_merger_is_ok(self) -> None:
        findings = gov.check_pr_body(self.GOOD_BODY, author="codex", merged_by="claude")
        self.assertNotIn("author-merged-own-pr", {f.code for f in findings})
        self.assertEqual(gov.worst_level(findings), gov.OK)


class CliTests(unittest.TestCase):
    def test_run_returns_zero_on_healthy_repo(self) -> None:
        with tempfile.TemporaryDirectory() as root:
            _make_healthy_repo(root)
            self.assertEqual(gov.run(["--repo", root]), 0)

    def test_run_returns_one_on_author_self_merge(self) -> None:
        with tempfile.TemporaryDirectory() as root:
            _make_healthy_repo(root)
            code = gov.run(["--repo", root, "--author", "x", "--merged-by", "x"])
            self.assertEqual(code, 1)

    def test_run_ok_when_author_differs_from_merger(self) -> None:
        # The --author/--merged-by-without---pr-body path must still resolve the
        # routing check and pass when the two identities differ.
        with tempfile.TemporaryDirectory() as root:
            _make_healthy_repo(root)
            code = gov.run(["--repo", root, "--author", "codex", "--merged-by", "claude"])
            self.assertEqual(code, 0)

    def test_run_errors_on_unreadable_pr_body(self) -> None:
        with tempfile.TemporaryDirectory() as root:
            _make_healthy_repo(root)
            missing = os.path.join(root, "does-not-exist.md")
            code = gov.run(["--repo", root, "--pr-body", missing])
            self.assertEqual(code, 1)


if __name__ == "__main__":
    unittest.main()
