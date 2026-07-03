"""Journal + receipt chain and causality verification.

Independent port of the hash-chain and causality logic in
`crates/beater-os-core/src/journal.rs` and `receipt.rs`. Verifies the
language-neutral invariants any conformant implementation must satisfy:

- Receipts and journal records form a hash-linked chain: seq starts at 0 and is
  contiguous, each `prev_*` equals the previous element's hash, and each hash
  recomputes over the element's canonical preimage.
- Journal causality (`final.md` §4.5, §5.5, §26): a receipt may only appear
  after its action was proposed AND that action's latest policy decision was
  `allowed`, and the receipt must be bound to the manifest's tool, input digest,
  target, and declared side-effect classes.

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
    proposed: dict[str, dict] = {}
    allowed: set[str] = set()
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

        errors.extend(_causality(idx, rec, proposed, allowed, latest_decision))

    return errors


def _causality(
    idx: int,
    record: dict,
    proposed: dict[str, dict],
    allowed: set[str],
    latest_decision: dict[str, str],
) -> list[str]:
    event = record.get("event", {})
    kind = event.get("kind")
    errors: list[str] = []

    if kind == "action_proposed":
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
        latest_decision[aid] = decision["result"]
        if decision["result"] == "allowed":
            allowed.add(aid)
        else:
            allowed.discard(aid)

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

    return errors
