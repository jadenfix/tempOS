# Design Spec: Risk Class Taxonomy and Assignment

Status: documents the **shipped** risk derivation and proposes narrow refinements.
Audit/plan-hardening lane (PR #21). Grounds `final.md` §12.3 `risk_class` in the
`RiskClass` enum and the `derived_risk_floor` / `effective_risk` logic already
implemented in `crates/beater-os-core/src/policy.rs`.

> Reconciliation note (2026-07-05): an earlier draft of this doc claimed the
> kernel "trusts the agent-supplied `risk_class`" and proposed a `classify_risk`
> follow-up. That is now **stale** — the kernel already derives a floor and takes
> the max with the agent value. This revision documents the shipped behavior and
> keeps only the genuinely-unshipped ideas as clearly-labeled proposals.

## 1. What ships today (canonical)

`RiskClass` is a 4-tier ordered enum (`contracts.rs`): `Low < Medium < High <
Critical`. Admission (`policy.rs`) computes:

```
derived_floor = derived_risk_floor(action_kind, expected_side_effects, data_classes)
effective_risk = manifest.risk_class.max(derived_floor)   // agent value may only RAISE
```

`derived_risk_floor` is a **pure function of kernel-derived fields only** — it
must never read the agent-asserted `risk_class` — so the floor is trustworthy
even if the model is manipulated (§13.1). The shipped mapping:

| Input | Contribution to floor |
| --- | --- |
| `ActionKind::Spend` / `Deploy` / `Delegate` | `High` |
| any other `ActionKind` | `Low` |
| `SideEffectClass::Payment` / `CloudMutation` / `Deployment` / `Delegation` | `High` |
| `SideEffectClass::NetworkWrite` / `BrowserSubmit` / `HumanCommunication` | `Medium` |
| `SideEffectClass::None` / `LocalWrite` / `MemoryWrite` | `Low` |
| `DataClass::Secret` / `Financial` | `High` |
| `DataClass::Customer` / `Personal` | `Medium` |
| other `DataClass` | `Low` |

The floor is the **max** across the action kind and every present side effect and
data class. Note the floor **never assigns `Critical`** — `Critical` enters only
when the agent declares it (and can then only raise, never lower, the effective
risk).

### 1.1 What each tier does (shipped gates in `policy.rs`)

- `effective_risk >= grant.approval.threshold_risk` → **needs human approval**.
- Untrusted taint (`UntrustedWeb/Email/Document`) on a `Spend`/`Deploy`/`Delegate`
  action → **needs action-bound approval** (cannot be auto-allowed).
- `effective_risk >= High` **and** the action has an external side effect
  (`manifest.has_external_side_effect()`) → **needs a passed simulation** before
  execution. A high-risk *local* action (e.g. `Secret`/`Financial` data with no
  external side effect) is **not** simulation-gated by the shipped code — it
  proceeds once grants/approvals pass.
- **Payment safety is separate from the tier system:** a payment
  (`is_payment_action`) is admitted only against an active `PaymentMandate` bound
  to the session+holder and covering the amount (`final.md` §12.7). So "spend is
  dangerous" is enforced by the mandate, not by forcing a `Critical` tier.

## 2. Invariants (already enforced; keep as regression checks)

1. **Raise-only:** `effective_risk = max(declared, derived_floor)` — an executed
   action's effective risk is never below its derived floor.
2. **Model-independence:** `derived_risk_floor` reads only typed
   `action_kind`/`side_effects`/`data_classes`; identical inputs → identical floor
   regardless of prompt or model.
3. **Consequence coupling:** `High + external side effect ⇒ simulation-gated`,
   `≥ threshold_risk ⇒ approval-gated` — unchanged; the floor decides the tier,
   not the gate.

## 3. Proposed refinements (NOT yet shipped — design input only)

These diverge from shipped behavior and are offered as options, not as the
current contract:

- **Explicit `Critical` floor for irreversible/production actions.** Today
  `Critical` is reachable only via agent declaration. A kernel floor of `Critical`
  for `Deployment`/production `CloudMutation` (distinguished from staging by
  target tags) would make the most dangerous class independent of the agent.
  Trade-off: needs a target-sensitivity signal the manifest doesn't yet carry.
- **Secret-egress bump.** Raise the floor when a `Secret`/`Financial` data class
  *leaves a trust boundary* (model route / external recipient), above the flat
  `High` it contributes today. Requires an egress signal on the action.
- **Reversibility bump.** Raise one tier when a non-`None` side effect declares no
  `compensation_plan`. Cheap; purely a function of existing fields.
- **Fail-safe default.** Unknown/未mapped inputs → `Critical`. The shipped floor
  defaults benign inputs to `Low`; a stricter "unknown ⇒ Critical" would match
  §12.3 "unknown side effects require denial or review" but needs an explicit
  "unknown" signal rather than the enum's exhaustive match.

Each refinement is a small, testable change to `derived_risk_floor` for the
kernel lane (codex/#1) to consider; none is required for this doc to be accurate.

## 4. Acceptance mapping (issue #8)

- [x] Enumerated risk classes with definitions — §1 (grounded in shipped `RiskClass`).
- [x] Deterministic assignment rule (inputs → class) — §1 table (the shipped
      `derived_risk_floor`).
- [x] Fail-safe/raise-only invariants documented — §2; stricter fail-safe is a §3
      proposal.
