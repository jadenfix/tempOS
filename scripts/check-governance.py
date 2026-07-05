#!/usr/bin/env python3
"""Lint the cross-agent coordination ledger.

Enforces the machine-checkable parts of the beaterOS review gate
(see docs/governance/review-checklist.md and docs/governance/README.md):

  1. Every `merged` PR names a Merger agent.
  2. The Merger agent differs from the Author agent (the "no self-merge" rule,
     enforced at the agent-identity layer since all agents share one GitHub
     account).
  3. Every non-draft PR names a Reviewer agent that differs from the Author.

The reviewer and merger may be the same non-author agent; the invariant is
independence from the PR author, not mandatory separation between all three
roles.

Fails **closed**: a row whose Status is not a recognized value is reported, not
skipped, so a mislabelled status (e.g. `merged (fast-forward)`) cannot silently
bypass the checks. Agent-identity comparisons are case-insensitive.

This is intentionally dependency-free (stdlib only) so it can run in any CI image
without a toolchain. Exit code 0 = clean, 1 = violations found.

Usage:
    python3 scripts/check-governance.py [path-to-ledger.md]
"""

from __future__ import annotations

import sys
from pathlib import Path

DEFAULT_LEDGER = "docs/governance/coordination-ledger.md"

# A table is the canonical review ledger only if it carries all of these
# columns. Narrative tables (e.g. a fleet snapshot) that lack the full set are
# ignored, so they cannot cause false passes or false failures.
REQUIRED_COLUMNS = ("pr", "author agent", "reviewer agent", "merger agent", "status")

# Recognized statuses. Anything else is reported (fail closed), never skipped.
KNOWN_STATUSES = {
    "claimed", "draft-pr", "in-review", "changes-requested",
    "approved", "merged", "dropped",
}
# Statuses in which the row represents a real, non-draft PR under review.
NONDRAFT_STATUSES = {"in-review", "changes-requested", "approved", "merged"}

# Exact tokens (after emphasis-stripping, case-folded) that name no concrete
# agent. Kept exact so a real name like `pending-bot` is NOT treated as a
# placeholder.
PLACEHOLDER_TOKENS = {"", "-", "--", "n/a", "na", "tbd", "none", "pending"}


def strip_emphasis(cell: str) -> str:
    """Drop surrounding whitespace and markdown emphasis (`*`/`_`)."""
    return cell.strip().strip("*_").strip()


def is_placeholder(cell: str) -> bool:
    """True when the cell names no concrete agent.

    A cell is a placeholder if it is empty/dash, an exact placeholder token, or
    wrapped in markdown emphasis (e.g. `_pending (non-author)_`). Real names such
    as `pending-bot` are NOT placeholders.
    """
    raw = cell.strip()
    emphasized = len(raw) >= 2 and raw[0] in "*_" and raw[-1] in "*_"
    inner = strip_emphasis(raw).casefold()
    return emphasized or inner in PLACEHOLDER_TOKENS


def normalize_status(cell: str) -> str:
    """Status keyword, case-folded, with any trailing parenthetical stripped.

    `merged (fast-forward)` -> `merged`; `In-Review` -> `in-review`.
    """
    return strip_emphasis(cell).split("(")[0].strip().casefold()


def same_agent(a: str, b: str) -> bool:
    """Case-insensitive identity comparison after stripping emphasis."""
    return strip_emphasis(a).casefold() == strip_emphasis(b).casefold()


def parse_ledger_table(text: str) -> list[dict[str, str]]:
    """Extract rows from the canonical review-ledger table(s).

    Only tables carrying the full REQUIRED_COLUMNS set are parsed.
    """
    rows: list[dict[str, str]] = []
    header: list[str] | None = None
    header_matches = False
    for line in text.splitlines():
        line = line.strip()
        if not line.startswith("|"):
            header = None
            header_matches = False
            continue
        cells = [c.strip() for c in line.strip("|").split("|")]
        if header is None:
            header = [c.lower() for c in cells]
            header_matches = all(col in header for col in REQUIRED_COLUMNS)
            continue
        if all(set(c) <= {"-", ":", " "} for c in cells):  # separator row
            continue
        if not header_matches:
            continue
        rows.append(dict(zip(header, cells)))
    return rows


def check(rows: list[dict[str, str]]) -> list[str]:
    problems: list[str] = []
    for row in rows:
        pr = row.get("pr", "?").strip() or "?"
        raw_status = row.get("status", "").strip()
        status = normalize_status(raw_status)
        author = row.get("author agent", "")
        reviewer = row.get("reviewer agent", "")
        merger = row.get("merger agent", "")

        # Fail closed on anything we don't recognize, rather than skipping it.
        if status not in KNOWN_STATUSES:
            problems.append(
                f"{pr}: unrecognized status '{raw_status}' (fail-closed; "
                f"expected one of {sorted(KNOWN_STATUSES)})."
            )
            continue

        if status == "merged":
            if is_placeholder(merger):
                problems.append(f"{pr}: status 'merged' but no Merger agent named.")
            elif not is_placeholder(author) and same_agent(merger, author):
                problems.append(
                    f"{pr}: Merger '{merger.strip()}' is the Author — a PR must "
                    f"be merged by a different agent."
                )

        if status in NONDRAFT_STATUSES:
            if is_placeholder(reviewer):
                problems.append(
                    f"{pr}: status '{status}' but no Reviewer agent named."
                )
            elif not is_placeholder(author) and same_agent(reviewer, author):
                problems.append(
                    f"{pr}: Reviewer '{reviewer.strip()}' is the Author — a PR "
                    f"must be reviewed by a different agent."
                )
    return problems


def main(argv: list[str]) -> int:
    ledger_path = Path(argv[1]) if len(argv) > 1 else Path(DEFAULT_LEDGER)
    if not ledger_path.exists():
        print(f"error: ledger not found at {ledger_path}", file=sys.stderr)
        return 1
    rows = parse_ledger_table(ledger_path.read_text(encoding="utf-8"))
    if not rows:
        print(f"error: no canonical ledger table found in {ledger_path} "
              f"(needs columns: {', '.join(REQUIRED_COLUMNS)})", file=sys.stderr)
        return 1
    problems = check(rows)
    if problems:
        print("Governance check FAILED:", file=sys.stderr)
        for p in problems:
            print(f"  - {p}", file=sys.stderr)
        return 1
    print(f"Governance check passed: {len(rows)} ledger row(s) satisfy the "
          f"reviewer/merger differ from author rules.")
    return 0


if __name__ == "__main__":
    raise SystemExit(main(sys.argv))
