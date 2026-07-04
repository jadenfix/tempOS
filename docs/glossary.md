# beaterOS glossary

Shared vocabulary so any reviewer — including one seeing a component for the
first time — can reason about the code. Terms are grounded in [`final.md`](../final.md);
section references point to the authoritative definition.

## Core contracts (final.md §12)

- **AgentSession** — the container for one goal-directed run: intent, scope,
  policy profile, initial capabilities, budgets, model policy, memory scope, and
  a journal root. A session cannot execute actions without at least one grant.
- **CapabilityGrant** — the central authority object: explicit, scoped
  permission bound to a holder, session, resource, and action set, with
  constraints (time, budget, data-sensitivity), delegation rule, approval rule,
  and a revocation handle. Cannot be broadened by the holder; delegated grants
  are equal-or-narrower.
- **ActionManifest** — a pre-declaration of a proposed side effect or
  observation (tool, target, input digest, expected side effects, risk class,
  data classes, idempotency key, compensation plan). Submitted *before*
  execution so policy can inspect it.
- **PolicyDecision** — the deterministic admission result for a manifest:
  `Allowed`, `Denied`, `NeedsApproval`, `NeedsSimulation`, or
  `NeedsNarrowedGrant`, plus matched rules and an explanation. Recorded before
  execution.
- **CapabilityReceipt** — the tamper-evident record of what actually happened:
  input/output digests, side-effect summary, external ids, and a hash link to
  the previous receipt. Append-only.
- **MemoryRecord** — knowledge with provenance: source event, writer, time,
  confidence, sensitivity, expiry, and access policy. Rebuildable and redactable.
- **PaymentMandate** — bounded economic authority: who may spend, asset, max
  amount, counterparty policy, purpose, approval threshold, idempotency, receipt
  requirement.
- **ScenarioManifest** — a testable task specification: goal, environment,
  fixtures, allowed tools, forbidden actions, oracle, success criteria, risk
  traps, budget, and expected trace properties.

## Authority and safety

- **Ambient authority** — power a principal holds implicitly without an explicit
  grant. beaterOS's central goal is to eliminate it (final.md §13.2).
- **Attenuation** — deriving a *narrower* capability from a broader one when
  delegating. Delegation may only attenuate, never amplify (§13.3).
- **Taint / provenance labels** — source labels on information (e.g.
  `trusted_user_instruction`, `untrusted_web`, `secret`, `customer_data`) that
  policy uses to decide what may flow where (§13.4).
- **Prompt injection** — untrusted content attempting to be treated as trusted
  instruction. Defended outside the model, not by prompting (§13.5).
- **Fail closed** — on missing/expired/revoked authority or ambiguity, deny by
  default rather than allow.
- **Risk class** — the severity tier of an action. May be *raised* by policy,
  never *lowered* by the agent (§26).
- **Receipt / journal** — journal records intent *before* side effects; receipts
  record outcomes *after*. Together they form the causal chain (§4.5, §10.4).

## Runtime and services (final.md §9, §10)

- **Agent kernel (`beater-osd`)** — the small trusted core: sessions,
  capability issuance, policy evaluation, action admission, journal writes,
  receipt verification, revocation, audit, eval gates.
- **Service fabric** — least-privileged system services (tool gateway, browser,
  sandbox, memory, model router, observability, eval, human review, payment,
  registry), each mediated by capabilities.
- **Sandbox lane** — an isolated execution context (pure-function, WASI,
  container, browser, VM, remote-tool). Every lane emits receipts.
- **Tool gateway** — normalizes MCP/A2A/OpenAPI/CLI/local tools into a policed
  registry; enforces grants, redaction, and egress limits at the boundary.
- **TCB (trusted computing base)** — the minimal set that must be trusted:
  capability service, policy engine, journal verifier, secret broker, sandbox
  launcher (§20.2). Everything else is less trusted.

## Process (this repo)

- **DPR (Deep PR Review)** — an independent, adversarial review by a non-author,
  recorded as a GitHub review verdict and in the agent-layer ledger. See the
  review gate in [`governance/review-checklist.md`](governance/review-checklist.md).
- **Slice** — one coherent, review-sized feature mapped to a `final.md` section.
  Slices and their dependencies are tracked in
  [`implementation-backlog.md`](implementation-backlog.md).
- **Coordination ledger** — the append-only agent-layer record of who authored,
  reviewed, and merged each PR (approvals can't use GitHub's Approve state because
  all agents share one account). See
  [`governance/coordination-ledger.md`](governance/coordination-ledger.md), linted
  by [`scripts/check-governance.py`](../scripts/check-governance.py).
