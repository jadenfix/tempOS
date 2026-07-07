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
2. **A payment intent is declared.** `payment_intent` normalizes the concrete
   rail payload into chain-neutral fields: mandate id, rail, adapter id, asset,
   integer amount, counterparty reference and digest, purpose, idempotency key,
   envelope format, envelope hash, and optional envelope expiry.
3. **The intent is internally bound.** The manifest target must be a
   `payment_rail`; the target rail, requested budget amount, and manifest
   idempotency key must match the payment intent; hashes must be lowercase
   32-byte hex.
4. **A covering mandate exists**, where the mandate is
   - bound to this session (`session_id`) and holder (`actor_id`),
   - still active (`expires_at > now`), and
   - selected by `payment_intent.mandate_id`.
5. **The mandate covers the intent**, where rail, asset, purpose, idempotency
   key, per-action amount ceiling, allowed adapter ids, and allowed envelope
   formats all match.
6. **The mandate has remaining capacity.** Admission receives a replay-derived
   `payment_reserved_by_mandate` meter. Until cancellation/release semantics
   exist, every prior non-denied payment decision (`Allowed`, `NeedsApproval`, or
   `NeedsSimulation`) reserves capacity against the mandate fail-closed, so
   parallel or staged payments cannot silently oversubscribe the ceiling before a
   receipt is emitted.

On success the rule `payment_authorized_by_mandate` is recorded and admission
proceeds to the existing grant, approval, and simulation gates — so a payment
now must satisfy **both** a grant and a mandate, and a high-risk one still routes
to simulation. `AdmissionContext` carries `mandates: Vec<PaymentMandate>` plus
the replay-derived `payment_reserved_by_mandate` meter.

## Placement

The mandate gate runs immediately after the payment/spend consistency check and
**before** grant matching. "No payment without a mandate" is thus a top-level
payment invariant: a mandate-less payment is denied outright, regardless of which
grants are held or whether the content is trusted.

## Adapter model

beaterOS remains payment-rail neutral. Stripe, cards, bank APIs, x402, and Aether
all enter policy as the same `PaymentIntent`. The adapter is responsible for
verifying concrete rail artifacts, such as a Stripe PaymentIntent or Aether
`aether-agent-payment-v1` envelope. Policy admits only the normalized projection:
rail, adapter, envelope format, envelope hash, amount, counterparty binding, and
mandate id.

For Aether, a mandate can set:

- `rail = "aether:aic"` or another logical Aether rail id,
- `allowed_adapter_ids = ["aether"]`,
- `allowed_envelope_formats = ["aether-agent-payment-v1"]`.

That makes Aether native to beaterOS policy without moving chain id, signature
algorithm, slot expiry, nonce, or settlement proof parsing into the OS authority
contract. Those fields stay in the Aether envelope and receipt artifacts.

## Follow-up surface

- **Mandate-driven approval threshold.** Implemented in core policy and daemon
  replay: above-threshold payment actions require action-bound human approval
  before they can proceed to simulation/admission.
- **Typed payment receipts.** Implemented in the receipt/journal lane and
  exposed through `beaterosctl receipt record`: required payment receipts carry
  mandate id, rail, adapter id, envelope hash, rail-receipt hash, and settlement
  status. Generic external ids are supplemental only.
- **Payment release/cancellation semantics.** The current meter reserves capacity
  for every non-denied payment decision. A later slice should add explicit
  release/cancel evidence so abandoned payment attempts can free reserved
  capacity without weakening replay.

## Verification

Admission tests cover: no mandate present (denied), missing payment intent
(denied), amount over per-action and cumulative ceilings (denied), undeclared
amount (denied), mandate bound to another session or holder (denied), expired
mandate/envelope (denied), rail/asset/purpose/idempotency/counterparty mismatch
(denied), invalid envelope hash (denied), Aether adapter or envelope format
mismatch (denied), daemon replay reservation, and a covered payment that passes
the gate and then routes to simulation. The independent Python conformance port
and adversarial payment scenarios exercise the same gates.

Related: #8 (risk floor — payments are Critical), #67/#40 (budget ceilings are a
*different* axis from mandate authority), #10 (a revoked grant already fails
closed; a mandate is the economic analogue).
