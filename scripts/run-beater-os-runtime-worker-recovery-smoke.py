#!/usr/bin/env python3
"""Run the runtime worker lease recovery smoke artifact."""

from __future__ import annotations

import argparse
import subprocess
from pathlib import Path


REPO_ROOT = Path(__file__).resolve().parent.parent


def run_smoke(*, as_json: bool) -> int:
    command = [
        "cargo",
        "run",
        "-q",
        "--locked",
        "-p",
        "beater-os-runtime",
        "--example",
        "local_shell_worker_recovery",
        "--",
    ]
    if as_json:
        command.append("--json")
    completed = subprocess.run(command, cwd=REPO_ROOT, check=False)
    return completed.returncode


def main() -> int:
    parser = argparse.ArgumentParser(description=__doc__)
    parser.add_argument(
        "--json",
        action="store_true",
        help="emit machine-readable runtime worker recovery smoke output",
    )
    args = parser.parse_args()
    return run_smoke(as_json=args.json)


if __name__ == "__main__":
    raise SystemExit(main())
