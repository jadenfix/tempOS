#!/usr/bin/env python3
"""Validate beaterOS contract examples against the canonical JSON Schemas.

This is the conformance harness for the language-neutral contracts in
`contracts/schemas/` (final.md section 12). It is dependency-free: if the
`jsonschema` package is installed it is used as the authoritative Draft 2020-12
validator, otherwise a built-in validator covering the subset of JSON Schema
these contracts actually use is applied. Both paths must agree on the bundled
examples, which the test suite checks.

Usage:
    python tools/contracts_validate.py                 # validate every example
    python tools/contracts_validate.py --list          # list schema/example pairs
    python tools/contracts_validate.py FILE SCHEMA      # validate one instance

Exit code is non-zero if any instance fails to validate.
"""
from __future__ import annotations

import argparse
import json
import re
import sys
from datetime import datetime
from pathlib import Path

REPO_ROOT = Path(__file__).resolve().parent.parent
SCHEMA_DIR = REPO_ROOT / "contracts" / "schemas"
EXAMPLE_DIR = REPO_ROOT / "contracts" / "examples"

# example file -> schema file (both relative to their dirs)
EXAMPLE_TO_SCHEMA = {
    "agent_session.example.json": "agent_session.schema.json",
    "capability_grant.example.json": "capability_grant.schema.json",
    "action_manifest.example.json": "action_manifest.schema.json",
    "policy_decision.example.json": "policy_decision.schema.json",
    "capability_receipt.example.json": "capability_receipt.schema.json",
    "memory_record.example.json": "memory_record.schema.json",
    "payment_mandate.example.json": "payment_mandate.schema.json",
    "scenario_manifest.example.json": "scenario_manifest.schema.json",
}

_DATE_TIME_RE = re.compile(
    r"(?P<y>\d{4})-(?P<mo>\d{2})-(?P<d>\d{2})[Tt]"
    r"(?P<h>\d{2}):(?P<mi>\d{2}):(?P<s>\d{2})(?:\.\d+)?"
    r"(?:[Zz]|(?P<oh>[+-]\d{2}):(?P<om>\d{2}))"
)


def is_rfc3339_datetime(value) -> bool:
    """True if ``value`` is a calendar-valid RFC 3339 date-time.

    fullmatch rejects trailing whitespace/newlines, the field ranges reject
    things like ``2026-99-99``, and constructing a ``datetime`` rejects
    impossible dates like ``2026-02-30``. Second 60 (leap second) is allowed by
    the range check but not fed to ``datetime``.
    """
    if not isinstance(value, str):
        return True
    match = _DATE_TIME_RE.fullmatch(value)
    if match is None:
        return False
    year, month, day, hour, minute, second = (
        int(match[name]) for name in ("y", "mo", "d", "h", "mi", "s")
    )
    if not (1 <= month <= 12 and 1 <= day <= 31 and hour <= 23 and minute <= 59):
        return False
    if second > 60:
        return False
    try:
        datetime(year, month, day, hour, minute, min(second, 59))
    except ValueError:
        return False
    if match["oh"] is not None:
        if not (-23 <= int(match["oh"]) <= 23 and int(match["om"]) <= 59):
            return False
    return True


def load_json(path: Path):
    with path.open(encoding="utf-8") as handle:
        return json.load(handle)


def load_registry() -> dict[str, dict]:
    """Map schema basename -> parsed schema document for $ref resolution."""
    return {p.name: load_json(p) for p in sorted(SCHEMA_DIR.glob("*.schema.json"))}


class ValidationError(Exception):
    pass


class MiniValidator:
    """A small, correct validator for the JSON Schema subset used by beaterOS.

    Supported keywords: type, enum, const, required, properties,
    additionalProperties (bool), items, $ref (cross-file + JSON pointer),
    anyOf, allOf, minLength, minItems, minimum, maximum, pattern, format
    (date-time). Anything unsupported is treated as "no additional constraint"
    rather than silently passing something it should reject, so the test suite
    pins behaviour with explicit negative cases.
    """

    def __init__(self, registry: dict[str, dict]):
        self.registry = registry

    def validate(self, instance, schema: dict, root: dict, path: str = "$") -> None:
        if "$ref" in schema:
            target, target_root = self._resolve_ref(schema["$ref"], root)
            self.validate(instance, target, target_root, path)
            # A $ref may sit alongside sibling keywords (2020-12); apply them too.
            siblings = {k: v for k, v in schema.items() if k != "$ref"}
            if siblings:
                self.validate(instance, siblings, root, path)
            return

        if "type" in schema:
            self._check_type(instance, schema["type"], path)

        if "enum" in schema and instance not in schema["enum"]:
            raise ValidationError(
                f"{path}: {instance!r} is not one of {schema['enum']}"
            )

        if "const" in schema and instance != schema["const"]:
            raise ValidationError(f"{path}: {instance!r} != const {schema['const']!r}")

        if "anyOf" in schema:
            errors = []
            for i, sub in enumerate(schema["anyOf"]):
                try:
                    self.validate(instance, sub, root, f"{path}/anyOf/{i}")
                    break
                except ValidationError as exc:
                    errors.append(str(exc))
            else:
                raise ValidationError(f"{path}: no anyOf branch matched: {errors}")

        for sub in schema.get("allOf", []):
            self.validate(instance, sub, root, path)

        if isinstance(instance, str):
            self._check_string(instance, schema, path)
        if isinstance(instance, (int, float)) and not isinstance(instance, bool):
            self._check_number(instance, schema, path)
        if isinstance(instance, list):
            self._check_array(instance, schema, root, path)
        if isinstance(instance, dict):
            self._check_object(instance, schema, root, path)

    def _resolve_ref(self, ref: str, root: dict) -> tuple[dict, dict]:
        file_part, _, pointer = ref.partition("#")
        target_root = root
        if file_part:
            name = file_part.split("/")[-1]
            if name not in self.registry:
                raise ValidationError(f"unresolvable $ref file: {ref}")
            target_root = self.registry[name]
        node = target_root
        for token in filter(None, pointer.split("/")):
            token = token.replace("~1", "/").replace("~0", "~")
            if not isinstance(node, dict) or token not in node:
                raise ValidationError(f"unresolvable $ref pointer: {ref}")
            node = node[token]
        return node, target_root

    @staticmethod
    def _check_type(instance, type_spec, path: str) -> None:
        types = type_spec if isinstance(type_spec, list) else [type_spec]
        if not any(MiniValidator._is_type(instance, t) for t in types):
            raise ValidationError(
                f"{path}: expected type {type_spec}, got {type(instance).__name__}"
            )

    @staticmethod
    def _is_type(instance, t: str) -> bool:
        if t == "null":
            return instance is None
        if t == "boolean":
            return isinstance(instance, bool)
        if t == "integer":
            return isinstance(instance, int) and not isinstance(instance, bool)
        if t == "number":
            return isinstance(instance, (int, float)) and not isinstance(instance, bool)
        if t == "string":
            return isinstance(instance, str)
        if t == "array":
            return isinstance(instance, list)
        if t == "object":
            return isinstance(instance, dict)
        return False

    @staticmethod
    def _check_string(instance: str, schema: dict, path: str) -> None:
        if "minLength" in schema and len(instance) < schema["minLength"]:
            raise ValidationError(f"{path}: shorter than minLength {schema['minLength']}")
        if "pattern" in schema and not re.search(schema["pattern"], instance):
            raise ValidationError(f"{path}: {instance!r} does not match {schema['pattern']!r}")
        if schema.get("format") == "date-time" and not is_rfc3339_datetime(instance):
            raise ValidationError(f"{path}: {instance!r} is not an RFC 3339 date-time")

    @staticmethod
    def _check_number(instance, schema: dict, path: str) -> None:
        if "minimum" in schema and instance < schema["minimum"]:
            raise ValidationError(f"{path}: {instance} < minimum {schema['minimum']}")
        if "maximum" in schema and instance > schema["maximum"]:
            raise ValidationError(f"{path}: {instance} > maximum {schema['maximum']}")

    def _check_array(self, instance: list, schema: dict, root: dict, path: str) -> None:
        if "minItems" in schema and len(instance) < schema["minItems"]:
            raise ValidationError(f"{path}: fewer than minItems {schema['minItems']}")
        item_schema = schema.get("items")
        if isinstance(item_schema, dict):
            for i, item in enumerate(instance):
                self.validate(item, item_schema, root, f"{path}[{i}]")

    def _check_object(self, instance: dict, schema: dict, root: dict, path: str) -> None:
        for req in schema.get("required", []):
            if req not in instance:
                raise ValidationError(f"{path}: missing required property {req!r}")
        props = schema.get("properties", {})
        for key, value in instance.items():
            if key in props:
                self.validate(value, props[key], root, f"{path}.{key}")
            elif schema.get("additionalProperties") is False:
                raise ValidationError(f"{path}: additional property {key!r} is not allowed")


def _build_format_checker():
    import jsonschema  # type: ignore

    # jsonschema treats `format` as an annotation, not an assertion, unless a
    # FormatChecker is supplied -- and its built-in date-time check depends on
    # optional packages that may be absent in CI. Register our own so date-time
    # is asserted identically to the built-in validator, with no extra deps.
    checker = jsonschema.FormatChecker()

    @checker.checks("date-time")
    def _is_date_time(value) -> bool:  # noqa: ANN001
        return is_rfc3339_datetime(value)

    return checker


def _validate_with_jsonschema(instance, schema, registry) -> None:
    import jsonschema  # type: ignore
    from referencing import Registry, Resource  # type: ignore

    resources = [
        (doc.get("$id", name), Resource.from_contents(doc))
        for name, doc in registry.items()
    ]
    reg = Registry().with_resources(resources)
    # Also register schemas by bare filename so relative $refs resolve.
    reg = reg.with_resources(
        [(name, Resource.from_contents(doc)) for name, doc in registry.items()]
    )
    validator = jsonschema.Draft202012Validator(
        schema, registry=reg, format_checker=_build_format_checker()
    )
    errors = sorted(validator.iter_errors(instance), key=lambda e: str(e.path))
    if errors:
        raise ValidationError("; ".join(e.message for e in errors))


def validate_instance(instance, schema: dict, registry: dict[str, dict]) -> None:
    """Validate one instance, preferring `jsonschema` when it is importable."""
    try:
        _validate_with_jsonschema(instance, schema, registry)
        return
    except ImportError:
        pass
    MiniValidator(registry).validate(instance, schema, schema)


def validate_all() -> list[str]:
    registry = load_registry()
    failures: list[str] = []
    for example_name, schema_name in sorted(EXAMPLE_TO_SCHEMA.items()):
        schema = registry.get(schema_name)
        if schema is None:
            failures.append(f"{schema_name}: schema not found")
            continue
        instance = load_json(EXAMPLE_DIR / example_name)
        try:
            validate_instance(instance, schema, registry)
            print(f"ok   {example_name} -> {schema_name}")
        except ValidationError as exc:
            failures.append(f"{example_name}: {exc}")
            print(f"FAIL {example_name} -> {schema_name}: {exc}")
    return failures


def main(argv: list[str]) -> int:
    parser = argparse.ArgumentParser(description=__doc__)
    parser.add_argument("--list", action="store_true", help="list schema/example pairs")
    parser.add_argument("instance", nargs="?", help="path to a JSON instance")
    parser.add_argument("schema", nargs="?", help="path to a JSON Schema")
    args = parser.parse_args(argv)

    if args.list:
        for example, schema in sorted(EXAMPLE_TO_SCHEMA.items()):
            print(f"{example} -> {schema}")
        return 0

    if args.instance and args.schema:
        registry = load_registry()
        instance = load_json(Path(args.instance))
        schema = load_json(Path(args.schema))
        try:
            validate_instance(instance, schema, registry)
        except ValidationError as exc:
            print(f"FAIL {args.instance}: {exc}")
            return 1
        print(f"ok   {args.instance}")
        return 0

    failures = validate_all()
    if failures:
        print(f"\n{len(failures)} example(s) failed validation", file=sys.stderr)
        return 1
    print("\nall examples valid")
    return 0


if __name__ == "__main__":
    raise SystemExit(main(sys.argv[1:]))
