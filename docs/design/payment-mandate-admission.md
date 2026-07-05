# Design Spec: Payment Mandate Admission

Status: design spec. Closes the core gap tracked in **issue #73**. Grounded in
the merged `crates/beater-os-core` code (`PaymentMandate`, `AdmissionContext`,
policy admission). Does not edit `final.md`.

`final.md` §12.7 states two payment invariants — *"no payment without a mandate"*
and *"no silent mandate expansion"* — and the `PaymentMandate` contract has
existed since the schema package landed. But `PolicyEngine::admit` never
consulted it: a `Spend`/`Payment` action was admitted on capability grants
alone. Grants authorize the *act* of spending; nothing checked the object that
authorizes the *money*. This slice closes that hole.

## The distinction the kernel was missing

- A **CapabilityGrant** answers "may this agent perform a spend action on this
  rail?" — an authority over *verbs and targets*.
- A **PaymentMandate** answers "is this specific movement of money — this amount,
  to this counterparty, for this purpose, under this ceiling — authorized?" — an
  authority over *economics*.

They are orthogonal. Holding a spend grant with no mandate is exactly the state
§12.7 forbids, and it was silently admitted.

## What admission now enforces

A payment action is any manifest that declares a `Payment` side effect **or**
uses the `Spend` verb (both, so a spend cannot dodge review by omitting the
side-effect label — the same anti-laundering stance as #46/#8). For such an
action, admission fails closed unless a mandate covers it:

1. **Amount is declared.** `requested_budget.max_payment_minor_units` must be
   `Some`. A payment that does not state how much it moves cannot be bounded, so
   it is denied ("no silent mandate expansion" begins with knowing the amount).
2. **A covering mandate exists**, where the mandate is
   - bound to this session (`session_id`) and holder (`actor_id`),
   - still active (`expires_at > now`), and
   - sufficient for the amount (`amount <= max_minor_units`).

On success the rule `payment_authorized_by_mandate` is recorded and admission
proceeds to the existing grant, approval, and simulation gates — so a payment
now must satisfy **both** a grant and a mandate, and a high-risk one still routes
to simulation. `AdmissionContext` gains `mandates: Vec<PaymentMandate>`.

## Placement

The mandate gate runs immediately after the payment/spend consistency check and
**before** grant matching. "No payment without a mandate" is thus a top-level
payment invariant: a mandate-less payment is denied outright, regardless of which
grants are held or whether the content is trusted.

## Deliberately deferred (documented, not coded)

The current contracts do not yet express every axis §12.7 and §6.8 describe, so
this slice binds what is unambiguous and leaves the rest as a tracked follow-up
rather than inventing manifest fields:

- **Counterparty / asset / purpose binding.** The mandate carries
  `counterparty_policy`, `asset`, and `purpose`, but the manifest has no
  counterparty or purpose field to match against. Binding these needs a manifest
  extension (a payment sub-record) and belongs to the full payment lane (backlog
  slice 15).
- **Mandate-driven approval threshold.** `approval_threshold_minor_units` should
  force human approval above a per-mandate limit. The approval machinery exists
  on grants; wiring the mandate threshold into it is the next increment.
- **Payment receipt requirement.** `receipt_requirement` should gate completion,
  which lives in the execution/receipt lane, not admission.
- **Idempotency at the money layer.** The manifest already requires an
  `idempotency_key` for external effects; binding it to the mandate's
  `idempotency_key` so a retried-then-cancelled step cannot double-commit is part
  of the full lane.

## Verification

Admission tests cover: no mandate present (denied), amount over ceiling (denied),
undeclared amount (denied), mandate bound to another session (denied), and a
covered payment that passes the gate and then routes to simulation. The two
pre-existing untrusted-payment tests were updated to supply a covering mandate,
and still assert their `NeedsApproval` outcome — proving the mandate gate sits
cleanly ahead of the taint/approval gates.

Related: #8 (risk floor — payments are Critical), #67/#40 (budget ceilings are a
*different* axis from mandate authority), #10 (a revoked grant already fails
closed; a mandate is the economic analogue).
