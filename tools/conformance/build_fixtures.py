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

FIXTURE = Path(__file__).resolve().parents[2] / "examples" / "traces" / "coding-workflow.trace.json"

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


def main() -> int:
    bundle = build_bundle()
    payload = _serialize(bundle)
    if "--check" in sys.argv:
        if not FIXTURE.exists():
            print(f"FAIL: fixture missing: {FIXTURE}")
            return 1
        current = FIXTURE.read_text()
        if current != payload:
            print(f"FAIL: {FIXTURE} is out of date; run build_fixtures.py without --check")
            return 1
        print(f"ok: {FIXTURE.name} reproduces from build_fixtures.py")
        return 0
    FIXTURE.parent.mkdir(parents=True, exist_ok=True)
    FIXTURE.write_text(payload)
    print(f"wrote {FIXTURE}")
    return 0


if __name__ == "__main__":
    raise SystemExit(main())
