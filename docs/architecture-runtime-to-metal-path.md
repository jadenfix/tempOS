# Runtime-First Architecture Path for beaterOS

This file is the systems-engineering map for moving from hosted agent runtime to metal-facing execution.
It operationalizes `final.md` and the bare-metal readiness tooling so each PR can be reviewed against a real migration path.

## 1) Current operating principle

- **Phase 0 (Invariant):** Keep the public OS claims in `final.md` stable and never weaken the model.
- **Phase 1 (Runtime first):** Make the hosted agent kernel complete and measurable.
  - Session contracts
  - Grant admission + decisioning
  - Receipt and journal invariants
  - Sandbox mediation for high-risk side effects
  - Replayable audit surface
- **Phase 2 (Service expansion):** Add stable service lanes above the same authority boundary.
  - Tool registry + MCP/A2A gateway
  - Shell/code/browser/payments/eval lanes
  - Memory projection and incident tooling
- **Phase 3 (Measured metal move):** Move only what this host/runtime contract proves cannot be safely done through hosted compatibility lanes.
  - Introduce explicit accelerator/resource contracts for new lanes.
  - Preserve fail-closed policy semantics at every new hardware boundary.
  - Treat GPU, TPU, LPU, NPU, Apple Silicon, media-engine, enclave, and future
    ASIC execution as scheduled OS resources with admission, bounded queues,
    memory/residency budgets, cancellation, telemetry, receipts, and fallback.

A PR is only valid for this objective if it increases capability in a layer *above* the previous layer, or explicitly proves a reason to move down-stack.

## 2) Mapping from `final.md` to repo scope

- `final.md` §7, §10 → Runtime contracts and control plane
  - Implemented primarily in `crates/beater-os-core`, `crates/beater-osd`, `crates/beaterosctl`.
- `final.md` §10, §11, §13 → Safety and service boundaries
  - Implemented in service crates + registry + sandbox + audit tooling.
- `final.md` §14 → Eval and gating
  - Implemented as scenario/e2e gate infrastructure plus CI/local gate plan.
- `final.md` §18 + §8.1.1 → Hosted-first compatibility lane
  - Implemented via the bare-metal readiness manifest and matrix.
- `final.md` §26 → Non-negotiable invariants
  - Enforced through PR checklists, governance ledger, and runtime/tests in CI/local gates.

## 3) Runtime-to-metal migration model

### 3.1 Required control-plane baseline

A host is **runtime-compatible** when:
- control-plane lane is runnable,
- required workload class `policy-admission` is routeable,
- optimal route for `policy-admission` is `portable-control-plane`.

The implementation requirement is enforced in the local gate with:
`scripts/check-bare-metal-readiness.py --require-migration-phase runtime`.

### 3.2 Optional-lane proof is not metal-OS proof

A host has **optional-lane readiness** when:
- runtime requirements above hold,
- at least one optional lane is also runnable (e.g. `linux-cuda-lane` or `apple-metal-lane`),
- PRs that claim metal behavior gate their matrix cases with `require_migration_phase=metal-ready`.

The current readiness manifest still uses the compatibility label
`metal-ready` for this gate because it proves the host can route beyond the
portable control plane. That label is not evidence that a full metal OS contract
exists, and it must not let a Linux CUDA, Apple Metal, provider SDK, or other
optional accelerator path stand in for portable authority, scheduling, memory,
receipt, telemetry, fallback, or macOS behavior. PRs that need true
metal-touching evidence must name the boundary being moved and provide traces,
benchmarks, security evidence, and rollback proof showing the hosted and Linux
add-on lanes cannot satisfy the contract.

### 3.3 Runtime-first execution rule for every PR

For this repo, any PR is valid only if it demonstrates this sequence:

1. **Runtime contract remains valid on host**: host still supports
   `portable-control-plane` with workload classes required by the slice.
2. **Migration frontier not weakened**: mandatory runtime lanes remain runnable,
   dependency order is unchanged unless explicitly re-reviewed, and `policy_version`
   remains authoritative in daemon-owned admission paths.
3. **Non-runtime expansion is isolated**: changes beyond runtime are either
   fenced under specific lanes (`apple-metal-lane`, `linux-cuda-lane`, etc.) or
   explicitly blocked behind their own feature flags/migration profile.
4. **Evidence is emitted**: each PR updates/consumes a concrete artifact
   (`runtime`/`metal-ready` readiness report, matrix case, manifest update, or
   host-profile capture) proving the stated frontier.

### 3.4 Migration gates already encoded in repo artifacts

- `docs/engineering/bare-metal-readiness-manifest.json`: lane graph, migration order, profiles.
- `docs/engineering/bare-metal-readiness.md`: readiness rules and payload contract.
- `scripts/check-bare-metal-readiness.py`: manifest validation + host matching + route scoring.
- `scripts/run-bare-metal-e2e-matrix.py`: deterministic PR-friendly matrix of routing assertions.
- `docs/engineering/bare-metal-e2e-matrix.json`: scenario fixtures, including phase gates.
- `scripts/local-e2e.py`: repo baseline gate now requiring `runtime` phase.

## 4) PR execution pattern (for this repository)

Each PR should map to exactly one migration layer:

- **Kernel/runtime slice:** contracts, session lifecycle, policy admission, journal/receipt semantics.
- **Service slice:** sandbox/tool/receipt/eval/memory/review layer.
- **Linux add-on or metal-adjacent slice:** accelerator, Linux-native, or
  platform-specific hardening with explicit readiness evidence and no claim that
  optional provider readiness is a portable metal OS contract.

For every slice include:
- A stated target layer.
- `final.md` section anchors.
- What changed in this layer is now runnable without host assumptions that were not proven in prior layers.
- Whether this PR affects the migration frontier and if so, which matrix case it extends.

## 5) Repository execution map

The next operating model is explicit by layer:

- **Layer A — Hosted runtime kernel (mandatory first):**
  - `crates/beater-os-core`
  - `crates/beater-os-session`
  - `crates/beater-os-sandbox` (policy-gated lanes)
  - `crates/beater-osd`
  - `crates/beaterosctl`
- **Layer B — Service planes (opt-in after A):**
  - `crates/beater-os-memory`
  - `crates/beater-os-audit`
  - `crates/beater-os-tool-registry`
- **Layer C — Platform/metal pilots (only with proof):**
  - `apple-metal-lane` workloads in manifest profiles
  - `linux-cuda-lane` workloads in manifest profiles
  - future TPU, LPU, NPU, media-engine, enclave, and custom-silicon lanes only
    after portable accelerator contracts and replay evidence exist
  - micro-VM/hypervisor or driver-adjacent experiments

A PR must target only one layer unless the release explicitly proves transition.

## 6) Runtime-to-metal validation checklist (practical)

- **Pre-merge local gate:** `python3 scripts/local-e2e.py` (or equivalent split
  gates) includes daemon smoke, readiness phase, and matrix coverage.
- **Runtime slice proof:** `scripts/run-beater-osd-runtime-smoke.py --json` and
  `scripts/run-beater-os-runtime-smoke.py --json` must pass on the target
  environment when runtime-layer contracts are touched.
- **Migration proof for this slice:**
  - runtime-only slices: `require_migration_phase=runtime`
  - optional-lane slices: `require_migration_phase=metal-ready` and explicit
    route/lane assertions, while documenting that this proves optional provider
    readiness rather than a full metal OS boundary
  - manifest/profile edits require updated migration metadata and matrix case.
- **Review proof:** PR checklist (`docs/governance/review-checklist.md`) includes
  explicit references to `final.md` and a no-author review entry in the
  coordination ledger.

## 7) Engineering stack discipline

### 7.1 Language/Framework rule

- Keep control-plane and authority surfaces in Rust.
- Use Python for infra scripts and reproducibility surfaces where startup cost is low and safety claims are declarative.
- Use C/C++ only when there is a stable ABI, driver/hypervisor boundary, or measured requirement.
- No framework-level rewrite from faster languages without evidence (profile + baseline + target + regression gate).

### 7.2 Algorithmic rule

For every optimization claim, classify one of:
- contract simplification,
- algorithmic fix,
- allocation/copy reduction,
- queue/backpressure correction,
- bounded scheduling fix.

A change without one of these categories is reviewed as a refactor and should state why it is required.

## 8) Current hard requirements for this path

1. Do not reduce migration evidence.
2. Do not introduce a path that bypasses `PolicyEngine`.
3. Do not add side-effect behavior without manifest + manifest hash + receipt evidence.
4. Keep macOS/portable surfaces as the baseline for correctness.
5. Any metal claim must have:
   - a lane profile in the readiness manifest,
   - at least one matrix case asserting that lane,
   - host assumptions in code and docs.

## 9) What to do next

- Continue with kernel/service slice completion in `docs/implementation-backlog.md`.
- Extend matrix cases for any new optional lane claim.
- Require PR review sign-off on:
  - migration frontier impact,
  - security invariant retention,
  - macOS compatibility path,
  - no-ambient-authority proof.
