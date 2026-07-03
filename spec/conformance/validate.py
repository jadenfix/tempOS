#!/usr/bin/env python3
"""beaterOS contract conformance runner.

This is a dependency-free JSON Schema (draft 2020-12 subset) validator plus a
conformance runner for the beaterOS core data contracts under spec/contracts.

Why dependency-free: the conformance suite is the shared, language-neutral
source of truth for every beaterOS implementation (Rust today, others later).
It must run in CI and on any contributor's machine with only a stock Python 3,
no network and no pip install.

Usage:
    python3 spec/conformance/validate.py            # run the whole suite
    python3 spec/conformance/validate.py --quiet     # only print failures + summary

Layout it expects (all paths relative to the repo root):
    spec/contracts/<name>.schema.json            normative schemas
    spec/examples/valid/<name>/*.json            instances that MUST validate
    spec/examples/invalid/<name>/*.json          instances that MUST NOT validate

<name> is the schema file's basename without ".schema.json" (e.g. "agent-session").

Exit code is 0 iff every valid example passes, every invalid example fails, and
every contract schema has at least one valid and one invalid example.
"""

from __future__ import annotations

import json
import re
import sys
from pathlib import Path

# ----------------------------------------------------------------------------
# Paths
# ----------------------------------------------------------------------------

SPEC_DIR = Path(__file__).resolve().parent.parent
CONTRACTS_DIR = SPEC_DIR / "contracts"
VALID_DIR = SPEC_DIR / "examples" / "valid"
INVALID_DIR = SPEC_DIR / "examples" / "invalid"

# RFC 3339 date-time, the shape serde/chrono emits for DateTime<Utc>.
_DATETIME_RE = re.compile(
    r"^\d{4}-\d{2}-\d{2}[Tt]\d{2}:\d{2}:\d{2}(\.\d+)?([Zz]|[+-]\d{2}:\d{2})$"
)


class SchemaError(Exception):
    """Raised when a schema itself is malformed (a bug in the spec, not the data)."""


class Validator:
    """A small JSON Schema validator covering exactly the keywords the beaterOS
    contract schemas use. It is deliberately strict and explicit rather than a
    general-purpose implementation, so failures are easy to read and the code is
    easy for any reviewer to audit."""

    def __init__(self, registry: dict[str, dict]):
        # registry maps a schema $id (its filename, e.g. "common.schema.json")
        # to the parsed schema document.
        self.registry = registry

    # -- reference resolution -------------------------------------------------

    def _resolve(self, ref: str, base_id: str) -> tuple[dict, str]:
        """Resolve a $ref into (target_schema, new_base_id).

        Refs are either "#/pointer" (within the current document) or
        "file.schema.json#/pointer" (a named document in the registry)."""
        doc_part, _, pointer = ref.partition("#")
        doc_id = doc_part if doc_part else base_id
        if doc_id not in self.registry:
            raise SchemaError(f"$ref to unknown document {doc_id!r} (from {ref!r})")
        node = self.registry[doc_id]
        for raw in pointer.split("/"):
            if raw == "":
                continue
            token = raw.replace("~1", "/").replace("~0", "~")
            if not isinstance(node, dict) or token not in node:
                raise SchemaError(f"$ref pointer {ref!r} does not resolve")
            node = node[token]
        if not isinstance(node, dict):
            raise SchemaError(f"$ref {ref!r} does not point at a schema object")
        return node, doc_id

    # -- validation -----------------------------------------------------------

    def validate(self, instance, schema: dict, base_id: str, path: str, errors: list):
        if "$ref" in schema:
            target, new_base = self._resolve(schema["$ref"], base_id)
            self.validate(instance, target, new_base, path, errors)
            # A $ref node in these schemas never carries sibling keywords, so we
            # stop here rather than merging (draft 2020-12 would allow siblings).
            return

        # Combinators.
        if "oneOf" in schema:
            matches = 0
            for sub in schema["oneOf"]:
                trial: list = []
                self.validate(instance, sub, base_id, path, trial)
                if not trial:
                    matches += 1
            if matches != 1:
                errors.append(f"{path}: matched {matches} of oneOf branches (expected exactly 1)")
                return
        if "anyOf" in schema:
            if not any(self._matches(instance, sub, base_id) for sub in schema["anyOf"]):
                errors.append(f"{path}: did not match any anyOf branch")
                return
        if "allOf" in schema:
            for sub in schema["allOf"]:
                self.validate(instance, sub, base_id, path, errors)

        if "const" in schema and instance != schema["const"]:
            errors.append(f"{path}: expected const {schema['const']!r}, got {instance!r}")
            return
        if "enum" in schema and instance not in schema["enum"]:
            errors.append(f"{path}: {instance!r} is not one of {schema['enum']}")
            return

        # Type check.
        if "type" in schema:
            types = schema["type"]
            types = [types] if isinstance(types, str) else types
            if not any(_is_type(instance, t) for t in types):
                got = _typename(instance)
                errors.append(f"{path}: expected type {types}, got {got} ({instance!r})")
                return

        if isinstance(instance, str):
            self._check_string(instance, schema, path, errors)
        if isinstance(instance, bool):
            pass  # bools carry no further constraints in these schemas
        elif isinstance(instance, (int, float)):
            self._check_number(instance, schema, path, errors)
        if isinstance(instance, list):
            self._check_array(instance, schema, base_id, path, errors)
        if isinstance(instance, dict):
            self._check_object(instance, schema, base_id, path, errors)

    def _matches(self, instance, schema: dict, base_id: str) -> bool:
        trial: list = []
        self.validate(instance, schema, base_id, "", trial)
        return not trial

    def _check_string(self, instance: str, schema: dict, path: str, errors: list):
        if "minLength" in schema and len(instance) < schema["minLength"]:
            errors.append(f"{path}: string shorter than minLength {schema['minLength']}")
        if "maxLength" in schema and len(instance) > schema["maxLength"]:
            errors.append(f"{path}: string longer than maxLength {schema['maxLength']}")
        if "pattern" in schema and not re.search(schema["pattern"], instance):
            errors.append(f"{path}: {instance!r} does not match pattern {schema['pattern']!r}")
        if schema.get("format") == "date-time" and not _DATETIME_RE.match(instance):
            errors.append(f"{path}: {instance!r} is not an RFC 3339 date-time")

    def _check_number(self, instance, schema: dict, path: str, errors: list):
        if "minimum" in schema and instance < schema["minimum"]:
            errors.append(f"{path}: {instance} is below minimum {schema['minimum']}")
        if "maximum" in schema and instance > schema["maximum"]:
            errors.append(f"{path}: {instance} is above maximum {schema['maximum']}")

    def _check_array(self, instance: list, schema: dict, base_id: str, path: str, errors: list):
        if "minItems" in schema and len(instance) < schema["minItems"]:
            errors.append(f"{path}: array has fewer than minItems {schema['minItems']}")
        if "maxItems" in schema and len(instance) > schema["maxItems"]:
            errors.append(f"{path}: array has more than maxItems {schema['maxItems']}")
        if schema.get("uniqueItems") and not _all_unique(instance):
            errors.append(f"{path}: array items are not unique")
        if "items" in schema:
            for i, item in enumerate(instance):
                self.validate(item, schema["items"], base_id, f"{path}[{i}]", errors)

    def _check_object(self, instance: dict, schema: dict, base_id: str, path: str, errors: list):
        props = schema.get("properties", {})
        for req in schema.get("required", []):
            if req not in instance:
                errors.append(f"{path}: missing required property {req!r}")
        additional = schema.get("additionalProperties", True)
        pattern_props = schema.get("patternProperties", {})
        for key, value in instance.items():
            child = f"{path}.{key}" if path else key
            if key in props:
                self.validate(value, props[key], base_id, child, errors)
                continue
            matched = False
            for pat, subschema in pattern_props.items():
                if re.search(pat, key):
                    self.validate(value, subschema, base_id, child, errors)
                    matched = True
            if matched:
                continue
            if additional is False:
                errors.append(f"{path}: unexpected property {key!r}")
            elif isinstance(additional, dict):
                self.validate(value, additional, base_id, child, errors)


# ----------------------------------------------------------------------------
# Type helpers
# ----------------------------------------------------------------------------

def _is_type(instance, t: str) -> bool:
    if t == "null":
        return instance is None
    if t == "boolean":
        return isinstance(instance, bool)
    if t == "integer":
        # In JSON, `true`/`false` are booleans, never integers.
        return isinstance(instance, int) and not isinstance(instance, bool)
    if t == "number":
        return isinstance(instance, (int, float)) and not isinstance(instance, bool)
    if t == "string":
        return isinstance(instance, str)
    if t == "array":
        return isinstance(instance, list)
    if t == "object":
        return isinstance(instance, dict)
    raise SchemaError(f"unknown type keyword {t!r}")


def _typename(instance) -> str:
    if instance is None:
        return "null"
    if isinstance(instance, bool):
        return "boolean"
    if isinstance(instance, int):
        return "integer"
    if isinstance(instance, float):
        return "number"
    if isinstance(instance, str):
        return "string"
    if isinstance(instance, list):
        return "array"
    if isinstance(instance, dict):
        return "object"
    return type(instance).__name__


def _all_unique(items: list) -> bool:
    seen = []
    for item in items:
        canonical = json.dumps(item, sort_keys=True)
        if canonical in seen:
            return False
        seen.append(canonical)
    return True


# ----------------------------------------------------------------------------
# Suite loading + running
# ----------------------------------------------------------------------------

def load_registry() -> dict[str, dict]:
    registry: dict[str, dict] = {}
    for schema_path in sorted(CONTRACTS_DIR.glob("*.schema.json")):
        doc = json.loads(schema_path.read_text())
        schema_id = doc.get("$id")
        if schema_id != schema_path.name:
            raise SchemaError(
                f"{schema_path.name}: $id must equal its filename, got {schema_id!r}"
            )
        registry[schema_id] = doc
    if not registry:
        raise SchemaError(f"no schemas found under {CONTRACTS_DIR}")
    return registry


def schema_basename(schema_id: str) -> str:
    return schema_id[: -len(".schema.json")]


def _examples_for(base: str, root: Path) -> list[Path]:
    directory = root / base
    if not directory.is_dir():
        return []
    return sorted(directory.glob("*.json"))


def run(quiet: bool = False) -> int:
    registry = load_registry()
    validator = Validator(registry)

    contract_ids = [sid for sid in registry if sid != "common.schema.json"]
    passes = 0
    failures: list[str] = []

    for schema_id in sorted(contract_ids):
        base = schema_basename(schema_id)
        schema = registry[schema_id]

        valid_examples = _examples_for(base, VALID_DIR)
        invalid_examples = _examples_for(base, INVALID_DIR)

        if not valid_examples:
            failures.append(f"[coverage] {base}: no valid examples under examples/valid/{base}/")
        if not invalid_examples:
            failures.append(f"[coverage] {base}: no invalid examples under examples/invalid/{base}/")

        for example in valid_examples:
            errors: list = []
            instance = json.loads(example.read_text())
            validator.validate(instance, schema, schema_id, "$", errors)
            rel = example.relative_to(SPEC_DIR)
            if errors:
                failures.append(f"[valid MUST pass] {rel}:\n    " + "\n    ".join(errors))
            else:
                passes += 1
                if not quiet:
                    print(f"  ok   {rel}")

        for example in invalid_examples:
            errors = []
            instance = json.loads(example.read_text())
            validator.validate(instance, schema, schema_id, "$", errors)
            rel = example.relative_to(SPEC_DIR)
            if not errors:
                failures.append(
                    f"[invalid MUST fail] {rel}: validated successfully but should have been rejected"
                )
            else:
                passes += 1
                if not quiet:
                    reason = errors[0].split("\n")[0]
                    print(f"  ok   {rel}  (rejected: {reason})")

    print()
    if failures:
        print(f"FAIL: {len(failures)} problem(s), {passes} check(s) passed\n")
        for f in failures:
            print(f"  - {f}")
        return 1
    print(f"PASS: {passes} conformance check(s) across {len(contract_ids)} contract(s)")
    return 0


def main(argv: list[str]) -> int:
    quiet = "--quiet" in argv[1:]
    try:
        return run(quiet=quiet)
    except SchemaError as exc:
        print(f"SCHEMA ERROR: {exc}", file=sys.stderr)
        return 2


if __name__ == "__main__":
    raise SystemExit(main(sys.argv))
