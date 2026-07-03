# beaterOS Contracts (schema-first interop layer)

This directory is the **language-neutral source of truth** for the six-plus core
data contracts described in [`final.md` section 12](../final.md). Every beaterOS
implementation — the reference Rust crate `beater-os-core`, any future Python or
TypeScript service, an MCP gateway, a scenario runner — should serialize objects
that validate against these schemas. Contracts are how independently-built,
independently-owned components stay interoperable.

## Why schemas, not just Rust structs

`final.md` is explicit that the OS "should own the contract around authority,
state, observability, and evals" and that models/tools/implementations are
replaceable. A JSON Schema is the smallest artifact that:

- every language can produce and consume,
- a reviewer can read without a toolchain,
- CI can enforce mechanically,
- and multiple parallel agents can converge on without owning each other's code.

The Rust crate is **one conformant implementation**; these schemas are the
contract it (and everything else) conforms to.

## Layout

```
contracts/
  schemas/    canonical JSON Schemas (Draft 2020-12), one per contract + common.schema.json
  examples/   one valid instance per contract, forming a coherent end-to-end trace
```

The examples are not arbitrary: `agent_session` → `capability_grant` →
`action_manifest` → `policy_decision` → `capability_receipt` → `memory_record`
read as a single run (fix a parser test under a scoped grant), so a reviewer can
follow authority flowing through the system.

## Contracts

| Schema | final.md | Purpose |
| --- | --- | --- |
| `agent_session` | 12.1 | one goal-directed run |
| `capability_grant` | 12.2 | explicit, non-ambient authority |
| `action_manifest` | 12.3 | predeclared side effect / observation |
| `policy_decision` | 12.4 | deterministic admission result |
| `capability_receipt` | 12.5 | tamper-evident record of what happened |
| `memory_record` | 12.6 | knowledge with provenance |
| `payment_mandate` | 12.7 | bounded economic authority |
| `scenario_manifest` | 12.8 | a testable task |

`common.schema.json` holds the controlled vocabularies (enums) and shared value
objects. Enum values are `snake_case` so any language can target them.

## Design decisions (read before changing a schema)

- **Canonical required fields are mandated exactly as `final.md` lists them.**
  Removing or renaming a required field is a weakening of the plan and will be
  caught in review.
- **`additionalProperties` is open at the top level of each contract.** This is
  deliberate: implementations must be free to carry extra fields (the Rust crate
  adds `denied_actions`, `revoked`, `resolved_target`, …) without breaking
  interop. The schema pins the *floor*, not the ceiling.
- **Invariants that need runtime state are not encoded here.** "Expired grants
  fail closed", "risk can be raised but not lowered", and hash-chain continuity
  are enforced by the policy engine and journal, not by shape validation. The
  schema guarantees the *fields required to enforce them* are present and typed.

## Interop with the reference Rust crate (`beater-os-core`)

The Rust implementation (PR #1, `codex/agent-kernel-contracts`) is conformant on
structure but uses a few names that differ from `final.md`. These are tracked
here so the fleet can converge rather than drift:

| Contract | `final.md` / schema | Rust crate | Resolution |
| --- | --- | --- | --- |
| ActionManifest | `action_type` | `action_kind` | add a serde `alias`/`rename`, or agree to amend `final.md` |
| ActionManifest | `idempotency_key` (required, non-null) | `Option<String>` (required only for external side effects) | schema follows `final.md` (always required); the crate should populate it on every manifest to conform, even a deterministic key for pure reads |
| CapabilityGrant | `resource` + `actions` | nested `scope { selector, actions }` | grant schema keeps `final.md`'s flat shape; a `scope` object is accepted as an additional property |
| CapabilityGrant | `approval_requirements` | `approval` | serde `rename` on the Rust side |
| PolicyDecision | `required_review` / `required_simulation` | `Option<String>` (review/sim handle) | `final.md` mandates the fields but not their type; the schema accepts boolean OR string OR null so both a flag and a handle round-trip. The fleet should pick one reading and pin it |

Most of these do not break serialization today (open `additionalProperties`, and
`PolicyDecision`/`required_*` now accept both readings). The two that the schema
holds to `final.md` deliberately — `action_type` naming and `idempotency_key`
always-present — are the crate's to reconcile. See the review note filed on PR #1.

## Validate locally (zero dependencies)

```sh
python3 tools/contracts_validate.py          # validate every example
python3 -m unittest discover -s tests        # full conformance + negative tests
```

The validator prefers the `jsonschema` package (authoritative Draft 2020-12) and
falls back to a small built-in validator when it is absent, so any reviewer can
run it. Both paths are tested to agree, including that `date-time` formats are
asserted (not merely annotated).
