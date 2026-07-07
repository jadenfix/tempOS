#!/usr/bin/env python3
"""Validate optimization doctrine and toolchain evidence docs.

This is a lightweight doc-health gate. It does not prove a performance claim;
it prevents the repo from losing the structures agents need to make future
claims replayable.
"""

from __future__ import annotations

import sys
from pathlib import Path

REPO_ROOT = Path(__file__).resolve().parent.parent

REQUIRED_MARKERS: dict[str, tuple[str, ...]] = {
    "docs/optimization-agent-playbook.md": (
        "## Bottleneck Taxonomy",
        "## Required Optimization Packet",
        "## Portable Accelerator Contract Sketch",
        "## Accelerator Review Packet",
        "docs/engineering/metal-os-blueprint.md",
    ),
    "docs/engineering/metal-os-blueprint.md": (
        "## First-Principles OS Shape",
        "## Three Engineering Lanes",
        "## Accelerator Fabric",
        "## Optimization Infrastructure For Agents",
        "## Review Gate",
    ),
    "docs/engineering/optimization-evidence-runbook.md": (
        "## First-Principles Start",
        "## Replay Packet",
        "## Language Boundary Review",
        "## Accelerator Packet",
        "## Local Gate",
    ),
    "docs/source-matrix.md": (
        "## Toolchain Freshness Ledger",
        "| Component | Upstream version/status | Repo baseline | Primary source | Source type | Source date | Verified on | Optimization relevance | Claim boundary |",
        "| Rust |",
        "| LLVM |",
        "| CUDA Toolkit |",
        "| Apple Metal |",
        "| Cloud TPU |",
        "| Groq LPU |",
    ),
    "docs/sota-systems-engineering.md": (
        "Before optimizing, classify the bottleneck:",
        "Accelerator engineering rules:",
        "Toolchain facts are temporal.",
        "docs/engineering/metal-os-blueprint.md",
    ),
    "AGENTS.md": (
        "docs/engineering/metal-os-blueprint.md",
        "docs/engineering/optimization-evidence-runbook.md",
        "Performance-Sensitive PR Packet",
    ),
    ".codex/skills/beateros-systems-engineering/SKILL.md": (
        "docs/engineering/metal-os-blueprint.md",
        "docs/engineering/optimization-evidence-runbook.md",
        "docs/source-matrix.md",
    ),
    "CLAUDE.md": (
        "docs/engineering/metal-os-blueprint.md",
        "docs/engineering/optimization-evidence-runbook.md",
        "docs/source-matrix.md",
        "GPU, TPU, LPU, NPU",
    ),
    ".cursor/rules/beateros.mdc": (
        "docs/engineering/metal-os-blueprint.md",
        "docs/engineering/optimization-evidence-runbook.md",
        "docs/source-matrix.md",
        "GPU, TPU, LPU, NPU",
    ),
    "README.md": (
        "docs/engineering/metal-os-blueprint.md",
        "docs/engineering/optimization-evidence-runbook.md",
        "python3 scripts/check-optimization-docs.py",
    ),
    ".github/PULL_REQUEST_TEMPLATE.md": (
        "docs/engineering/metal-os-blueprint.md",
        "docs/engineering/optimization-evidence-runbook.md",
        "python3 scripts/check-optimization-docs.py",
    ),
    "docs/governance/review-checklist.md": (
        "## D. Optimization and metal-readiness review",
        "docs/engineering/metal-os-blueprint.md",
        "docs/engineering/optimization-evidence-runbook.md",
        "docs/source-matrix.md",
    ),
}


def validate_file(relative_path: str, markers: tuple[str, ...]) -> list[str]:
    path = REPO_ROOT / relative_path
    if not path.exists():
        return [f"{relative_path}: missing file"]
    body = path.read_text(encoding="utf-8")
    return [f"{relative_path}: missing marker {marker!r}" for marker in markers if marker not in body]


def main() -> int:
    errors: list[str] = []
    for relative_path, markers in REQUIRED_MARKERS.items():
        errors.extend(validate_file(relative_path, markers))
    if not errors:
        print("optimization docs OK")
        return 0
    print("optimization docs check failed")
    for error in errors:
        print(f"- {error}")
    return 1


if __name__ == "__main__":
    sys.exit(main())
