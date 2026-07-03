"""A small, dependency-free JSON Schema validator.

Supports exactly the subset used by `contracts/schema/*.json` (a pragmatic slice
of JSON Schema draft 2020-12): type, properties, required, additionalProperties,
enum, const, items, oneOf, allOf, $ref (intra- and cross-file), minimum,
maximum, minItems, minLength, pattern, and format: date-time.

Kept minimal on purpose: a full validator is a large dependency, and the schemas
here are authored to stay within this subset. If a schema uses a keyword this
validator does not implement, `UnsupportedKeyword` is raised loudly rather than
silently passing -- so the gate can never give false assurance.
"""

from __future__ import annotations

import json
import re
from pathlib import Path
from typing import Any

_DATE_TIME = re.compile(
    r"^\d{4}-\d{2}-\d{2}[Tt]\d{2}:\d{2}:\d{2}(\.\d+)?([Zz]|[+-]\d{2}:\d{2})$"
)

_SUPPORTED = {
    "$schema", "$id", "$defs", "$ref", "title", "description", "type",
    "properties", "required", "additionalProperties", "enum", "const",
    "items", "oneOf", "allOf", "minimum", "maximum", "minItems", "minLength",
    "pattern", "format", "uniqueItems", "examples", "default",
}


class UnsupportedKeyword(Exception):
    pass


class SchemaRegistry:
    """Loads every schema file in a directory, keyed by file basename."""

    def __init__(self) -> None:
        self.docs: dict[str, dict] = {}

    def load_dir(self, directory: str | Path) -> "SchemaRegistry":
        for path in sorted(Path(directory).glob("*.json")):
            self.docs[path.name] = json.loads(path.read_text())
        return self

    def resolve_ref(self, ref: str, current_file: str) -> dict:
        file_part, _, pointer = ref.partition("#")
        doc_name = file_part if file_part else current_file
        if doc_name not in self.docs:
            raise KeyError(f"$ref to unknown schema file: {doc_name!r} (from {current_file})")
        node: Any = self.docs[doc_name]
        for token in [t for t in pointer.split("/") if t]:
            token = token.replace("~1", "/").replace("~0", "~")
            node = node[token]
        return node, doc_name


def validate(instance: Any, schema_file: str, registry: SchemaRegistry) -> list[str]:
    """Validate `instance` against the top-level schema in `schema_file`."""
    schema = registry.docs[schema_file]
    errors: list[str] = []
    _validate(instance, schema, schema_file, registry, "$", errors)
    return errors


def _validate(inst, schema, cur_file, reg, path, errors) -> None:
    for kw in schema:
        if kw not in _SUPPORTED:
            raise UnsupportedKeyword(f"{cur_file}{path}: unsupported schema keyword {kw!r}")

    if "$ref" in schema:
        target, target_file = reg.resolve_ref(schema["$ref"], cur_file)
        _validate(inst, target, target_file, reg, path, errors)
        return

    if "allOf" in schema:
        for sub in schema["allOf"]:
            _validate(inst, sub, cur_file, reg, path, errors)

    if "oneOf" in schema:
        matches = 0
        collected: list[str] = []
        for sub in schema["oneOf"]:
            sub_errors: list[str] = []
            _validate(inst, sub, cur_file, reg, path, sub_errors)
            if not sub_errors:
                matches += 1
            else:
                collected.extend(sub_errors)
        if matches != 1:
            errors.append(f"{path}: matched {matches} of oneOf branches (expected exactly 1)")
        return

    if "const" in schema and inst != schema["const"]:
        errors.append(f"{path}: {inst!r} != const {schema['const']!r}")

    if "enum" in schema and inst not in schema["enum"]:
        errors.append(f"{path}: {inst!r} not in enum {schema['enum']}")

    t = schema.get("type")
    if t is not None and not _type_ok(inst, t):
        errors.append(f"{path}: expected type {t}, got {_typename(inst)}")
        return  # further checks assume the type held

    if isinstance(inst, str):
        if "minLength" in schema and len(inst) < schema["minLength"]:
            errors.append(f"{path}: string shorter than minLength {schema['minLength']}")
        if "pattern" in schema and not re.search(schema["pattern"], inst):
            errors.append(f"{path}: {inst!r} does not match pattern {schema['pattern']!r}")
        if schema.get("format") == "date-time" and not _DATE_TIME.match(inst):
            errors.append(f"{path}: {inst!r} is not an RFC3339 date-time")

    if isinstance(inst, bool):
        pass  # bool is not treated as a number below
    elif isinstance(inst, (int, float)):
        if "minimum" in schema and inst < schema["minimum"]:
            errors.append(f"{path}: {inst} < minimum {schema['minimum']}")
        if "maximum" in schema and inst > schema["maximum"]:
            errors.append(f"{path}: {inst} > maximum {schema['maximum']}")

    if isinstance(inst, list):
        if "minItems" in schema and len(inst) < schema["minItems"]:
            errors.append(f"{path}: array shorter than minItems {schema['minItems']}")
        if schema.get("uniqueItems") and _has_dupes(inst):
            errors.append(f"{path}: array items are not unique")
        if "items" in schema:
            for i, item in enumerate(inst):
                _validate(item, schema["items"], cur_file, reg, f"{path}[{i}]", errors)

    if isinstance(inst, dict):
        props = schema.get("properties", {})
        for req in schema.get("required", []):
            if req not in inst:
                errors.append(f"{path}: missing required property {req!r}")
        addl = schema.get("additionalProperties", True)
        for key, value in inst.items():
            if key in props:
                _validate(value, props[key], cur_file, reg, f"{path}.{key}", errors)
            elif addl is False:
                errors.append(f"{path}: additional property {key!r} not allowed")
            elif isinstance(addl, dict):
                _validate(value, addl, cur_file, reg, f"{path}.{key}", errors)


def _type_ok(inst, t) -> bool:
    if isinstance(t, list):
        return any(_type_ok(inst, one) for one in t)
    if t == "object":
        return isinstance(inst, dict)
    if t == "array":
        return isinstance(inst, list)
    if t == "string":
        return isinstance(inst, str)
    if t == "integer":
        return isinstance(inst, int) and not isinstance(inst, bool)
    if t == "number":
        return isinstance(inst, (int, float)) and not isinstance(inst, bool)
    if t == "boolean":
        return isinstance(inst, bool)
    if t == "null":
        return inst is None
    raise UnsupportedKeyword(f"unknown type {t!r}")


def _typename(inst) -> str:
    if isinstance(inst, bool):
        return "boolean"
    if isinstance(inst, str):
        return "string"
    if isinstance(inst, int):
        return "integer"
    if isinstance(inst, float):
        return "number"
    if isinstance(inst, list):
        return "array"
    if isinstance(inst, dict):
        return "object"
    if inst is None:
        return "null"
    return type(inst).__name__


def _has_dupes(items) -> bool:
    seen = []
    for item in items:
        if item in seen:
            return True
        seen.append(item)
    return False
