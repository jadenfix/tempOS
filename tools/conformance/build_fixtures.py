#!/usr/bin/env python3
"""Deterministically build the golden trace fixture with valid hash chains.

Generating the fixture from code (rather than hand-writing 14 linked hashes)
keeps it reproducible and internally consistent: `--check` regenerates and
compares against the committed file so it can never silently drift.

The trace is the `final.md` §24 MVP proof, made concrete: an agent reads a file
(allowed), is blocked when it tries to write outside its granted path
(needs_narrowed_grant), writes the fix inside scope (allowed), and runs the test
runner (allowed) -- every side effect carrying a hash-linked receipt, every
step journaled with causality.
"""

from __future__ import annotations

import json
import sys
from pathlib import Path

from canonical import GENESIS_HASH, hash_preimage, sha256_hex

TRACES = Path(__file__).resolve().parents[2] / "examples" / "traces"
FIXTURE = TRACES / "coding-workflow.trace.json"

POLICY_VERSION = "policy-2026-07-03.dev"
AGENT = "agent:coder-1"
SESSION_ID = "sess-coding-001"
T = "2026-07-03T18:0{}:00Z"  # minute slots 0-9


def build_bundle() -> dict:
    session = {
        "session_id": SESSION_ID,
        "created_at": T.format(0),
        "created_by": "user:jaden",
        "agent_id": AGENT,
        "workspace_id": "ws:beateros",
        "goal": "Fix the failing parser test and run the suite.",
        "constraints": ["no network without a grant", "no push to origin"],
        "policy_profile": "developer-default",
        "initial_capability_ids": ["grant-repo-rw", "grant-test-exec"],
        "budget": {"max_model_cents": 5000, "max_tool_calls": 50, "max_wall_ms": 600000},
        "journal_root": GENESIS_HASH,
        "status": "completed",
    }

    grant_rw = {
        "grant_id": "grant-repo-rw",
        "issuer": "user:jaden",
        "holder": AGENT,
        "session_id": SESSION_ID,
        "scope": {"selector": {"resource_kind": "file_path", "resource_id": "*"}, "actions": ["read", "write"]},
        "constraints": {"max_risk": "medium", "max_data_class": "code", "path_prefixes": ["/ws/beateros"]},
        "expires_at": "2026-07-03T20:00:00Z",
        "delegation": "none",
        "revocation_handle": "rev:grant-repo-rw",
        "policy_version": POLICY_VERSION,
        "reason": "Edit repository files inside the workspace to fix the parser bug.",
    }
    grant_exec = {
        "grant_id": "grant-test-exec",
        "issuer": "user:jaden",
        "holder": AGENT,
        "session_id": SESSION_ID,
        "scope": {"selector": {"resource_kind": "tool", "resource_id": "test-runner"}, "actions": ["execute"]},
        "constraints": {"max_risk": "medium", "max_data_class": "code"},
        "expires_at": "2026-07-03T20:00:00Z",
        "delegation": "none",
        "revocation_handle": "rev:grant-test-exec",
        "policy_version": POLICY_VERSION,
        "reason": "Run the sandboxed test runner to verify the fix.",
    }

    m_read = {
        "action_id": "act-read-parser",
        "session_id": SESSION_ID,
        "tool_id": "fs-read",
        "action_kind": "read",
        "target": {"resource_kind": "file_path", "resource_id": "/ws/beateros/src/parser.rs"},
        "resolved_target": {"resource_kind": "file_path", "resource_id": "/ws/beateros/src/parser.rs"},
        "inputs_digest": "sha256:in-read-parser",
        "inputs_summary": "Read src/parser.rs to locate the bug.",
        "expected_side_effects": ["none"],
        "required_grants": ["grant-repo-rw"],
        "risk_class": "low",
        "data_classes": ["code"],
        "human_explanation": "Read the parser source before editing.",
    }
    m_escape = {
        "action_id": "act-write-escape",
        "session_id": SESSION_ID,
        "tool_id": "fs-write",
        "action_kind": "write",
        "target": {"resource_kind": "file_path", "resource_id": "/etc/passwd"},
        "resolved_target": {"resource_kind": "file_path", "resource_id": "/etc/passwd"},
        "inputs_digest": "sha256:in-write-escape",
        "inputs_summary": "Attempt to write outside the granted workspace prefix.",
        "expected_side_effects": ["local_write"],
        "required_grants": ["grant-repo-rw"],
        "risk_class": "medium",
        "data_classes": ["code"],
        "human_explanation": "Blocked example: path escapes the grant's /ws/beateros prefix.",
    }
    m_fix = {
        "action_id": "act-write-fix",
        "session_id": SESSION_ID,
        "tool_id": "fs-write",
        "action_kind": "write",
        "target": {"resource_kind": "file_path", "resource_id": "/ws/beateros/src/parser.rs"},
        "resolved_target": {"resource_kind": "file_path", "resource_id": "/ws/beateros/src/parser.rs"},
        "inputs_digest": "sha256:in-write-fix",
        "inputs_summary": "Apply the off-by-one fix in src/parser.rs.",
        "expected_side_effects": ["local_write"],
        "required_grants": ["grant-repo-rw"],
        "risk_class": "medium",
        "data_classes": ["code"],
        "human_explanation": "Write the fix inside the granted workspace path.",
    }
    m_test = {
        "action_id": "act-run-tests",
        "session_id": SESSION_ID,
        "tool_id": "test-runner",
        "action_kind": "execute",
        "target": {"resource_kind": "tool", "resource_id": "test-runner"},
        "inputs_digest": "sha256:in-run-tests",
        "inputs_summary": "Run the sandboxed test suite.",
        "expected_side_effects": ["none"],
        "required_grants": ["grant-test-exec"],
        "risk_class": "medium",
        "data_classes": ["code"],
        "human_explanation": "Execute the test runner to confirm the fix.",
    }

    d_read = _decision("dec-read", "act-read-parser", "allowed", T.format(1),
                       "action admitted by explicit active capability grant")
    d_escape = _decision("dec-escape", "act-write-escape", "needs_narrowed_grant", T.format(2),
                         "available grants do not allow this action, target, risk, data class, or time window")
    d_fix = _decision("dec-fix", "act-write-fix", "allowed", T.format(3),
                      "action admitted by explicit active capability grant")
    d_test = _decision("dec-test", "act-run-tests", "allowed", T.format(4),
                       "action admitted by explicit active capability grant")

    # Receipts (only for allowed, executed actions), hash-linked in order.
    receipts = _chain_receipts([
        _receipt("rcpt-read", 0, m_read, T.format(1), "read 1180 bytes from src/parser.rs", []),
        _receipt("rcpt-fix", 1, m_fix, T.format(3), "wrote 1204 bytes to src/parser.rs", ["local_write"]),
        _receipt("rcpt-test", 2, m_test, T.format(4), "ran 42 tests, 42 passed", []),
    ])

    events = [
        {"kind": "session_created", "session": session},
        {"kind": "capability_granted", "grant": grant_rw},
        {"kind": "capability_granted", "grant": grant_exec},
        {"kind": "action_proposed", "manifest": m_read},
        {"kind": "policy_decided", "decision": d_read},
        {"kind": "receipt_appended", "receipt": receipts[0]},
        {"kind": "action_proposed", "manifest": m_escape},
        {"kind": "policy_decided", "decision": d_escape},
        {"kind": "action_proposed", "manifest": m_fix},
        {"kind": "policy_decided", "decision": d_fix},
        {"kind": "receipt_appended", "receipt": receipts[1]},
        {"kind": "action_proposed", "manifest": m_test},
        {"kind": "policy_decided", "decision": d_test},
        {"kind": "receipt_appended", "receipt": receipts[2]},
    ]
    times = [T.format(0), T.format(0), T.format(0), T.format(1), T.format(1), T.format(1),
             T.format(2), T.format(2), T.format(3), T.format(3), T.format(3),
             T.format(4), T.format(4), T.format(4)]
    journal = _chain_journal(events, times)

    return {
        "bundle_id": "coding-workflow-mvp",
        "description": "final.md §24 MVP proof: granted repo read/edit/test with a blocked out-of-scope write, full receipts and journal.",
        "policy_version": POLICY_VERSION,
        "sessions": [session],
        "grants": [grant_rw, grant_exec],
        "manifests": [m_read, m_escape, m_fix, m_test],
        "decisions": [d_read, d_escape, d_fix, d_test],
        "receipts": receipts,
        "journal": journal,
    }


def build_payment_bundle() -> dict:
    """A bounded-payment workflow exercising the approval-SATISFIED -> allowed path.

    The coding trace only shows approval being *required*. This one shows the
    other half of final.md's human-review design (§7.9, §13.14, §16.1): a spend
    over the grant's approval threshold is admitted only *because* valid,
    action-bound human approval evidence exists, and it carries a PaymentMandate
    (§12.7) plus a payment receipt.
    """
    sid = "sess-pay-001"
    agent = "agent:ap-1"
    tp = "2026-07-03T09:0{}:00Z"

    session = {
        "session_id": sid,
        "created_at": tp.format(0),
        "created_by": "user:jaden",
        "agent_id": agent,
        "workspace_id": "ws:finance",
        "goal": "Pay an approved vendor invoice within the standing mandate.",
        "constraints": ["no payment above the mandate ceiling", "human approval over threshold"],
        "policy_profile": "finance-default",
        "initial_capability_ids": ["grant-vendor-spend"],
        "budget": {"max_payment_minor_units": 100000},
        "journal_root": GENESIS_HASH,
        "status": "completed",
    }

    mandate = {
        "mandate_id": "mandate-vendor",
        "issuer": "user:jaden",
        "holder": agent,
        "session_id": sid,
        "rail": "stripe",
        "asset": "USD",
        "max_minor_units": 100000,
        "counterparty_policy": "allowlist:approved-vendors",
        "purpose": "Settle approved vendor invoices.",
        "expires_at": "2026-07-03T12:00:00Z",
        "approval_threshold_minor_units": 5000,
        "idempotency_key": "idem-mandate-vendor",
        "receipt_requirement": "required",
    }

    grant = {
        "grant_id": "grant-vendor-spend",
        "issuer": "user:jaden",
        "holder": agent,
        "session_id": sid,
        "scope": {"selector": {"resource_kind": "payment_rail", "resource_id": "stripe"}, "actions": ["spend"]},
        "constraints": {"max_risk": "high", "max_data_class": "financial",
                        "budget": {"max_payment_minor_units": 100000}},
        "approval": {"mode": "human", "threshold_risk": "medium", "reviewer_ids": ["user:jaden"]},
        "expires_at": "2026-07-03T12:00:00Z",
        "delegation": "none",
        "revocation_handle": "rev:grant-vendor-spend",
        "policy_version": POLICY_VERSION,
        "reason": "Pay approved vendor invoices via the standing mandate.",
    }

    m_pay = {
        "action_id": "act-pay-invoice",
        "session_id": sid,
        "tool_id": "payment",
        "action_kind": "spend",
        "target": {"resource_kind": "payment_rail", "resource_id": "stripe"},
        "inputs_digest": "sha256:in-pay-invoice",
        "inputs_summary": "Charge 6200 USD minor units to an approved vendor.",
        "expected_side_effects": ["payment"],
        "required_grants": ["grant-vendor-spend"],
        "requested_budget": {"max_payment_minor_units": 6200},
        "risk_class": "medium",
        "data_classes": ["financial"],
        "taint": ["payment_instruction"],
        "idempotency_key": "idem-pay-invoice",
        "human_explanation": "Pay the approved vendor invoice #4471.",
    }

    approval = {
        "review_id": "review-pay",
        "action_id": "act-pay-invoice",
        "grant_id": "grant-vendor-spend",
        "reviewer_id": "user:jaden",
        "approved_at": tp.format(1),
        "policy_version": POLICY_VERSION,
    }

    d_pay = _decision("dec-pay", "act-pay-invoice", "allowed", tp.format(2),
                      "action admitted by explicit active capability grant with valid human approval")

    receipts = _chain_receipts([
        _receipt("rcpt-pay", 0, m_pay, tp.format(2),
                 "charged 6200 USD minor units to approved vendor #4471", ["payment"]),
    ])
    receipts[0]["external_ids"] = ["stripe:ch_test_4471"]
    # Re-hash after adding external_ids so the chain stays valid.
    receipts[0].pop("receipt_hash")
    receipts[0]["receipt_hash"] = sha256_hex(hash_preimage(receipts[0], "receipt_hash"))

    events = [
        {"kind": "session_created", "session": session},
        {"kind": "capability_granted", "grant": grant},
        {"kind": "action_proposed", "manifest": m_pay},
        {"kind": "policy_decided", "decision": d_pay},
        {"kind": "receipt_appended", "receipt": receipts[0]},
    ]
    times = [tp.format(0), tp.format(0), tp.format(1), tp.format(2), tp.format(2)]
    journal = _chain_journal(events, times)

    return {
        "bundle_id": "payment-workflow",
        "description": "final.md §16.1/§13.14: bounded vendor payment admitted only because valid human approval evidence exists; carries a PaymentMandate and a payment receipt.",
        "policy_version": POLICY_VERSION,
        "sessions": [session],
        "payment_mandates": [mandate],
        "grants": [grant],
        "approvals": [approval],
        "manifests": [m_pay],
        "decisions": [d_pay],
        "receipts": receipts,
        "journal": journal,
    }


def _decision(did, aid, result, created_at, explanation):
    return {
        "decision_id": did,
        "action_id": aid,
        "policy_version": POLICY_VERSION,
        "result": result,
        "explanation": explanation,
        "created_at": created_at,
    }


def _receipt(rid, seq, manifest, finished_at, summary, side_effects):
    target = manifest.get("resolved_target") or manifest["target"]
    return {
        "receipt_id": rid,
        "seq": seq,
        "action_id": manifest["action_id"],
        "tool_id": manifest["tool_id"],
        "target": target,
        "started_at": finished_at,
        "finished_at": finished_at,
        "status": "ok",
        "input_digest": manifest["inputs_digest"],
        "output_digest": "sha256:out-" + manifest["action_id"],
        "side_effect_summary": summary,
        "side_effects": side_effects,
    }


def _chain_receipts(receipts):
    prev = GENESIS_HASH
    for r in receipts:
        r["prev_receipt_hash"] = prev
        r["receipt_hash"] = sha256_hex(hash_preimage(r, "receipt_hash"))
        prev = r["receipt_hash"]
    return receipts


def _chain_journal(events, times):
    records = []
    prev = GENESIS_HASH
    for seq, (event, created_at) in enumerate(zip(events, times)):
        rec = {"seq": seq, "created_at": created_at, "event": event, "prev_hash": prev}
        rec["hash"] = sha256_hex(hash_preimage(rec, "hash"))
        prev = rec["hash"]
        records.append(rec)
    return records


def _serialize(bundle) -> str:
    return json.dumps(bundle, indent=2, ensure_ascii=False) + "\n"


FIXTURES = {
    "coding-workflow.trace.json": build_bundle,
    "payment-workflow.trace.json": build_payment_bundle,
}


def main() -> int:
    check = "--check" in sys.argv
    failed = False
    for name, builder in FIXTURES.items():
        path = TRACES / name
        payload = _serialize(builder())
        if check:
            if not path.exists() or path.read_text() != payload:
                print(f"FAIL: {name} is missing or out of date; run build_fixtures.py without --check")
                failed = True
            else:
                print(f"ok: {name} reproduces from build_fixtures.py")
        else:
            path.parent.mkdir(parents=True, exist_ok=True)
            path.write_text(payload)
            print(f"wrote {path}")
    return 1 if failed else 0


if __name__ == "__main__":
    raise SystemExit(main())
