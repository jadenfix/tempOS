#!/usr/bin/env python3
"""Run the beater-osd runtime bootstrap smoke in an isolated runtime root."""

from __future__ import annotations

import argparse
import subprocess
import tempfile
import shutil
from pathlib import Path


REPO_ROOT = Path(__file__).resolve().parent.parent


def run_smoke(root: Path, *, as_json: bool) -> int:
    command = [
        "cargo",
        "run",
        "-q",
        "-p",
        "beater-osd",
        "--",
        "runtime-smoke",
        "--root",
        str(root),
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
        help="emit machine-readable smoke output from beater-osd",
    )
    parser.add_argument(
        "--root",
        type=Path,
        help=(
            "runtime root for this smoke run; defaults to a temporary directory "
            "for repeatable isolated execution"
        ),
    )
    parser.add_argument(
        "--keep-root",
        action="store_true",
        help="keep temporary runtime root for manual inspection",
    )
    args = parser.parse_args()

    if args.root is not None:
        return run_smoke(args.root, as_json=args.json)

    with tempfile.TemporaryDirectory(prefix="beater-osd-smoke-") as temporary:
        root = Path(temporary).resolve()
        code = run_smoke(root, as_json=args.json)
        if args.keep_root and code == 0:
            preserved = root
            # If we keep the root for post-run inspection, move it to a stable path
            # under the temporary folder's current parent.
            stable = root.parent / f"kept-beater-osd-runtime-smoke-{root.name}"
            if stable.exists():
                shutil.rmtree(stable)
            root.replace(stable)
            print(f"beater-osd runtime smoke root preserved at: {stable}")
        return code


if __name__ == "__main__":
    raise SystemExit(main())
