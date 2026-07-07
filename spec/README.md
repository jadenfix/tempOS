# beaterOS Contract Spec (`spec/`)

Spec version: **0.1.0** (see [`VERSION`](VERSION)).

This directory is the **language-neutral source of truth** for the beaterOS core
data contracts described in [`final.md`](../final.md) sections **7 ("What Agents
Should Have In An OS")** and **12 ("Core Data Contracts")**.

`final.md` describes these contracts in prose. The Rust crate
`crates/beater-os-core` implements them for one runtime. This directory closes
the gap between the two: it defines the **on-the-wire JSON shape** every beaterOS
implementation must agree on, as machine-checkable [JSON Schema](https://json-schema.org)
(draft 2020-12) plus a runnable conformance suite. Any implementation in any
language — the Rust kernel today, a TypeScript UI or Python eval harness later —
is conformant iff its serialized contract objects validate against these schemas.

Because it is language-neutral and dependency-free, every reviewer and every
agent working on the repo can read, run, and extend it — not just the author of
any one runtime.

## Layout

```
spec/
  VERSION                          # spec semantic version
  contracts/
    common.schema.json             # shared enums + value objects (Budget, CapabilitySelector, ...)
    agent-session.schema.json      # final.md 12.1
    capability-grant.schema.json   # final.md 12.2
    action-manifest.schema.json    # final.md 12.3
    policy-decision.schema.json    # final.md 12.4
    capability-receipt.schema.json # final.md 12.5
    memory-record.schema.json      # final.md 12.6
    payment-mandate.schema.json    # final.md 12.7
    scenario-manifest.schema.json  # final.md 12.8
  examples/
    valid/<contract>/*.json        # instances that MUST validate
    invalid/<contract>/*.json      # instances that MUST be rejected (one defect each)
  conformance/
    validate.py                    # dependency-free validator + runner
    README.md                      # how the runner works
  COORDINATION.md                  # cross-agent work registry + protocol
```

## Running the conformance suite

Requires only a stock Python 3 (3.8+). No `pip install`, no network.

```bash
python3 spec/conformance/validate.py          # verbose
python3 spec/conformance/validate.py --quiet   # failures + summary only
```

Exit code is `0` iff every valid example validates, every invalid example is
rejected, and every contract has at least one valid and one invalid example.
CI runs this on every push and PR (`.github/workflows/contracts-conformance.yml`).

## Contract index

| Contract | `final.md` | Purpose | Key invariants (enforced by runtimes; documented here) |
| --- | --- | --- | --- |
| AgentSession | 12.1 | One goal-directed run | No action without a grant; status transitions journaled; pause/resume keeps causality; resume fails closed while unresolved execution leases exist unless an explicit `outcome_unknown` reconciliation closes the blocker |
| CapabilityGrant | 12.2 | Explicit authority | Bound to holder + session; cannot be broadened; expired/revoked fail closed; never inferred from prompt text |
| ActionManifest | 12.3 | Predeclared side effect | Risk raised by policy, never lowered by agent; unknown side effects need denial/review; payment actions bind a normalized `payment_intent`; external effects need receipts |
| PolicyDecision | 12.4 | Deterministic admission | Denied cannot execute; review-required cannot execute pre-approval; journaled before execution; new admission fails closed while unresolved execution leases exist unless explicitly reconciled |
| CapabilityReceipt | 12.5 | What happened | Append-only; hash-chained (`prev_receipt_hash` → `receipt_hash`); redaction via refs preserves the chain |
| MemoryRecord | 12.6 | Knowledge + provenance | Has a source; has an access policy; invalidatable; rebuildable from the journal |
| PaymentMandate | 12.7 | Bounded economic authority | No payment without a mandate; no silent expansion; rail adapter/envelope allowlists constrain concrete payment envelopes; every attempt produces a receipt |
| ScenarioManifest | 12.8 | Testable task | Versioned; comparable across model/policy versions; incidents become scenarios |

## Wire-format notes (schema vs. `final.md` prose)

The schemas track the **serialized** representation (what `serde` emits in
`crates/beater-os-core`), which differs from the prose field names in a few
places. These are intentional and documented in each schema's `description`:

- ActionManifest: prose `action_type` → wire `action_kind`; adds
  `resolved_target`, `requested_budget`, `taint`, `human_explanation`.
- PolicyDecision: the wire format adds a required `manifest_hash` that binds the
  decision to the exact manifest it decided on (not named in prose §12.4);
  conversely prose §12.4 lists `matched_rules` / `required_review` /
  `required_simulation` as required, but the wire format makes them optional
  (`serde(default)`).
- CapabilityGrant: prose flat `resource` + `actions` → wire `scope`
  (`{ selector, actions }`) + `denied_actions` + `constraints`.
- CapabilityReceipt: prose `previous_receipt_hash` → wire `prev_receipt_hash`;
  adds `seq`.
- MemoryRecord: `confidence` is integer **basis points**, not a float. The
  schema enforces the semantic ceiling `0–10000`; the Rust type is a wider `u16`,
  so the schema is intentionally stricter than the storage type.
- ScenarioManifest: prose `seed_data` is folded into `fixtures`.
- Money is always integer **minor units** — never a float (see the
  `payment-mandate/float-amount` invalid example).

Optional fields use `serde(default)`; on the wire an absent optional may appear
as an explicit `null` (e.g. `memory_scope`, `Budget` dimensions). The schemas
allow both omission and `null` for those fields.

## Compatibility & versioning

- Schemas use `additionalProperties: false`, so an unknown field is a hard
  failure. Adding a field is a spec change and **must** bump [`VERSION`](VERSION)
  and land through a reviewed PR.
- Enum vocabularies here are the canonical set. Adding an enum value is a spec
  change; runtimes must not emit values not listed here.
- When a runtime and this spec disagree, that is a bug in one of them and should
  be raised as a PR comment or issue, not silently worked around. This directory
  is the tie-breaker for wire shape.
