#!/usr/bin/env python3
"""beaterOS multi-agent PR governance checker.

This is a small, dependency-free (Python standard library only) tool that
encodes the *process* invariants beaterOS depends on when many agents work on
the same repository in parallel. It is deliberately language-agnostic: it does
not build or test the Rust workspace (that is `cargo`'s job in the separate CI
job); it checks that the collaboration contract described in ``AGENTS.md`` and
``docs/multi-agent-review-protocol.md`` is being respected.

It enforces two classes of rules:

* Repository invariants (checked in CI on every PR):
    - ``final.md`` exists and has not been gutted/weakened
      (final.md Section 26: "final.md must not be shortened or weakened").
    - The shared coordination artifacts exist (warning-only so that
      in-flight PRs opened before this tool landed are not hard-failed).

* PR-routing invariants (checked on demand with ``--pr-body``):
    - The PR body carries the shared "Review routing" contract.
    - The author of a PR is never also its merger / sole reviewer
      (final.md-aligned rule: "No PR is merged by the agent who authored it").

Exit code is non-zero only when at least one ERROR-level finding is produced,
so wiring this into CI cannot spuriously break another agent's unrelated PR.
"""

from __future__ import annotations

import argparse
import os
import sys
from dataclasses import dataclass
from typing import List, Optional

# --- Tunable thresholds -----------------------------------------------------

# final.md is the source of truth for the whole project. A large accidental (or
# adversarial) truncation is one of the few things worth *hard*-failing a PR on.
# The floor is intentionally well below the current length so ordinary edits and
# refactors pass; only a gutting trips it. Override with FINAL_MD_MIN_LINES.
DEFAULT_FINAL_MD_MIN_LINES = 3000

REQUIRED_PROCESS_FILES = [
    "AGENTS.md",
    "docs/multi-agent-review-protocol.md",
    "docs/review-checklist.md",
    "docs/agent-coordination-log.md",
    ".github/PULL_REQUEST_TEMPLATE.md",
]

# Phrases that must survive in the PR template / body for the routing contract
# to be considered present. Kept loose so wording can evolve.
ROUTING_MARKERS = [
    "did not author",  # "Reviewed by an agent/person who did not author the PR."
    "Merge performed",  # "Merge performed by an agent/person who did not author the PR."
]

ERROR = "ERROR"
WARN = "WARN"
OK = "OK"

_LEVEL_RANK = {OK: 0, WARN: 1, ERROR: 2}


@dataclass(frozen=True)
class Finding:
    level: str
    code: str
    message: str

    def format(self) -> str:
        return f"[{self.level:<5}] {self.code}: {self.message}"


def _read_text(path: str) -> Optional[str]:
    try:
        with open(path, "r", encoding="utf-8") as handle:
            return handle.read()
    except OSError:
        return None


def _final_md_min_lines() -> int:
    raw = os.environ.get("FINAL_MD_MIN_LINES")
    if not raw:
        return DEFAULT_FINAL_MD_MIN_LINES
    try:
        return int(raw)
    except ValueError:
        return DEFAULT_FINAL_MD_MIN_LINES


def check_repo_invariants(repo_root: str) -> List[Finding]:
    """Checks that do not need a PR body. Safe to run in CI on any PR."""
    findings: List[Finding] = []

    final_path = os.path.join(repo_root, "final.md")
    final_text = _read_text(final_path)
    if final_text is None:
        findings.append(
            Finding(ERROR, "final-md-missing", "final.md is missing (source of truth for beaterOS).")
        )
    else:
        line_count = len(final_text.splitlines())
        floor = _final_md_min_lines()
        if line_count < floor:
            findings.append(
                Finding(
                    ERROR,
                    "final-md-weakened",
                    f"final.md has {line_count} lines, below the floor of {floor}. "
                    "final.md must not be shortened or weakened (in the spirit of "
                    "final.md Section 26, 'What Not To Compromise').",
                )
            )
        else:
            findings.append(
                Finding(OK, "final-md-ok", f"final.md present with {line_count} lines.")
            )

    for rel in REQUIRED_PROCESS_FILES:
        if _read_text(os.path.join(repo_root, rel)) is None:
            findings.append(
                Finding(
                    WARN,
                    "process-file-missing",
                    f"expected coordination artifact '{rel}' not found "
                    "(warning only so in-flight PRs are not blocked).",
                )
            )
        else:
            findings.append(Finding(OK, "process-file-ok", f"found '{rel}'."))

    return findings


def check_pr_body(
    body: str,
    author: Optional[str] = None,
    merged_by: Optional[str] = None,
) -> List[Finding]:
    """Checks the routing contract carried by a PR description."""
    findings: List[Finding] = []

    missing_markers = [m for m in ROUTING_MARKERS if m not in body]
    if missing_markers:
        findings.append(
            Finding(
                WARN,
                "routing-section-incomplete",
                "PR body is missing review-routing language: "
                + ", ".join(repr(m) for m in missing_markers),
            )
        )
    else:
        findings.append(Finding(OK, "routing-section-ok", "PR body carries the review-routing contract."))

    # The one rule we hard-enforce: an author may not merge their own PR.
    if author is not None and merged_by is not None:
        if _normalize(author) == _normalize(merged_by):
            findings.append(
                Finding(
                    ERROR,
                    "author-merged-own-pr",
                    f"author '{author}' is also the merger. A PR must be merged by a "
                    "different agent/person than the one who wrote it.",
                )
            )
        else:
            findings.append(
                Finding(OK, "author-differs-from-merger", f"author '{author}' != merger '{merged_by}'.")
            )

    return findings


def _normalize(name: str) -> str:
    return name.strip().lower()


def worst_level(findings: List[Finding]) -> str:
    worst = OK
    for finding in findings:
        if _LEVEL_RANK[finding.level] > _LEVEL_RANK[worst]:
            worst = finding.level
    return worst


def run(argv: Optional[List[str]] = None) -> int:
    parser = argparse.ArgumentParser(description="beaterOS multi-agent PR governance checker")
    parser.add_argument("--repo", default=".", help="path to the repository root (default: .)")
    parser.add_argument("--pr-body", help="path to a file containing the PR description to check")
    parser.add_argument("--author", help="login/handle of the PR author (for merge-routing check)")
    parser.add_argument("--merged-by", help="login/handle of whoever is merging (for merge-routing check)")
    args = parser.parse_args(argv)

    findings = check_repo_invariants(args.repo)

    if args.pr_body:
        body = _read_text(args.pr_body)
        if body is None:
            findings.append(Finding(ERROR, "pr-body-unreadable", f"could not read --pr-body '{args.pr_body}'."))
        else:
            findings.extend(check_pr_body(body, author=args.author, merged_by=args.merged_by))
    elif args.author or args.merged_by:
        # Author/merged-by are only meaningful alongside a body's routing contract,
        # but the merge-routing check is valuable on its own too.
        findings.extend(check_pr_body("", author=args.author, merged_by=args.merged_by))

    for finding in findings:
        print(finding.format())

    level = worst_level(findings)
    print()
    print(f"governance check result: {level}")
    return 1 if level == ERROR else 0


if __name__ == "__main__":
    raise SystemExit(run())
