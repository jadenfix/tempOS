# beaterOS Runtime And Evidence Schemas

JSON Schemas (draft 2020-12) for runtime, evidence, optimization, and eval
artifacts that sit around the tempOS core contracts.

This directory is **not** the canonical source of truth for the core wire
contracts. The canonical, language-neutral core contract source is
[`spec/contracts`](../spec/contracts), documented in
[`spec/README.md`](../spec/README.md), and validated by
[`spec/conformance/validate.py`](../spec/conformance/validate.py). When a schema
name exists in both directories, `spec/contracts` owns the portable core
contract shape.

`contracts/schema` may carry broader runtime/evidence schemas and compatibility
mirrors needed by local tooling. Those mirrors must either be regenerated from
the core source or explicitly documented as runtime-specific extensions before
they diverge.

## Schemas

| Schema | Contract | `final.md` |
| --- | --- | --- |
| `agent-session.schema.json` | AgentSession runtime mirror; core shape owned by `spec/contracts` | §7.2, §12.1 |
| `capability-grant.schema.json` | CapabilityGrant runtime mirror; core shape owned by `spec/contracts` | §7.3, §12.2 |
| `action-manifest.schema.json` | ActionManifest runtime mirror; core shape owned by `spec/contracts` | §7.4, §12.3 |
| `policy-decision.schema.json` | PolicyDecision runtime mirror; core shape owned by `spec/contracts` | §7.5, §12.4 |
| `capability-receipt.schema.json` | CapabilityReceipt runtime mirror; core shape owned by `spec/contracts` | §7.6, §12.5 |
| `memory-record.schema.json` | MemoryRecord runtime mirror; core shape owned by `spec/contracts` | §7.7, §12.6 |
| `payment-mandate.schema.json` | PaymentMandate runtime mirror; core shape owned by `spec/contracts` | §12.7, §16.1 |
| `scenario-manifest.schema.json` | ScenarioManifest runtime mirror; core shape owned by `spec/contracts` | §7.10, §12.8 |
| `journal.schema.json` | JournalRecord + JournalEvent | §4.5, §10.4 |
| `common.schema.json` | Shared enums + sub-structures | — |
| `trace-bundle.schema.json` | A full end-to-end run (harness input) | §24 |
| `security-scenario.schema.json` | Adversarial eval + admission probe | §14.5 |
| `performance-trace.schema.json` | Optimization trace evidence envelope | §8, §13 |
| `accelerator-telemetry.schema.json` | Vendor-neutral accelerator job telemetry | §8, §13 |
| `worker-preflight-plan.schema.json` | Side-effect-free worker scheduler plan | §4.4, §6.4, §7 |

## Versioning & provenance

- Runtime mirrors should track the `crates/beater-os-core` wire format: exact
  field names and snake_case enum values that match serde's
  `rename_all = "snake_case"`. Core contract changes start in `spec/contracts`.
- `additionalProperties: false` throughout, so the corpus is validated strictly.
  When the Rust core adds a field, add it here in the same or a follow-up change
  and note it in `AGENTS.md` (PR #19/#20 own that coordination doc).
- Enum orderings that carry meaning (`risk_class`, `data_class`) are listed in
  severity order; the conformance harness relies on that order for ceiling
  comparisons.

## Validation

The schemas are exercised by the conformance gate:

```
python3 tools/conformance/validate.py
```

See `tools/conformance/README.md` for the semantic invariants (admission,
causality, hash chains) layered on top of structural validation, and for the
open cross-language canonical-hashing item.

Run the canonicality wording guard after editing this README:

```
python3 scripts/check-contract-canonicality.py
```
