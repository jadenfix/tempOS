#!/usr/bin/env python3
"""Run the beater-os-runtime supervised worker service smoke artifact."""

from __future__ import annotations

import argparse
import subprocess
import sys
from pathlib import Path


REPO_ROOT = Path(__file__).resolve().parent.parent


def main() -> int:
    parser = argparse.ArgumentParser(description=__doc__)
    parser.add_argument(
        "--json",
        action="store_true",
        help="emit machine-readable runtime worker supervisor service output",
    )
    args = parser.parse_args()
    command = [
        "cargo",
        "run",
        "-q",
        "-p",
        "beater-os-runtime",
        "--example",
        "local_shell_supervisor_service",
        "--",
    ]
    if args.json:
        command.append("--json")
    completed = subprocess.run(command, cwd=REPO_ROOT, check=False)
    return completed.returncode


if __name__ == "__main__":
    sys.exit(main())
