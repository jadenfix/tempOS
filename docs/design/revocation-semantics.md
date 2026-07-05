# Design Spec: Capability Revocation Semantics

Status: design spec. Closes the gap tracked in **issue #10**. Grounded in the
merged `crates/beater-os-core` code (`CapabilityGrant`, `AdmissionContext`,
policy admission). Does not edit `final.md`.

Revocation is never-compromise (`final.md` §26) and appears throughout (§6.2
"revoked through indirection", §12.2 "revoked grants fail closed", §13.15
incident mode "revoke grants"), but the *semantics* were unspecified: what
happens to an in-flight action, whether revoking a parent reaches its delegated
children, and how revocation coexists with the receipts a revoked grant already
produced. This spec answers all three and describes the code that enforces them.

## 1. Revocation is indirection, not a flag flip

A grant carries a `revocation_handle` and (new in this slice) an optional
`parent_grant_id`. Two independent revocation signals are honored at admission:

1. **Direct** — the grant's own `revoked` flag is `true`, or its `expires_at`
   has passed (`CapabilityGrant::is_active_at`).
2. **Registry / epoch** — the grant's `revocation_handle` is present in
   `AdmissionContext.revoked_handles`. This is the out-of-band case and the
   common one: you revoke a grant you already handed out without mutating the
   stored copy the agent holds.

`revoked_handles` is the **monotonic revocation epoch**: it only grows. Because
admission is a pure function of `(manifest, ctx)` and the epoch never shrinks, a
decision is deterministic and reproducible under replay — a revoked action stays
revoked when the trace is re-evaluated.

## 2. Delegation propagation (parent → child)

A delegated grant is authority *indirected through its parent*. This is now
literal: `grant_chain_effectively_active` walks `parent_grant_id` from the
exercised grant up to a root, and the grant is exercisable only if **every** link
is unexpired and unrevoked (by flag or registry).

Consequences:

- **Revoking a parent immediately kills all descendants.** No cascade job, no
  propagation latency to reason about: the child's liveness is *computed* from
  the parent's at each admission, so the parent's revocation is visible to the
  child the instant the child's next action is admitted.
- **A missing named ancestor fails closed.** If a grant names a parent that is
  not in the admission context, its liveness is unknown, so admission denies
  rather than assuming live.
- **A cycle fails closed.** The walk is bounded by a visited set; a malformed
  chain (a → b → a) can never admit an action.

The consistency guarantee is therefore "effective at the next admission point":
revocation does not reach *inside* an action already executing (see §3), but no
*new* action — including any delegated sub-action — is admitted once its chain
is revoked.

## 3. In-flight actions

Admission is the **pre-commit gate**, and it is re-evaluated for every action
against the current epoch. That yields a clean rule keyed to the side effect's
commit point:

- **Not yet committed.** The action has been admitted but its side effect has
  not landed (the `idempotency_key` has not been consumed downstream). Revoking
  the grant means the *next* admission checkpoint denies, and the pending side
  effect is aborted by its idempotency key — it is safe to drop because it never
  committed.
- **Already committed.** An irreversible side effect (a sent payment, a
  submitted form) cannot be un-executed by revocation. Containment moves to the
  action's `compensation_plan` (§12.3): revocation freezes the session and the
  compensation path runs to reverse or mitigate. Revocation stops *future*
  authority; compensation handles *past* effects.

This is the honest boundary: revocation is strong at admission checkpoints and
silent between them, and the design closes the gap with idempotency (uncommitted)
and compensation (committed) rather than pretending a policy decision can reach
into a syscall already in flight.

## 4. Revocation vs. receipts

A `CapabilityReceipt` is tamper-evident evidence of *what happened*, not a live
authorization. Revoking a grant does **not** invalidate the receipts it already
produced: the hash-linked chain must stay verifiable so the audit record of a
now-revoked authority survives. Liveness (can this grant authorize a *new*
action?) and audit validity (did this action happen under a valid grant at the
time?) are orthogonal, and only the former is affected by revocation.

## 5. What this slice implements vs. defers

**Implemented in code:** `parent_grant_id` on the grant, `revoked_handles` on the
admission context, transitive chain liveness (cycle- and missing-ancestor-safe),
and the admission denial with matched rule `grant_delegation_chain_active`. The
JSON schemas gain the optional `parent_grant_id`.

**Deferred (documented here, not coded):** the runtime abort path for uncommitted
side effects and the compensation trigger for committed ones live in the
execution/scheduler lane, not in the pure admission core. This spec fixes their
contract so that lane has an unambiguous target.

Related: #8 (risk floor — same admission-time grounding pattern), #46
(tool-registry grounding), #47 (concurrency — leases are time-bounded grants and
lease-break is a revocation), #15 (incident mode revokes grants).
