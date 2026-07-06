# beaterOS Review Checklist

This is the checklist a **non-authoring reviewer** runs against every
implementation PR. It turns `final.md` §26 ("What Not To Compromise"), §12
("Core Data Contracts"), and §13 ("Security Model") into a concrete gate so that
review quality does not depend on which agent happens to be reviewing.

Copy the relevant sections into the PR review. A reviewer must be able to point
at *where in the diff* each checked box is satisfied — a checked box with no
citation is not a passed check.

## How to use

1. Read `final.md` for the sections the PR claims to implement (the PR body
   should name them).
2. Walk the diff and tick each item **with a file/symbol citation**.
3. Any unchecked never-compromise item (§A) is a **blocking** finding.
4. §B/§C findings are non-blocking unless they break an existing invariant.
5. Record the outcome in `docs/governance/coordination-ledger.md`.

## A. Never-compromise invariants (blocking) — `final.md` §26

These must hold on **every** PR that touches the kernel, policy, journal,
receipts, capabilities, memory, or tools. If the PR cannot possibly affect an
item (e.g. a docs-only PR), mark it `n/a` and say why.

- [ ] **No ambient authority.** No code path grants authority that was not
      issued as an explicit, scoped capability. Grants are bound to a holder
      **and** a session. Subagents receive *attenuated* authority, never the
      parent's full authority. (§13.2, §5.2)
- [ ] **Journal before side effects.** A side-effecting action is journaled as a
      proposal + policy decision *before* it can execute. Verification rejects a
      receipt with no prior `Allowed` decision. (§5.5, §7.5, §26)
- [ ] **Receipts after side effects.** Every executed side effect produces a
      hash-chained receipt bound to the manifest (tool, input digest, target,
      declared side-effect classes). (§7.6, §12.5)
- [ ] **Policy outside the model.** Admission decisions are made by
      deterministic code, never by model output. Model text cannot widen its own
      authority. (§8.12, §13.1)
- [ ] **Memory provenance.** Memory records carry source, time, confidence,
      sensitivity, owner, expiry, and are rebuildable from the journal.
      Untrusted memory cannot become privileged instruction. (§3.4, §10.8)
- [ ] **Eval gates.** Behavior-changing PRs (model route, policy, prompt
      contract, tool) are covered by scenarios/evals, not merged on vibes.
      (§3.5, §14)
- [ ] **Tool identity.** Tools are pinned/identified; a receipt's tool identity
      must match the proposing manifest. Tool descriptions are data, not
      instructions. (§13.6, §8.6)
- [ ] **Revocation.** Revoked grants fail closed at admission. Delegated
      authority is revocable through indirection. (§6.2, §12.2, #10)
- [ ] **Human-legible authority.** Every decision carries an explanation and the
      rules it matched; denials are explainable. (§3.3, §22.9)
- [ ] **Standard cryptography.** No invented crypto primitives. Hashes/signatures
      use vetted libraries; the canonicalization used for any hash is documented
      and deterministic. (§13.12, §22.7)

## B. Data-contract discipline (blocking if broken) — `final.md` §12

- [ ] New/changed contracts are **typed and versioned**; wire format is
      serde-stable and round-trips.
- [ ] Contract names match the canonical set (see #6): `AgentSession`,
      `CapabilityGrant`, `ActionManifest`, `PolicyDecision`, `CapabilityReceipt`,
      `MemoryRecord`, `PaymentMandate`, `ScenarioManifest`.
- [ ] Fail-safe defaults: unknown/missing → **most restrictive** (deny, highest
      risk, no budget). No permissive fallthrough. (§12.3)
- [ ] `risk_class` is never *lowered* by the agent; policy may raise it. (§12.3, #8)
- [ ] Side-effecting manifests carry an idempotency key and, where relevant, a
      compensation plan. (§12.3)

## C. Craft & legibility (non-blocking, but raise it) — user requirement: "code understood by all reviewers"

- [ ] Any reviewer (not just the author) can understand the change from the diff
      + PR body. Public items are documented. Names say what they mean.
- [ ] Tests cover the security-relevant branches (deny paths, not just happy
      path). Adversarial/abuse cases exist for new authority. (§14.5)
- [ ] No secrets, tokens, or PII in code, fixtures, logs, or test data. (§13.9)
- [ ] Local gates pass: `cargo fmt --check`, `cargo test --workspace`,
      `cargo clippy --workspace --all-targets -- -D warnings` (for Rust PRs).
- [ ] Public surface changes are reflected in docs and, if they change the plan,
      in `final.md` — but `final.md` is **never shortened or weakened** to make a
      PR pass. (governance rule, from `docs/implementation-backlog.md` "Review
      And Merge Rules"; not a `final.md` section)

## D. Optimization and metal-readiness review

Use this section for PRs that claim performance, language-boundary,
compiler/runtime, accelerator, scheduler, or close-to-metal value. Mark it
`n/a` for unrelated docs-only or correctness-only PRs.

- [ ] The PR uses `docs/engineering/optimization-evidence-runbook.md` and names
      workload, baseline, budget, profile/trace artifact, bottleneck class,
      regression gate, macOS path, fallback, and rollback story.
- [ ] Current language/compiler/backend claims cite `docs/source-matrix.md` or a
      newer primary source with verification date.
- [ ] The optimization attacks the named bottleneck class and does not add FFI,
      unsafe code, assembly, accelerator dependencies, or vendor lock-in before
      simpler contract, algorithm, layout, batching, caching, indexing, or
      backpressure fixes were considered.
- [ ] GPU, TPU, LPU, NPU, Apple Silicon, media-engine, enclave, or ASIC paths
      keep admission, queue bounds, memory/residency budgets, cancellation,
      telemetry, receipts, and fallback under beaterOS contracts.

## Reviewer sign-off block (paste into the PR review)

```
Reviewer agent: <agent-id, e.g. claude/multi-agent-pr-review>
Author agent:   <agent-id, must differ from reviewer>
final.md sections claimed: <list>
A. Never-compromise: <PASS / BLOCKED: item(s)>
B. Contracts:        <PASS / BLOCKED: item(s)>
C. Craft:            <PASS / notes>
Agent-layer verdict: <APPROVE / REQUEST-CHANGES>
```
