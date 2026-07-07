# beaterOS Repository Map and Runtime Frontier

This map is the working dependency map for runtime-first development.
Use it when planning PR scope, writing migration evidence, and validating code
review boundaries.

## 1) Runtime and authority foundation (must remain healthy first)

- `crates/beater-os-core`
  - Contracts (`AgentSession`, `CapabilityGrant`, `ActionManifest`,
    `PolicyDecision`, `CapabilityReceipt`, `MemoryRecord`, `PaymentMandate`).
  - Policy decision and journal semantics.
- `crates/beater-os-session`
  - Session model and lifecycle semantics used by all upper layers.
- `crates/beater-osd`
  - Runtime daemon store, admission boundary, projection, receipt append path.
  - Local loopback control-plane API for health and token-gated session
    projection.
  - Canonical proof of authority writes (`PolicyEngine` is only invocation point
    for admission decisions).
- `crates/beater-osd-http`
  - Loopback HTTP control-plane binary over `beater-osd` and the tool gateway,
    including token-gated local shell execution.
- `crates/beaterosctl`
  - Operator CLI for session/grant/manifests/receipts.

## 2) Service planes (runtime depends on these contracts)

- `crates/beater-os-sandbox`
  - Tool and action execution paths with path confinement, environment
    normalization, and side-effect evidence constraints.
- `crates/beater-os-memory`
  - Memory and provenance surface on top of session/journal evidence.
- `crates/beater-os-tool-registry`
  - Tool schema registry, tool risk metadata, and execution manifest binding.
- `crates/beater-os-tool-gateway`
  - Registered-tool resolution, kernel-derived manifest construction, daemon
    admission, sandbox execution, and receipt append for local shell tools.
- `crates/beater-os-runtime`
  - Typed agent runtime loop over the daemon store: session bootstrap, bounded
    grant issuance, sequential step admission, and no-side-effect observation
    receipts.
  - Deterministic step replay evidence anchored to proposal, decision, receipt,
    journal-root, and receipt-root hashes.
- `crates/beater-os-audit`
  - Trace/receipt validation, integrity reporting, replay tooling, and full
    trace/action bundle serialization.

## 3) Infrastructure and hardening gates

- `scripts/collect-bare-metal-host-profile.py`
  - Deterministic host snapshot capture.
- `scripts/check-bare-metal-readiness.py`
  - Manifest + host compatibility + migration-phase inference.
- `scripts/run-bare-metal-e2e-matrix.py`
  - Multi-host deterministic migration assertions.
- `scripts/run-beater-osd-runtime-smoke.py`
  - Runtime first smoke proof.
- `scripts/run-beater-os-runtime-smoke.py`
  - Hosted agent runtime bundle smoke proof over `beater-os-runtime`.
- `scripts/run-beater-osd-http-execute-smoke.py`
  - Token-gated daemon HTTP execution smoke over the local shell gateway.
- `scripts/local-e2e.py`
  - Aggregate gate when doing full lane validation locally.

## 4) Documents that shape architecture boundaries

- `final.md`
  - Product and architecture intent. **Do not shorten or weaken.**
- `docs/architecture-runtime-to-metal-path.md`
  - Execution contract for moving from runtime into optional metal lanes.
- `docs/engineering/bare-metal-readiness-manifest.json`
  - Source-of-truth lane graph, profiles, and workload classes.
- `docs/engineering/bare-metal-readiness.md`
  - Readiness semantics and migration terminology.
- `docs/engineering/bare-metal-e2e-matrix.json`
  - Deterministic phase and lane matrix fixtures.
- `docs/implementation-backlog.md`
  - Slice assignments, sequencing, and ownership.
- `docs/governance/review-checklist.md`
  - Mandatory reviewer gates.

## 5) Operating rulebook

1. Keep runtime contracts green before adding non-runtime work.
2. Don’t widen the migration frontier without:
   - manifest entry updates,
   - matrix/evidence updates,
   - and a non-author reviewer sign-off.
3. No claim can remove the hosted control plane contract without a staged
   replacement with equivalent evidence.
