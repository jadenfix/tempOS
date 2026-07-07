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
  - Durable session budget replay for tool-call and wall-clock runtime quotas.
  - Durable execution leases between `Allowed` policy decisions and spawned
    side-effecting tools; unresolved open leases are projected as recovery
    blockers and prevent blind replay, new admission, and session resume after
    crash windows until explicit operator reconciliation records
    `outcome_unknown` without fabricating a receipt.
  - Local loopback control-plane API for health and token-gated session
    projection.
  - Store-owned claimable execution-action projection so scheduler workers get
    manifest hash, decision id, target, budget, grants, and pinned tool
    version/digest from the daemon authority instead of re-deriving lossy state.
  - Canonical proof of authority writes (`PolicyEngine` is only invocation point
    for admission decisions).
- `crates/beater-osd-http`
  - Loopback HTTP control-plane binary over `beater-osd` and the tool gateway,
    including token-gated local shell execution, hosted runtime bundle
    submission, and scheduler execution-lease claim/completion routes.
  - Session projection responses expose execution-lease recovery blockers so
    operators can distinguish ordinary idle state from runnable pending work,
    paused admission, and unresolved side-effect recovery debt without exporting
    the full journal.
  - Scheduler claim routes derive execution leases from journaled manifest and
    policy decision state using expected manifest/decision/tool
    compare-and-set fields, resolve pinned tool identity through the
    daemon-owned registry, and return the derived target/grants/budget lease
    authority; completion requires the exact open lease id before appending a
    receipt.
  - Local-shell execution can dispatch an existing scheduler-runnable pending
    action only when the journal projection proves it has no receipt, open
    execution lease, or outcome-unknown reconciliation; the daemon execution
    lease remains the atomic worker claim.
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
    admission, durable execution lease acquisition, sandbox execution, and
    receipt append for local shell tools.
  - Lease-bound local-shell worker execution for already-admitted actions:
    re-check active grants, registry pin, admitted input digest, confinement,
    and observed side effects before completing the open daemon lease.
- `crates/beater-os-runtime`
  - Typed agent runtime loop over the daemon store: session bootstrap, bounded
    grant issuance, sequential step admission, and no-side-effect observation
    receipts.
  - Typed local-shell worker-once API that registers the exact command digest,
    selects a daemon-claimable admitted action, claims a durable execution
    lease, executes through the gateway, and returns the completed receipt plus
    projection summary.
  - Deterministic step replay evidence anchored to proposal, decision, receipt,
    journal-root, and receipt-root hashes.
  - Bundle projection summaries include open execution-lease recovery blockers
    pending/runnable action queues, admission blockers, and reconciliation
    counts for scheduler/operator visibility.
  - Service-facing `RuntimeBundle` contract used by daemon HTTP adapters without
    exposing direct store mutation APIs.
- `crates/beater-os-audit`
  - Trace/receipt validation, integrity reporting, read-only verification
    tooling, and full trace/action bundle serialization.
  - `verify-trace` checks exported full trace bundles offline without importing
    them into daemon state.

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
- `scripts/run-beater-os-runtime-worker-smoke.py`
  - Typed runtime worker-once smoke proving an admitted local-shell action is
    selected, claimed, executed through the gateway, completed with a receipt,
    and removed from the runnable queue.
- `scripts/run-beater-osd-http-execute-smoke.py`
  - Token-gated daemon HTTP execution smoke over the local shell gateway.
- `scripts/run-beater-osd-http-claims-smoke.py`
  - Token-gated daemon HTTP scheduler claim/complete smoke covering pinned
    tool compare-and-set refusal, exact lease-id completion, and journal
    verification.
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
