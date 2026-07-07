#!/usr/bin/env python3
"""Guard the repo against two competing core contract sources of truth."""

from __future__ import annotations

from pathlib import Path

REPO_ROOT = Path(__file__).resolve().parent.parent
SPEC_README = REPO_ROOT / "spec" / "README.md"
CONTRACTS_README = REPO_ROOT / "contracts" / "README.md"
SPEC_CONTRACTS = REPO_ROOT / "spec" / "contracts"
RUNTIME_SCHEMAS = REPO_ROOT / "contracts" / "schema"


def schema_names(path: Path) -> set[str]:
    return {schema.name for schema in path.glob("*.schema.json")}


def main() -> int:
    errors: list[str] = []
    spec_text = SPEC_README.read_text(encoding="utf-8")
    contracts_text = CONTRACTS_README.read_text(encoding="utf-8")

    if "language-neutral source of truth" not in spec_text:
        errors.append("spec/README.md must identify spec/contracts as the source of truth")

    required_contracts_markers = (
        "This directory is **not** the canonical source of truth",
        "spec/contracts",
        "name exists in both directories",
        "`spec/contracts` owns the portable core",
    )
    for marker in required_contracts_markers:
        if marker not in contracts_text:
            errors.append(f"contracts/README.md missing marker: {marker}")

    forbidden_claims = (
        "They are the canonical, cross-language definition",
        "canonical, cross-language definition that every implementation",
    )
    for claim in forbidden_claims:
        if claim in contracts_text:
            errors.append(f"contracts/README.md still claims canonical ownership: {claim}")

    duplicate_names = sorted(schema_names(SPEC_CONTRACTS) & schema_names(RUNTIME_SCHEMAS))
    runtime_only = sorted(schema_names(RUNTIME_SCHEMAS) - schema_names(SPEC_CONTRACTS))

    if not duplicate_names:
        errors.append("expected at least one mirrored schema name to exercise canonicality guard")

    if errors:
        print("contract canonicality check failed:")
        for error in errors:
            print(f"  - {error}")
        return 1

    print(
        "contract canonicality check passed: "
        f"{len(duplicate_names)} mirrored schema(s), {len(runtime_only)} runtime-only schema(s)"
    )
    return 0


if __name__ == "__main__":
    raise SystemExit(main())
