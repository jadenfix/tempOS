#!/usr/bin/env python3
"""Guard that `final.md` is never shortened or weakened.

`final.md` is the shared source of truth for the whole beaterOS fleet, and the
backlog rule is explicit: "final.md must not be shortened or weakened as part of
implementation." With several agents editing the repo in parallel, that rule
needs a machine check, not just etiquette.

This guard pins, in `scripts/final-integrity.lock.json`: the set of section
headings, the total document length, and the body length of each section. The
check FAILS if any pinned heading disappears, the document shrinks below the
pinned line count, or any individual section's body shrinks below its pinned
length (so prose cannot be gutted from one section while padding another). Growth
(new sections, more detail) is always allowed; only regressions fail.

The `sha256` field is an informational fingerprint only -- it is NOT enforced,
because additive edits legitimately change it. Content preservation is enforced
by the per-section line counts, not the hash.

Usage:
    python3 scripts/check-final-integrity.py            # check (non-zero exit on regression)
    python3 scripts/check-final-integrity.py --update   # re-pin after an intentional additive edit
"""
from __future__ import annotations

import argparse
import hashlib
import json
import re
import sys
from pathlib import Path

REPO_ROOT = Path(__file__).resolve().parent.parent
FINAL_MD = REPO_ROOT / "final.md"
LOCK_FILE = Path(__file__).resolve().parent / "final-integrity.lock.json"

HEADING_RE = re.compile(r"^(#{1,6})\s+(.*\S)\s*$")


def scan(path: Path) -> dict:
    text = path.read_text(encoding="utf-8")
    lines = text.splitlines()
    headings: list[str] = []
    # Body line count per heading (summed if a heading string repeats), so a
    # section cannot be hollowed out while total line count is padded elsewhere.
    section_lines: dict[str, int] = {}
    current: str | None = None
    for line in lines:
        match = HEADING_RE.match(line)
        if match:
            current = match.group(0).strip()
            headings.append(current)
            section_lines.setdefault(current, 0)
        elif current is not None:
            section_lines[current] += 1
    return {
        "line_count": len(lines),
        "headings": headings,
        "section_lines": section_lines,
        "sha256": hashlib.sha256(text.encode("utf-8")).hexdigest(),
    }


def update() -> int:
    snapshot = scan(FINAL_MD)
    LOCK_FILE.write_text(json.dumps(snapshot, indent=2) + "\n", encoding="utf-8")
    print(f"pinned final.md: {len(snapshot['headings'])} headings, "
          f"{snapshot['line_count']} lines")
    return 0


def check() -> int:
    if not LOCK_FILE.exists():
        print("no lock file; run --update first", file=sys.stderr)
        return 1
    locked = json.loads(LOCK_FILE.read_text(encoding="utf-8"))
    current = scan(FINAL_MD)

    problems: list[str] = []

    missing = [h for h in locked["headings"] if h not in current["headings"]]
    if missing:
        problems.append(
            "final.md lost pinned section(s):\n  - " + "\n  - ".join(missing)
        )

    if current["line_count"] < locked["line_count"]:
        problems.append(
            f"final.md shrank from {locked['line_count']} to "
            f"{current['line_count']} lines (weakening the plan is not allowed)"
        )

    locked_sections = locked.get("section_lines", {})
    current_sections = current["section_lines"]
    shrunk = [
        f"{heading!r}: {current_sections.get(heading, 0)} < {pinned} lines"
        for heading, pinned in locked_sections.items()
        if heading in current["headings"] and current_sections.get(heading, 0) < pinned
    ]
    if shrunk:
        problems.append(
            "final.md section(s) were hollowed out:\n  - " + "\n  - ".join(shrunk)
        )

    if problems:
        print("final.md integrity check FAILED:\n", file=sys.stderr)
        for problem in problems:
            print(problem + "\n", file=sys.stderr)
        print(
            "If this change is an intentional, additive edit, re-pin with:\n"
            "    python3 scripts/check-final-integrity.py --update",
            file=sys.stderr,
        )
        return 1

    grew = current["line_count"] - locked["line_count"]
    note = f" (+{grew} lines since pin)" if grew else ""
    print(f"final.md integrity OK: {len(current['headings'])} headings{note}")
    return 0


def main(argv: list[str]) -> int:
    parser = argparse.ArgumentParser(description=__doc__)
    parser.add_argument("--update", action="store_true", help="re-pin the lock file")
    args = parser.parse_args(argv)
    return update() if args.update else check()


if __name__ == "__main__":
    raise SystemExit(main(sys.argv[1:]))
