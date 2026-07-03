#!/usr/bin/env python3
"""Guards against a false-green gate.

A conformance gate that passes everything is worse than none. These negative
tests assert the gate actually *rejects* malformed data and *blocks* an obvious
attack -- so if the schema validator or admission port silently degrades to
"accept all", CI goes red here instead of giving false assurance.
"""

from __future__ import annotations

import sys
from pathlib import Path

import admission
from canonical import GENESIS_HASH, hash_preimage, sha256_hex
from journalcheck import verify_receipt_chain
from schema import SchemaRegistry, validate

SCHEMA_DIR = Path(__file__).resolve().parents[2] / "contracts" / "schema"


def _reg() -> SchemaRegistry:
    return SchemaRegistry().load_dir(SCHEMA_DIR)


def run() -> list[str]:
    reg = _reg()
    fails: list[str] = []

    def expect(cond: bool, label: str) -> None:
        if not cond:
            fails.append(label)

    # 1. Schema rejects a session missing required fields + wrong enum.
    bad_session = {"session_id": "x", "status": "not-a-real-status"}
    errs = validate(bad_session, "agent-session.schema.json", reg)
    expect(bool(errs), "schema should reject an invalid session")

    # 2. Schema rejects an unknown property (additionalProperties: false).
    bad_manifest = {
        "action_id": "a", "session_id": "S", "tool_id": "t", "action_kind": "read",
        "target": {"resource_kind": "file_path", "resource_id": "/x"},
        "inputs_digest": "d", "inputs_summary": "", "risk_class": "low",
        "human_explanation": "", "surprise_field": 1,
    }
    errs = validate(bad_manifest, "action-manifest.schema.json", reg)
    expect(bool(errs), "schema should reject an unknown property")

    # 3. Admission denies a session mismatch.
    manifest = {
        "action_id": "a", "session_id": "S1", "tool_id": "t", "action_kind": "read",
        "target": {"resource_kind": "file_path", "resource_id": "/x"},
        "inputs_digest": "d", "inputs_summary": "", "risk_class": "low", "human_explanation": "",
        "required_grants": ["g"],
    }
    ctx = {"now": "2026-07-03T00:00:00Z", "actor_id": "agent", "session_id": "S2",
           "policy_version": "p", "grants": [], "approvals": [], "simulations": []}
    expect(admission.admit(manifest, ctx)["result"] == "denied", "session mismatch should deny")

    # 4. Admission blocks untrusted-web spend without approval.
    spend = {
        "action_id": "s", "session_id": "S", "tool_id": "pay", "action_kind": "spend",
        "target": {"resource_kind": "payment_rail", "resource_id": "r"},
        "inputs_digest": "d", "inputs_summary": "", "risk_class": "high", "human_explanation": "",
        "required_grants": ["g"], "expected_side_effects": ["payment"], "idempotency_key": "i",
        "taint": ["untrusted_web"], "data_classes": ["financial"],
        "requested_budget": {"max_payment_minor_units": 1},
    }
    grant = {
        "grant_id": "g", "issuer": "u", "holder": "agent", "session_id": "S",
        "scope": {"selector": {"resource_kind": "payment_rail", "resource_id": "r"}, "actions": ["spend"]},
        "constraints": {"max_risk": "high", "max_data_class": "financial",
                        "budget": {"max_payment_minor_units": 1000}},
        "expires_at": "2026-07-03T01:00:00Z", "delegation": "none",
        "revocation_handle": "rev", "policy_version": "p", "reason": "",
    }
    ctx2 = {"now": "2026-07-03T00:30:00Z", "actor_id": "agent", "session_id": "S",
            "policy_version": "p", "grants": [grant], "approvals": [], "simulations": []}
    expect(admission.admit(spend, ctx2)["result"] == "needs_approval",
           "untrusted-web spend without approval should escalate")

    # 5. Receipt chain detects a tampered hash.
    r = {"receipt_id": "r", "seq": 0, "action_id": "a", "tool_id": "t",
         "target": {"resource_kind": "tool", "resource_id": "x"},
         "started_at": "2026-07-03T00:00:00Z", "finished_at": "2026-07-03T00:00:00Z",
         "status": "ok", "input_digest": "d", "output_digest": "o",
         "side_effect_summary": "", "prev_receipt_hash": GENESIS_HASH}
    r["receipt_hash"] = sha256_hex(hash_preimage(r, "receipt_hash"))
    expect(not verify_receipt_chain([r]), "valid receipt chain should pass")
    r["status"] = "tampered"
    expect(bool(verify_receipt_chain([r])), "tampered receipt should be detected")

    return fails


def main() -> int:
    fails = run()
    if fails:
        print("SELFTEST FAILED:")
        for f in fails:
            print(f"  - {f}")
        return 1
    print("selftest: gate rejects malformed data and blocks attacks (ok)")
    return 0


if __name__ == "__main__":
    raise SystemExit(main())
