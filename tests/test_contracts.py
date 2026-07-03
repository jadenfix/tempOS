"""Conformance tests for the beaterOS core data contracts.

These run under either `pytest` or the stdlib `unittest` runner so any reviewer
can execute them with zero third-party dependencies:

    python -m unittest discover -s tests
    pytest tests/

The negative cases matter as much as the positive ones: they prove the
validator actually rejects malformed instances instead of rubber-stamping them.
"""
from __future__ import annotations

import copy
import json
import sys
import unittest
from pathlib import Path

REPO_ROOT = Path(__file__).resolve().parent.parent
sys.path.insert(0, str(REPO_ROOT / "tools"))

import contracts_validate as cv  # noqa: E402


class SchemaWellFormednessTest(unittest.TestCase):
    def test_every_schema_is_valid_json_with_id_and_title(self) -> None:
        registry = cv.load_registry()
        self.assertIn("common.schema.json", registry)
        for name, doc in registry.items():
            with self.subTest(schema=name):
                self.assertIn("$schema", doc)
                self.assertIn("$id", doc)
                self.assertIn("title" if name != "common.schema.json" else "$defs", doc)

    def test_every_contract_has_an_example(self) -> None:
        registry = cv.load_registry()
        contract_schemas = {
            n for n in registry if n != "common.schema.json"
        }
        mapped_schemas = set(cv.EXAMPLE_TO_SCHEMA.values())
        self.assertEqual(contract_schemas, mapped_schemas)


class PositiveExampleTest(unittest.TestCase):
    def test_all_bundled_examples_validate(self) -> None:
        failures = cv.validate_all()
        self.assertEqual(failures, [], f"unexpected validation failures: {failures}")


class NegativeExampleTest(unittest.TestCase):
    """Mutate valid examples and assert the validator rejects each mutation."""

    def setUp(self) -> None:
        self.registry = cv.load_registry()

    def _load(self, example: str):
        instance = cv.load_json(cv.EXAMPLE_DIR / example)
        schema = self.registry[cv.EXAMPLE_TO_SCHEMA[example]]
        return instance, schema

    def _assert_rejects(self, instance, schema) -> None:
        with self.assertRaises(cv.ValidationError):
            cv.validate_instance(instance, schema, self.registry)

    def test_missing_required_field_rejected(self) -> None:
        instance, schema = self._load("agent_session.example.json")
        del instance["status"]
        self._assert_rejects(instance, schema)

    def test_bad_enum_value_rejected(self) -> None:
        instance, schema = self._load("agent_session.example.json")
        instance["status"] = "on_fire"
        self._assert_rejects(instance, schema)

    def test_bad_action_type_rejected(self) -> None:
        instance, schema = self._load("action_manifest.example.json")
        instance["action_type"] = "teleport"
        self._assert_rejects(instance, schema)

    def test_bad_timestamp_rejected(self) -> None:
        instance, schema = self._load("policy_decision.example.json")
        instance["created_at"] = "yesterday"
        self._assert_rejects(instance, schema)

    def test_bad_digest_pattern_rejected(self) -> None:
        instance, schema = self._load("capability_receipt.example.json")
        instance["receipt_hash"] = "not-a-digest"
        self._assert_rejects(instance, schema)

    def test_confidence_out_of_range_rejected(self) -> None:
        instance, schema = self._load("memory_record.example.json")
        instance["confidence"] = 1.5
        self._assert_rejects(instance, schema)

    def test_grant_with_empty_actions_rejected(self) -> None:
        instance, schema = self._load("capability_grant.example.json")
        instance["actions"] = []
        self._assert_rejects(instance, schema)

    def test_wrong_type_rejected(self) -> None:
        instance, schema = self._load("scenario_manifest.example.json")
        instance["success_criteria"] = "should-be-an-array"
        self._assert_rejects(instance, schema)

    def test_nested_selector_missing_field_rejected(self) -> None:
        instance, schema = self._load("action_manifest.example.json")
        del instance["target"]["resource_kind"]
        self._assert_rejects(instance, schema)

    def test_calendar_invalid_timestamp_rejected(self) -> None:
        instance, schema = self._load("policy_decision.example.json")
        for bad in ("2026-99-99T99:99:99Z", "2026-02-30T00:00:00Z"):
            with self.subTest(ts=bad):
                mutated = dict(instance, created_at=bad)
                self._assert_rejects(mutated, schema)

    def test_trailing_newline_in_digest_rejected(self) -> None:
        instance, schema = self._load("capability_receipt.example.json")
        instance["receipt_hash"] = "sha256:abcd\n"
        self._assert_rejects(instance, schema)


class InteropAcceptanceTest(unittest.TestCase):
    """Shapes the reference Rust crate emits must validate (interop floor)."""

    def setUp(self) -> None:
        self.registry = cv.load_registry()

    def test_policy_decision_accepts_review_handle_and_nulls(self) -> None:
        instance = cv.load_json(cv.EXAMPLE_DIR / "policy_decision.example.json")
        schema = self.registry["policy_decision.schema.json"]
        for value in ("review-123", None, True, False):
            with self.subTest(required_review=value):
                mutated = dict(instance, required_review=value)
                cv.validate_instance(mutated, schema, self.registry)

    def test_valid_offset_and_fractional_timestamp_accepted(self) -> None:
        instance = cv.load_json(cv.EXAMPLE_DIR / "policy_decision.example.json")
        schema = self.registry["policy_decision.schema.json"]
        mutated = dict(instance, created_at="2026-07-03T12:00:00.123+02:00")
        cv.validate_instance(mutated, schema, self.registry)


class ContractInvariantTest(unittest.TestCase):
    """Structural checks tying the examples back to final.md invariants."""

    def setUp(self) -> None:
        self.registry = cv.load_registry()

    def test_receipt_chain_hashes_present(self) -> None:
        receipt = cv.load_json(cv.EXAMPLE_DIR / "capability_receipt.example.json")
        # Genesis receipt: previous hash may be null but receipt_hash must exist.
        self.assertIn("receipt_hash", receipt)
        self.assertTrue(receipt["receipt_hash"].startswith("sha256:"))

    def test_payment_mandate_requires_receipts(self) -> None:
        mandate = cv.load_json(cv.EXAMPLE_DIR / "payment_mandate.example.json")
        self.assertTrue(
            mandate["receipt_requirement"],
            "final.md 12.7: all payment attempts must produce receipts",
        )

    def test_manifest_declares_only_known_side_effects(self) -> None:
        manifest = cv.load_json(cv.EXAMPLE_DIR / "action_manifest.example.json")
        allowed = set(
            self.registry["common.schema.json"]["$defs"]["SideEffectClass"]["enum"]
        )
        self.assertTrue(set(manifest["expected_side_effects"]).issubset(allowed))


if __name__ == "__main__":
    unittest.main()
