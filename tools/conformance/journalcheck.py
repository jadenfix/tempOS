"""Journal + receipt chain and causality verification.

Independent port of the hash-chain and causality logic in
`crates/beater-os-core/src/journal.rs` and `receipt.rs`. Verifies the
language-neutral invariants any conformant implementation must satisfy:

- Receipts and journal records form a hash-linked chain: seq starts at 0 and is
  contiguous, each `prev_*` equals the previous element's hash, and each hash
  recomputes over the element's canonical preimage.
- Journal causality (`final.md` §4.5, §5.5, §26): session lifecycle transitions
  must follow the legal state machine; a policy decision must bind to the exact
  proposed manifest digest; a receipt may only appear after its
  action was proposed AND that action's latest policy decision was `allowed`;
  and the receipt must be bound to the manifest's tool, input digest, target,
  and declared side-effect classes.

Digest recomputation uses this repo's JCS canonical form (`canonical.py`); see
the harness README for the cross-language convergence note.
"""

from __future__ import annotations

from typing import Any

from canonical import GENESIS_HASH, hash_preimage, sha256_hex

NON_EXTERNAL: set[str] = set()  # not needed here; kept for symmetry


def recompute_receipt_hash(receipt: dict[str, Any]) -> str:
    return sha256_hex(hash_preimage(receipt, "receipt_hash"))


def recompute_journal_hash(record: dict[str, Any]) -> str:
    return sha256_hex(hash_preimage(record, "hash"))


def verify_receipt_chain(receipts: list[dict]) -> list[str]:
    errors: list[str] = []
    prev = GENESIS_HASH
    for idx, r in enumerate(receipts):
        if r.get("seq") != idx:
            errors.append(f"receipt[{idx}] seq {r.get('seq')} != expected {idx}")
        if r.get("prev_receipt_hash") != prev:
            errors.append(
                f"receipt[{idx}] prev_receipt_hash {r.get('prev_receipt_hash')} "
                f"!= previous hash {prev}"
            )
        expected = recompute_receipt_hash(r)
        if r.get("receipt_hash") != expected:
            errors.append(
                f"receipt[{idx}] receipt_hash {r.get('receipt_hash')} != recomputed {expected}"
            )
        prev = r.get("receipt_hash", "")
    return errors


def verify_journal_chain(records: list[dict]) -> list[str]:
    errors: list[str] = []
    prev = GENESIS_HASH
    sessions: dict[str, str] = {}
    transition_ids: set[str] = set()
    event_ids: set[str] = set()
    proposed: dict[str, dict] = {}
    allowed: dict[str, str] = {}
    latest_decision: dict[str, str] = {}

    for idx, rec in enumerate(records):
        if rec.get("seq") != idx:
            errors.append(f"journal[{idx}] seq {rec.get('seq')} != expected {idx}")
        if rec.get("prev_hash") != prev:
            errors.append(
                f"journal[{idx}] prev_hash {rec.get('prev_hash')} != previous hash {prev}"
            )
        expected = recompute_journal_hash(rec)
        if rec.get("hash") != expected:
            errors.append(f"journal[{idx}] hash {rec.get('hash')} != recomputed {expected}")
        prev = rec.get("hash", "")

        errors.extend(
            _causality(
                idx,
                rec,
                sessions,
                transition_ids,
                event_ids,
                proposed,
                allowed,
                latest_decision,
            )
        )
        event_id = _primary_event_id(rec)
        if event_id is not None:
            if event_id in event_ids:
                errors.append(f"journal[{idx}] event id {event_id} appears more than once")
            event_ids.add(event_id)

    return errors


def _causality(
    idx: int,
    record: dict,
    sessions: dict[str, str],
    transition_ids: set[str],
    event_ids: set[str],
    proposed: dict[str, dict],
    allowed: dict[str, str],
    latest_decision: dict[str, str],
) -> list[str]:
    event = record.get("event", {})
    kind = event.get("kind")
    errors: list[str] = []

    if kind == "session_created":
        session = event["session"]
        sid = session["session_id"]
        if sid in sessions:
            errors.append(f"journal[{idx}] session {sid} created more than once")
        sessions[sid] = session["status"]

    elif kind == "session_status_changed":
        transition_id = event.get("transition_id", "")
        sid = event["session_id"]
        current = sessions.get(sid)
        if not transition_id.strip():
            errors.append(f"journal[{idx}] session transition id is empty")
        elif transition_id in transition_ids:
            errors.append(f"journal[{idx}] session transition {transition_id} appears more than once")
        elif current is None:
            errors.append(f"journal[{idx}] transition {transition_id} references unknown session {sid}")
        elif current != event["from"]:
            errors.append(
                f"journal[{idx}] transition {transition_id} from {event['from']} "
                f"does not match current status {current}"
            )
        elif not _valid_session_transition(event["from"], event["to"]):
            errors.append(
                f"journal[{idx}] illegal session transition {transition_id}: "
                f"{event['from']} -> {event['to']}"
            )
        else:
            transition_ids.add(transition_id)
            sessions[sid] = event["to"]

    elif kind == "action_proposed":
        manifest = event["manifest"]
        aid = manifest["action_id"]
        if aid in proposed:
            errors.append(f"journal[{idx}] action {aid} proposed more than once")
        proposed[aid] = manifest

    elif kind == "policy_decided":
        decision = event["decision"]
        aid = decision["action_id"]
        if aid not in proposed:
            errors.append(
                f"journal[{idx}] decision {decision['decision_id']} references action "
                f"{aid} before it was proposed"
            )
        else:
            expected_hash = sha256_hex(proposed[aid])
            if decision.get("manifest_hash") != expected_hash:
                errors.append(
                    f"journal[{idx}] decision {decision['decision_id']} manifest_hash "
                    f"{decision.get('manifest_hash')} != action digest {expected_hash}"
                )
        latest_decision[aid] = decision["result"]
        if decision["result"] == "allowed":
            allowed[aid] = decision.get("manifest_hash", "")
        else:
            allowed.pop(aid, None)

    elif kind == "receipt_appended":
        receipt = event["receipt"]
        aid = receipt["action_id"]
        manifest = proposed.get(aid)
        if manifest is None:
            errors.append(
                f"journal[{idx}] receipt {receipt['receipt_id']} references action "
                f"{aid} before it was proposed"
            )
            return errors
        if aid not in allowed:
            latest = latest_decision.get(aid, "missing")
            errors.append(
                f"journal[{idx}] receipt {receipt['receipt_id']} references action {aid} "
                f"without a prior allowed decision (latest: {latest})"
            )
        elif allowed[aid] != sha256_hex(manifest):
            errors.append(
                f"journal[{idx}] receipt {receipt['receipt_id']} follows stale allowed "
                f"decision hash for action {aid}"
            )
        if receipt["tool_id"] != manifest["tool_id"]:
            errors.append(
                f"journal[{idx}] receipt {receipt['receipt_id']} tool {receipt['tool_id']} "
                f"!= action tool {manifest['tool_id']}"
            )
        if receipt["input_digest"] != manifest["inputs_digest"]:
            errors.append(
                f"journal[{idx}] receipt {receipt['receipt_id']} input digest != action input digest"
            )
        expected_target = manifest.get("resolved_target") or manifest["target"]
        if receipt["target"] != expected_target:
            errors.append(
                f"journal[{idx}] receipt {receipt['receipt_id']} target != action target"
            )
        declared = set(manifest.get("expected_side_effects", []))
        extra = set(receipt.get("side_effects", [])) - declared
        if extra:
            errors.append(
                f"journal[{idx}] receipt {receipt['receipt_id']} has undeclared side effects: "
                f"{sorted(extra)}"
            )

    elif kind == "memory_written":
        memory = event["memory"]
        source_event_id = memory.get("source_event_id", "")
        if not source_event_id.strip():
            errors.append(f"journal[{idx}] memory {memory['memory_id']} has an empty source_event_id")
        elif source_event_id not in event_ids:
            errors.append(
                f"journal[{idx}] memory {memory['memory_id']} references unknown source event "
                f"{source_event_id}"
            )

    return errors


def _valid_session_transition(from_status: str, to_status: str) -> bool:
    return (
        (from_status == "running" and to_status == "paused")
        or (from_status == "paused" and to_status == "running")
        or (from_status in {"running", "paused"} and to_status == "canceled")
    )


def _primary_event_id(record: dict) -> str | None:
    event = record.get("event", {})
    kind = event.get("kind")
    if kind == "session_created":
        return event["session"]["session_id"]
    if kind == "session_status_changed":
        return event["transition_id"]
    if kind == "capability_granted":
        return event["grant"]["grant_id"]
    if kind == "payment_mandate_issued":
        return event["mandate"]["mandate_id"]
    if kind == "action_proposed":
        return event["manifest"]["action_id"]
    if kind == "policy_decided":
        return event["decision"]["decision_id"]
    if kind == "approval_recorded":
        return event["approval"]["review_id"]
    if kind == "simulation_recorded":
        return event["simulation"]["simulation_id"]
    if kind == "receipt_appended":
        return event["receipt"]["receipt_id"]
    if kind == "memory_written":
        # memory_id is a mutable projection key, not an unambiguous event id:
        # later memory writes may intentionally replace the same memory_id.
        return None
    if kind == "scenario_evaluated":
        return event["scenario"]["scenario_id"]
    if kind == "incident_annotated":
        return event["incident_id"]
    return None
