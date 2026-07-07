#!/usr/bin/env python3
"""Replay registered optimization workloads.

This runner is intentionally small. It validates the benchmark manifest shape,
executes command-array workloads without a shell, enforces per-sample timeouts,
and fails when a registered workload exits non-zero. Numeric performance budget
comparison can be added when real timing/telemetry workloads are registered.
"""

from __future__ import annotations

import argparse
import json
import subprocess
import sys
import time
from pathlib import Path
from typing import Any

REPO_ROOT = Path(__file__).resolve().parent.parent
DEFAULT_MANIFEST = REPO_ROOT / "benchmarks" / "manifest.json"


def _load_manifest(path: Path) -> dict[str, Any]:
    with path.open(encoding="utf-8") as handle:
        manifest = json.load(handle)
    if not isinstance(manifest.get("workloads"), list):
        raise ValueError("manifest must contain a workloads array")
    for workload in manifest["workloads"]:
        if not isinstance(workload, dict):
            raise ValueError("each workload must be an object")
        command = workload.get("command")
        if not isinstance(command, list) or not command or not all(
            isinstance(part, str) and part for part in command
        ):
            raise ValueError(f"{workload.get('name', '<unnamed>')}: command must be a non-empty string array")
        timeout = workload.get("timeout_seconds")
        samples = workload.get("samples")
        if not isinstance(timeout, int) or timeout <= 0:
            raise ValueError(f"{workload.get('name', '<unnamed>')}: timeout_seconds must be positive")
        if not isinstance(samples, int) or samples <= 0:
            raise ValueError(f"{workload.get('name', '<unnamed>')}: samples must be positive")
    return manifest


def _selected_workloads(manifest: dict[str, Any], only: str | None) -> list[dict[str, Any]]:
    workloads = manifest["workloads"]
    if only is None:
        return workloads
    selected = [workload for workload in workloads if workload.get("name") == only]
    if not selected:
        raise ValueError(f"unknown workload: {only}")
    return selected


def _run_workload(workload: dict[str, Any]) -> int:
    name = workload.get("name", "<unnamed>")
    command = workload["command"]
    samples = workload["samples"]
    timeout = workload["timeout_seconds"]
    print(f"workload {name}: {' '.join(command)}")
    for sample in range(1, samples + 1):
        started = time.monotonic()
        completed = subprocess.run(
            command,
            cwd=REPO_ROOT,
            timeout=timeout,
            text=True,
            capture_output=True,
            check=False,
        )
        elapsed_ms = int((time.monotonic() - started) * 1000)
        if completed.stdout:
            print(completed.stdout, end="")
        if completed.stderr:
            print(completed.stderr, end="", file=sys.stderr)
        print(f"sample {sample}/{samples}: exit={completed.returncode} elapsed_ms={elapsed_ms}")
        if completed.returncode != 0:
            return completed.returncode
    return 0


def main() -> int:
    parser = argparse.ArgumentParser(description=__doc__)
    parser.add_argument("--manifest", type=Path, default=DEFAULT_MANIFEST)
    parser.add_argument("--only")
    parser.add_argument("--list", action="store_true")
    args = parser.parse_args()

    try:
        manifest = _load_manifest(args.manifest)
        workloads = _selected_workloads(manifest, args.only)
    except (OSError, ValueError, json.JSONDecodeError) as exc:
        print(f"optimization benchmark manifest error: {exc}", file=sys.stderr)
        return 2

    if args.list:
        for workload in workloads:
            print(workload.get("name", "<unnamed>"))
        return 0

    for workload in workloads:
        rc = _run_workload(workload)
        if rc != 0:
            return rc
    return 0


if __name__ == "__main__":
    sys.exit(main())
