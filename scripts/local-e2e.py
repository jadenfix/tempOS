#!/usr/bin/env python3
"""Run the local beaterOS end-to-end gate suite.

This is a dependency-free wrapper over the repo's existing checks. Default mode
fails fast for quick local iteration; `--full-report` runs every gate and reports
all failures before exiting.

Usage:
    python3 scripts/local-e2e.py
    python3 scripts/local-e2e.py --full-report
    python3 scripts/local-e2e.py --list
"""

from __future__ import annotations

import argparse
import shlex
import subprocess
import sys
import time
from dataclasses import dataclass
from pathlib import Path
from typing import Callable, Iterable, Sequence

REPO_ROOT = Path(__file__).resolve().parent.parent


@dataclass(frozen=True)
class Gate:
    name: str
    command: tuple[str, ...]


@dataclass(frozen=True)
class GateResult:
    gate: Gate
    returncode: int
    elapsed_s: float


def origin_main_available(repo_root: Path = REPO_ROOT) -> bool:
    """Whether an `origin/main` remote-tracking ref exists locally.

    Fresh or shallow clones (and CI jobs that never fetched it) have no such ref,
    so any gate that diffs against `origin/main` must degrade rather than error.
    """
    completed = subprocess.run(
        ("git", "rev-parse", "--verify", "--quiet", "origin/main"),
        cwd=repo_root,
        capture_output=True,
        check=False,
    )
    return completed.returncode == 0


def branch_whitespace_gate(python: str, *, available: bool) -> Gate:
    """The branch-whitespace gate, tolerant of a missing `origin/main` ref (#62).

    With the ref present, check whitespace across the branch's diff from main.
    Without it, `git diff origin/main...HEAD` would fail on the unknown ref
    rather than reflect real content, so skip with a note: the worktree-whitespace
    gate (`git diff HEAD --check`) already covers the local working tree.
    """
    if available:
        return Gate("branch-whitespace", ("git", "diff", "--check", "origin/main...HEAD"))
    note = "branch-whitespace skipped: origin/main not available (worktree-whitespace covers local content)"
    return Gate("branch-whitespace", (python, "-c", f"print({note!r})"))


def build_plan(
    python: str = "python3",
    *,
    branch_base_available: bool | None = None,
    host_profile: Path | None = None,
) -> list[Gate]:
    if branch_base_available is None:
        branch_base_available = origin_main_available()
    gates = [
        Gate("worktree-whitespace", ("git", "diff", "HEAD", "--check")),
        branch_whitespace_gate(python, available=branch_base_available),
        Gate("final-integrity", (python, "scripts/check-final-integrity.py")),
        Gate(
            "governance-ledger",
            (python, "scripts/check-governance.py", "docs/governance/coordination-ledger.md"),
        ),
    ]
    gates.append(
        Gate("beater-osd-runtime-smoke", (python, "scripts/run-beater-osd-runtime-smoke.py", "--json"))
    )
    gates.append(
        Gate("beater-os-runtime-smoke", (python, "scripts/run-beater-os-runtime-smoke.py", "--json"))
    )
    gates.append(
        Gate(
            "beater-osd-http-execute-smoke",
            (python, "scripts/run-beater-osd-http-execute-smoke.py", "--json"),
        )
    )
    bare_metal_readiness = [
        python,
        "scripts/check-bare-metal-readiness.py",
        "--check-host",
        "--require-control-plane-lane",
        "--require-workload-class",
        "policy-admission",
        "--require-workload-route",
        "policy-admission=portable-control-plane",
        "--require-migration-phase",
        "runtime",
    ]
    if host_profile is not None:
        bare_metal_readiness.extend(["--strict-host-context", "--host-profile", str(host_profile)])
    gates.append(
        Gate(
            "bare-metal-readiness",
            tuple(bare_metal_readiness),
        )
    )
    gates.append(Gate("bare-metal-e2e-matrix", (python, "scripts/run-bare-metal-e2e-matrix.py")))
    gates.append(Gate("optimization-docs", (python, "scripts/check-optimization-docs.py")))
    gates.extend(
        [
        Gate("python-unit-tests", (python, "-m", "unittest", "discover", "-s", "tests")),
        Gate("spec-conformance", (python, "spec/conformance/validate.py", "--quiet")),
        Gate("conformance-selftest", (python, "tools/conformance/selftest.py")),
        Gate(
            "fixture-freshness",
            (python, "tools/conformance/build_fixtures.py", "--check"),
        ),
        Gate("trace-and-scenario-conformance", (python, "tools/conformance/validate.py")),
        Gate("rustfmt", ("cargo", "fmt", "--all", "--", "--check")),
        Gate("rust-tests", ("cargo", "test", "--workspace", "--locked")),
        Gate(
            "rust-clippy",
            ("cargo", "clippy", "--workspace", "--all-targets", "--locked", "--", "-D", "warnings"),
        ),
        ]
    )
    return gates


def format_command(command: Sequence[str]) -> str:
    return " ".join(shlex.quote(part) for part in command)


def execute_gate(gate: Gate, *, repo_root: Path = REPO_ROOT) -> GateResult:
    started = time.monotonic()
    print(f"==> {gate.name}: {format_command(gate.command)}", flush=True)
    completed = subprocess.run(gate.command, cwd=repo_root, check=False)
    elapsed = time.monotonic() - started
    status = "ok" if completed.returncode == 0 else f"FAILED ({completed.returncode})"
    print(f"<== {gate.name}: {status} in {elapsed:.1f}s", flush=True)
    return GateResult(gate=gate, returncode=completed.returncode, elapsed_s=elapsed)


def run_plan(
    plan: Iterable[Gate],
    *,
    full_report: bool,
    execute: Callable[[Gate], GateResult],
) -> list[GateResult]:
    results: list[GateResult] = []
    for gate in plan:
        result = execute(gate)
        results.append(result)
        if result.returncode != 0 and not full_report:
            break
    return results


def print_summary(results: Sequence[GateResult], *, total_gates: int) -> None:
    print("\nLocal E2E summary:")
    for result in results:
        marker = "PASS" if result.returncode == 0 else "FAIL"
        print(f"  {marker:4} {result.gate.name} ({result.elapsed_s:.1f}s)")
    skipped = total_gates - len(results)
    if skipped:
        print(f"  SKIP {skipped} gate(s) after fail-fast stop")


def exit_code(results: Sequence[GateResult]) -> int:
    return 0 if all(result.returncode == 0 for result in results) else 1


def main(argv: list[str]) -> int:
    parser = argparse.ArgumentParser(description=__doc__)
    parser.add_argument(
        "--full-report",
        action="store_true",
        help="run every gate even after a failure",
    )
    parser.add_argument(
        "--host-profile",
        type=Path,
        help="optional host snapshot path passed to bare-metal-readiness",
    )
    parser.add_argument("--list", action="store_true", help="print the gate plan and exit")
    args = parser.parse_args(argv)

    plan = build_plan(host_profile=args.host_profile)
    if args.list:
        for gate in plan:
            print(f"{gate.name}: {format_command(gate.command)}")
        return 0

    results = run_plan(plan, full_report=args.full_report, execute=execute_gate)
    print_summary(results, total_gates=len(plan))
    return exit_code(results)


if __name__ == "__main__":
    raise SystemExit(main(sys.argv[1:]))
