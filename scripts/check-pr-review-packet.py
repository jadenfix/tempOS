#!/usr/bin/env python3
"""Validate the optimization packet in a pull request body."""

from __future__ import annotations

import os
import re
import sys


SECTION_RE = re.compile(
    r"^##\s+{heading}\s*$"
    r"(?P<section>.*?)"
    r"(?=^##\s+|\Z)",
    flags=re.IGNORECASE | re.MULTILINE | re.DOTALL,
)

CHECKED_BULLET_RE = re.compile(r"^\s*-\s*\[[xX]\]\s*(?P<body>.*)$")
ANY_BULLET_RE = re.compile(r"^\s*-\s*\[[ xX]\]\s*")

REQUIRED_ITEMS: tuple[tuple[str, tuple[str, ...]], ...] = (
    ("hot path", ("hot path",)),
    ("bottleneck class", ("bottleneck class",)),
    ("baseline/target/replay/regression", ("baseline", "target", "replay", "regression")),
    ("toolchain versions", ("compiler", "runtime", "backend")),
    ("authority/fallback/rollback", ("authority", "fallback", "rollback")),
    ("source links/dates", ("source links", "dates")),
)

SENSITIVE_TYPE_MARKERS = (
    "performance, language boundary, compiler/runtime, accelerator, or close-to-metal",
)


def _find_section(body: str, heading: str) -> str | None:
    pattern = re.compile(
        SECTION_RE.pattern.format(heading=re.escape(heading)),
        flags=SECTION_RE.flags,
    )
    match = pattern.search(body)
    if match is None:
        return None
    return match.group("section")


def _find_optimization_section(body: str) -> str | None:
    return _find_section(body, "Optimization packet")


def _is_explicit_na(section: str) -> bool:
    meaningful_lines = [
        line.strip()
        for line in section.splitlines()
        if line.strip() and not line.strip().startswith("<!--")
    ]
    return meaningful_lines == ["N/A"]


def _checked_bullet_blocks(section: str) -> list[str]:
    blocks: list[list[str]] = []
    current: list[str] | None = None
    for line in section.splitlines():
        checked = CHECKED_BULLET_RE.match(line)
        if checked is not None:
            current = [checked.group("body")]
            blocks.append(current)
            continue
        if ANY_BULLET_RE.match(line):
            current = None
            continue
        if current is not None and line.strip():
            current.append(line.strip())
    return [" ".join(block).lower() for block in blocks]


def _has_checked_sensitive_type(body: str) -> bool:
    section = _find_section(body, "Type of change")
    if section is None:
        return False
    checked_blocks = _checked_bullet_blocks(section)
    return any(
        any(marker in block for marker in SENSITIVE_TYPE_MARKERS)
        for block in checked_blocks
    )


def validate_pr_body(body: str) -> list[str]:
    section = _find_optimization_section(body)
    if section is None:
        return ["missing '## Optimization packet' section"]
    if _is_explicit_na(section):
        if _has_checked_sensitive_type(body):
            return ["optimization packet cannot be N/A for performance-sensitive changes"]
        return []

    checked_blocks = _checked_bullet_blocks(section)
    errors: list[str] = []
    for name, required_terms in REQUIRED_ITEMS:
        if not any(all(term in block for term in required_terms) for block in checked_blocks):
            errors.append(f"optimization packet missing checked item: {name}")
    return errors


def main() -> int:
    errors = validate_pr_body(os.environ.get("PR_BODY", ""))
    if not errors:
        return 0
    print("Optimization-sensitive PRs must include a completed packet, or mark this section exactly N/A.")
    for error in errors:
        print(f"- {error}")
    return 1


if __name__ == "__main__":
    sys.exit(main())
