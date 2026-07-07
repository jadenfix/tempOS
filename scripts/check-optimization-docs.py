#!/usr/bin/env python3
"""Validate optimization doctrine and toolchain evidence docs.

This is a lightweight doc-health gate. It does not prove a performance claim;
it prevents the repo from losing the structures agents need to make future
claims replayable.
"""

from __future__ import annotations

import json
import re
import sys
from pathlib import Path

REPO_ROOT = Path(__file__).resolve().parent.parent
FINAL_SOURCE_SECTION_RE = re.compile(
    r"^## 27\. Source Matrix(?P<section>.*?)(?=^## 28\. Final Strategic Recommendation|\Z)",
    flags=re.MULTILINE | re.DOTALL,
)
URL_RE = re.compile(r"https?://[^ )]+")
SOURCE_MATRIX_COUNT_RE = re.compile(
    r"- (?P<count>\d+) URLs are extracted from the current `final\.md` section 27\."
)

REQUIRED_MARKERS: dict[str, tuple[str, ...]] = {
    "docs/optimization-agent-playbook.md": (
        "## Bottleneck Taxonomy",
        "## Required Optimization Packet",
        "## Benchmark Acceptance Policy",
        "## Portable Accelerator Contract Sketch",
        "## Accelerator Review Packet",
        "docs/engineering/metal-os-blueprint.md",
        "benchmarks/manifest.json",
        "contracts/schema/performance-trace.schema.json",
        "contracts/schema/accelerator-telemetry.schema.json",
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
        "## Upcoming Or Breaking Target Changes",
        "| Component | Upstream version/status | Repo baseline | Primary source | Source type | Source date | Verified on | Optimization relevance | Claim boundary |",
        "| Rust |",
        "| LLVM |",
        "| CUDA Toolkit |",
        "| AMD ROCm/HIP |",
        "| Intel Level Zero |",
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

REQUIRED_JSON_FILES: tuple[str, ...] = (
    "benchmarks/manifest.json",
    "contracts/schema/performance-trace.schema.json",
    "contracts/schema/accelerator-telemetry.schema.json",
)


def validate_file(relative_path: str, markers: tuple[str, ...]) -> list[str]:
    path = REPO_ROOT / relative_path
    if not path.exists():
        return [f"{relative_path}: missing file"]
    body = path.read_text(encoding="utf-8")
    return [f"{relative_path}: missing marker {marker!r}" for marker in markers if marker not in body]


def validate_json(relative_path: str) -> list[str]:
    path = REPO_ROOT / relative_path
    if not path.exists():
        return [f"{relative_path}: missing file"]
    try:
        with path.open(encoding="utf-8") as handle:
            payload = json.load(handle)
    except json.JSONDecodeError as exc:
        return [f"{relative_path}: invalid JSON: {exc}"]
    if relative_path == "benchmarks/manifest.json":
        workloads = payload.get("workloads")
        if not isinstance(workloads, list):
            return [f"{relative_path}: missing workloads array"]
        for index, workload in enumerate(workloads):
            command = workload.get("command") if isinstance(workload, dict) else None
            if not isinstance(command, list) or not command:
                return [f"{relative_path}: workload {index} missing command array"]
    return []


def validate_source_matrix_url_count() -> list[str]:
    final_body = (REPO_ROOT / "final.md").read_text(encoding="utf-8")
    source_body = (REPO_ROOT / "docs" / "source-matrix.md").read_text(encoding="utf-8")
    section_match = FINAL_SOURCE_SECTION_RE.search(final_body)
    if section_match is None:
        return ["final.md: missing section 27 Source Matrix"]
    actual_count = len(URL_RE.findall(section_match.group("section")))
    count_match = SOURCE_MATRIX_COUNT_RE.search(source_body)
    if count_match is None:
        return ["docs/source-matrix.md: missing current final.md URL extraction count"]
    documented_count = int(count_match.group("count"))
    if actual_count != documented_count:
        return [
            "docs/source-matrix.md: current final.md URL extraction count "
            f"{documented_count} does not match actual count {actual_count}"
        ]
    return []


def main() -> int:
    errors: list[str] = []
    for relative_path, markers in REQUIRED_MARKERS.items():
        errors.extend(validate_file(relative_path, markers))
    for relative_path in REQUIRED_JSON_FILES:
        errors.extend(validate_json(relative_path))
    errors.extend(validate_source_matrix_url_count())
    if not errors:
        print("optimization docs OK")
        return 0
    print("optimization docs check failed")
    for error in errors:
        print(f"- {error}")
    return 1


if __name__ == "__main__":
    sys.exit(main())
