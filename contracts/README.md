# beaterOS Contract Schemas

Language-neutral JSON Schemas (draft 2020-12) for the beaterOS core contracts.
They are the canonical, cross-language definition that every implementation --
the Rust `beater-os-core` crate today, a TypeScript CLI or dashboard tomorrow --
should serialize to and validate against.

## Contracts

| Schema | Contract | `final.md` |
| --- | --- | --- |
| `agent-session.schema.json` | AgentSession | §7.2, §12.1 |
| `capability-grant.schema.json` | CapabilityGrant | §7.3, §12.2 |
| `action-manifest.schema.json` | ActionManifest | §7.4, §12.3 |
| `policy-decision.schema.json` | PolicyDecision | §7.5, §12.4 |
| `capability-receipt.schema.json` | CapabilityReceipt | §7.6, §12.5 |
| `memory-record.schema.json` | MemoryRecord | §7.7, §12.6 |
| `payment-mandate.schema.json` | PaymentMandate | §12.7, §16.1 |
| `scenario-manifest.schema.json` | ScenarioManifest | §7.10, §12.8 |
| `journal.schema.json` | JournalRecord + JournalEvent | §4.5, §10.4 |
| `common.schema.json` | Shared enums + sub-structures | — |
| `trace-bundle.schema.json` | A full end-to-end run (harness input) | §24 |
| `security-scenario.schema.json` | Adversarial eval + admission probe | §14.5 |

## Versioning & provenance

- These schemas mirror the `crates/beater-os-core` wire format: exact field
  names, and snake_case enum values that match serde's
  `rename_all = "snake_case"`.
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
