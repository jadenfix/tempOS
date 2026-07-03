#!/usr/bin/env python3
"""Validate every scenario manifest against the ScenarioManifest schema.

Reuses the dependency-free validator from ../contracts/validate.py so there is
one validation engine and one source-of-truth schema (no duplication). Runs on
a stock Python 3 with no install and no network.

Usage:
    python3 scenarios/validate_scenarios.py
    python3 scenarios/validate_scenarios.py --quiet

Exit code is 0 iff every scenarios/*.json validates against
contracts/schemas/scenario-manifest.schema.json.
"""
from __future__ import annotations

import json
import os
import sys

HERE = os.path.dirname(os.path.abspath(__file__))
REPO = os.path.dirname(HERE)
CONTRACTS = os.path.join(REPO, "contracts")

sys.path.insert(0, CONTRACTS)
try:
    from validate import Validator, load_schemas  # type: ignore
except Exception as exc:  # pragma: no cover
    print(f"error: could not import the contract validator from {CONTRACTS}: {exc}")
    print("This slice depends on the contracts/ schemas (PR-C). Ensure contracts/ is present.")
    raise SystemExit(2)

SCHEMA_NAME = "scenario-manifest.schema.json"


def main() -> int:
    quiet = "--quiet" in sys.argv
    schemas = load_schemas()
    if SCHEMA_NAME not in schemas:
        print(f"error: {SCHEMA_NAME} not found in contracts/schemas/")
        return 2
    validator = Validator(schemas)
    schema_doc = schemas[SCHEMA_NAME]

    files = sorted(f for f in os.listdir(HERE) if f.endswith(".json"))
    failures: list[str] = []
    passes = 0
    for fn in files:
        with open(os.path.join(HERE, fn)) as f:
            instance = json.load(f)
        errors: list[str] = []
        validator.validate(instance, schema_doc, schema_doc, "$", errors)
        if errors:
            failures.append(f"{fn}:\n    - " + "\n    - ".join(errors))
        else:
            passes += 1
            if not quiet:
                sid = instance.get("scenario_id", "?")
                print(f"  ok    {fn} ({sid})")

    print(f"\n{passes}/{len(files)} scenarios valid.")
    if failures:
        print(f"\n{len(failures)} FAILURE(S):")
        for fail in failures:
            print(f"- {fail}")
        return 1
    print("All scenario manifests validate against ScenarioManifest. ✔")
    return 0


if __name__ == "__main__":
    raise SystemExit(main())
