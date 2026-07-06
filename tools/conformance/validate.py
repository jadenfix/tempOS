#!/usr/bin/env python3
"""beaterOS conformance gate.

Validates the whole language-neutral corpus:

  * every trace bundle in `examples/traces/` against the JSON Schemas, then its
    receipt/journal hash chains, journal causality, and -- for each policy
    decision -- that an independent admission port reaches the same result;
  * every scenario in `scenarios/` against the schema, then that the adversarial
    probe is blocked/escalated exactly as declared.

Exit code is non-zero if anything fails, so it works as a CI release gate
(`final.md` §5.9, §14.6). Zero third-party dependencies.
"""

from __future__ import annotations

import json
import sys
from pathlib import Path

import admission
import journalcheck
from canonical import sha256_hex
from schema import SchemaRegistry, validate

ROOT = Path(__file__).resolve().parents[2]
SCHEMA_DIR = ROOT / "contracts" / "schema"
TRACES_DIR = ROOT / "examples" / "traces"
SCENARIOS_DIR = ROOT / "scenarios"


class Report:
    def __init__(self) -> None:
        self.checks = 0
        self.failures: list[str] = []

    def check(self, ok: bool, label: str, detail: str = "") -> None:
        self.checks += 1
        if not ok:
            self.failures.append(f"{label}: {detail}" if detail else label)

    def add_errors(self, label: str, errors: list[str]) -> None:
        self.checks += 1
        for err in errors:
            self.failures.append(f"{label}: {err}")


def _load_registry() -> SchemaRegistry:
    return SchemaRegistry().load_dir(SCHEMA_DIR)


def _validate_schema(rep: Report, reg, instance, schema_file, label) -> bool:
    errors = validate(instance, schema_file, reg)
    rep.add_errors(f"{label} [{schema_file}]", errors)
    return not errors


RESERVING_PAYMENT_RESULTS = {"allowed", "needs_approval", "needs_simulation"}


def _is_payment_manifest(manifest: dict) -> bool:
    return (
        manifest.get("action_kind") == "spend"
        or "payment" in manifest.get("expected_side_effects", [])
    )


def _payment_reserved_by_mandate(
    prior_decisions: list[dict],
    manifests: dict[str, dict],
    excluded_action_id: str,
) -> dict[str, int]:
    latest_decisions: dict[str, dict] = {}
    for decision in prior_decisions:
        action_id = decision.get("action_id")
        if action_id == excluded_action_id:
            continue
        latest_decisions[action_id] = decision

    reserved: dict[str, int] = {}
    for action_id, decision in latest_decisions.items():
        if decision.get("result") not in RESERVING_PAYMENT_RESULTS:
            continue
        manifest = manifests.get(action_id)
        if manifest is None:
            raise ValueError(
                f"reserving policy decision for action {action_id} has no manifest"
            )
        if not _is_payment_manifest(manifest):
            continue
        intent = manifest.get("payment_intent")
        if not isinstance(intent, dict):
            raise ValueError(
                f"reserving payment decision for action {action_id} has no payment_intent"
            )
        mandate_id = intent.get("mandate_id")
        amount = intent.get("amount_minor_units")
        if not mandate_id or type(amount) is not int or amount < 0:
            raise ValueError(
                f"reserving payment decision for action {action_id} has an invalid payment_intent"
            )
        next_total = reserved.get(mandate_id, 0) + amount
        if next_total > 2**64 - 1:
            raise ValueError(
                f"payment cumulative reservation overflowed mandate meter for {mandate_id}"
            )
        reserved[mandate_id] = next_total
    return reserved


# --- trace bundles --------------------------------------------------------


def check_trace_bundle(rep: Report, reg, path: Path) -> None:
    bundle = json.loads(path.read_text())
    name = path.name
    _validate_schema(rep, reg, bundle, "trace-bundle.schema.json", f"trace {name}")

    sessions = {s["session_id"]: s for s in bundle.get("sessions", [])}
    manifests = {m["action_id"]: m for m in bundle.get("manifests", [])}

    # Hash chains + causality.
    rep.add_errors(f"trace {name} receipt-chain", journalcheck.verify_receipt_chain(bundle.get("receipts", [])))
    rep.add_errors(f"trace {name} journal-chain", journalcheck.verify_journal_chain(bundle.get("journal", [])))

    # Independent admission: each decision must match a re-derived admit().
    decisions = bundle.get("decisions", [])
    for index, decision in enumerate(decisions):
        aid = decision["action_id"]
        manifest = manifests.get(aid)
        if manifest is None:
            rep.check(False, f"trace {name} decision {decision['decision_id']}",
                      f"references unknown action {aid}")
            continue
        session = sessions.get(manifest["session_id"])
        if session is None:
            rep.check(False, f"trace {name} manifest {aid}",
                      f"references unknown session {manifest['session_id']}")
            continue
        expected_hash = sha256_hex(manifest)
        rep.check(
            decision.get("manifest_hash") == expected_hash,
            f"trace {name} decision {decision['decision_id']} manifest-hash",
            f"recorded={decision.get('manifest_hash')} recomputed={expected_hash}",
        )
        try:
            payment_reserved_by_mandate = _payment_reserved_by_mandate(
                decisions[:index],
                manifests,
                aid,
            )
        except ValueError as exc:
            rep.add_errors(f"trace {name} payment reservation replay", [str(exc)])
            continue
        ctx = {
            "now": decision["created_at"],
            "actor_id": session["agent_id"],
            "session_id": manifest["session_id"],
            "policy_version": bundle["policy_version"],
            "grants": bundle.get("grants", []),
            "approvals": bundle.get("approvals", []),
            "simulations": bundle.get("simulations", []),
            "mandates": bundle.get("payment_mandates", []),
            "payment_reserved_by_mandate": payment_reserved_by_mandate,
        }
        got = admission.admit(manifest, ctx)
        rep.check(
            got["result"] == decision["result"],
            f"trace {name} admission {aid}",
            f"recorded={decision['result']} recomputed={got['result']} ({got['explanation']})",
        )

    # Every receipt must correspond to a manifest that was actually allowed.
    allowed = {d["action_id"] for d in bundle.get("decisions", []) if d["result"] == "allowed"}
    for r in bundle.get("receipts", []):
        rep.check(r["action_id"] in allowed, f"trace {name} receipt {r['receipt_id']}",
                  f"has no allowed decision for action {r['action_id']}")


# --- security scenarios ---------------------------------------------------


def check_scenario(rep: Report, reg, path: Path) -> None:
    scenario = json.loads(path.read_text())
    name = path.relative_to(SCENARIOS_DIR)
    _validate_schema(rep, reg, scenario, "security-scenario.schema.json", f"scenario {name}")

    probe = scenario["probe"]
    ctx = dict(probe["context"])
    ctx.setdefault("grants", [])
    ctx.setdefault("approvals", [])
    ctx.setdefault("simulations", [])
    ctx.setdefault("mandates", [])
    ctx.setdefault("payment_reserved_by_mandate", {})
    got = admission.admit(probe["manifest"], ctx)

    rep.check(
        got["result"] == probe["expected_result"],
        f"scenario {name} admission",
        f"expected={probe['expected_result']} got={got['result']} ({got['explanation']})",
    )
    if probe.get("must_be_blocked"):
        rep.check(
            got["result"] != "allowed",
            f"scenario {name} block-invariant",
            f"attack was ADMITTED ({got['explanation']}) -- policy layer failed to block/escalate",
        )


# --- driver ---------------------------------------------------------------


def main() -> int:
    reg = _load_registry()
    rep = Report()

    traces = sorted(TRACES_DIR.glob("*.trace.json")) if TRACES_DIR.exists() else []
    scenarios = sorted(SCENARIOS_DIR.rglob("*.json")) if SCENARIOS_DIR.exists() else []

    if not traces:
        rep.check(False, "corpus", "no trace bundles found under examples/traces/")
    if not scenarios:
        rep.check(False, "corpus", "no scenarios found under scenarios/")

    for path in traces:
        check_trace_bundle(rep, reg, path)
    for path in scenarios:
        check_scenario(rep, reg, path)

    print(f"conformance: {len(traces)} trace bundle(s), {len(scenarios)} scenario(s), "
          f"{rep.checks} checks")
    if rep.failures:
        print(f"\nFAILED ({len(rep.failures)}):")
        for f in rep.failures:
            print(f"  - {f}")
        return 1
    print("PASS: all conformance checks green")
    return 0


if __name__ == "__main__":
    raise SystemExit(main())
