# beaterOS Bare-Metal Readiness Program

This document defines the next implementation lane for moving into metal-facing work:
start from contracts, measure where host abstractions saturate, and only then add
new low-level ownership or scheduler boundaries.

## Current lane intent

- Keep the compatibility lane shipping on Linux/macOS from contracts and runtime
  today.
- Add a reusable capability model so every bare-metal-oriented PR maps to a
  machine class and accelerator class budget.
- Keep this lane measurable: each PR has an optimization packet and a host class
  plan with explicit queue/copy/fallback constraints.


## Architecture lane contract

- `architecture.control_plane_lane` names the always-available control-plane lane that must remain portable and stable.
- `architecture.lanes` defines execution lanes for control-plane, accelerator experimentation, and policy-bounded batch paths.
- `architecture.migration_order` is a required lane-order list used by planning and rollout checks.
- Each lane references a `profile` and a `workload_classes` list; the validator enforces profile existence and known workload classes.
- Lanes are dependency-ordered by `depends_on`; cycles are rejected to keep migration and scheduling order deterministic.
- `--report` now includes a `migration_plan` and `workload_routing` artifact for host-aware lane planning.
- `--report` also emits `architecture.migration_phase` with `runtime`, `metal-ready`, or `blocked`.
- `--report-only` emits machine-readable JSON without human summary lines.
- `--require-workload-class` can be used by e2e gates to assert a workload class has a ready routing lane on a host.
- `--require-workload-route` can assert the *optimal* routing result for a workload class (e.g. `policy-admission=portable-control-plane`) and uses the same scoring used by `preferred_workload_routes`.
- `--require-migration-phase` locks an execution host to a lane phase (`runtime` or `metal-ready`) to prevent accidental jumps before hosted controls are stable.
- `--require-lane` and `--require-control-plane-lane` both validate the lane is currently runnable in `migration_plan` (dependency-ready), not only host-compatible.
- `fallback_chain` guarantees there is a lower-cost profile to route work when a profile lane is not available.
- `--report` now emits a resource-aware migration plan and workload routing map so bare-metal PRs can validate what is actually runnable on a host snapshot.

The local e2e gate requires the host to match the manifest's control-plane lane via
`--require-control-plane-lane` so the repository always proves the control-plane path
remains runnable on compatible hardware before merge.

For reproducible host checks, tests and CI can provide an explicit host snapshot via:
- `BEATEROS_HOST_OS`
- `BEATEROS_HOST_ARCH`
- `BEATEROS_HOST_MEMORY_GIB`
- `BEATEROS_HOST_STORAGE_IOPS`
- `BEATEROS_HOST_MEMORY_BANDWIDTH_GBPS`
- `BEATEROS_HOST_GPU_MEM_GIB`
- `BEATEROS_HOST_PCIE_BWL_GBPS`
- `BEATEROS_HOST_RESIDUAL_LATENCY_MS`
- `BEATEROS_HOST_GPU_TEMP_C`
- accelerator feature flags: `BEATEROS_ACCELERATOR_CPU`, `BEATEROS_ACCELERATOR_CUDA`, `BEATEROS_ACCELERATOR_APPLE_GPU`, `BEATEROS_ACCELERATOR_TPU`, `BEATEROS_ACCELERATOR_ENCLAVE`

For scripted CI runs and reproducible local gates, use:
- `scripts/collect-bare-metal-host-profile.py` to capture a JSON profile.
- `python3 scripts/check-bare-metal-readiness.py --host-profile <path>` to evaluate readiness against that snapshot.
  The `--report` payload now includes:
  - `architecture.migration_plan`: ordered lane readiness with fallback dependencies.
  - `architecture.workload_routing`: all host-ready lanes per workload class.
  - `architecture.preferred_workload_routes`: selected optimal lane per workload class with score metadata.
    - `lane` is the chosen lane name.
    - `score` is the computed lane score used for selection.
    - selection first minimizes `score` and then uses migration-order as a tie-breaker.
  - `architecture.lane_scores`: per-lane score breakdown including resource/optimization components and penalty trace.
  - score model:
    - resource pressure: each min constraint contributes `required / host_value`; each max
      constraint contributes `host_value / cap`.
    - optimization pressure: lower-is-better metrics use `metric * 0.01`; higher-is-better
      metrics use `1 / (1 + metric) * 0.01`.
    - lane score is resource pressure + optimization pressure (lower is better).
- `python3 scripts/local-e2e.py --host-profile <path>` to pass the same profile into the local e2e readiness gate.
- `python3 scripts/run-bare-metal-e2e-matrix.py` to run a deterministic matrix of
  host classes for portable/cuda/metal routing and expected-fail lanes.
- `python3 scripts/run-bare-metal-e2e-matrix.py --validate-only` to validate the matrix
  fixture structure and manifest references without executing per-case checker runs.
- `python3 scripts/run-bare-metal-e2e-matrix.py --report-json <path>` when CI or PRs
  need a machine-readable matrix artifact.
- Matrix cases default to `docs/engineering/bare-metal-e2e-matrix.json`, with support for
  adding CI-specific and PR-specific case overrides.
- Matrix cases are prevalidated before execution:
  - referenced `require_profile` must resolve to a manifest profile.
  - referenced `require_lane` / route targets must resolve to manifest lanes.
  - referenced required workload classes must exist in lane workload coverage.
  - invalid route shapes fail fast.

- Keep author-review separation: the implementation PR author never merges their own
  PR, and PRs for this lane need a separate reviewer.

## What this file means for implementation

1. If a PR claims bare-metal value, it must include at least one path in the
   readiness manifest that it changes or requires.
2. If a PR changes `docs/sota-systems-engineering.md`, `docs/optimization-agent-playbook.md`
   or `final.md` clauses that affect metal-lane behavior, that PR also updates this
   manifest entry to avoid drift.
3. PRs changing bare-metal pathways must either preserve or tighten one of:
   - authority boundary (`policy`, `admission`, `receipt`, `audit`, `memory`)
   - data movement budget (`copy`, `resident`, `queue`, `serialization`)
   - fallback story (`cpu fallback`, `microVM fallback`, `software control path`)

## Progress policy

- Every PR in this lane:
  - updates `docs/engineering/bare-metal-readiness-manifest.json` if machine-class
    assumptions change,
  - adds or updates a test for manifest schema or host-class validation logic,
  - lands with a non-author reviewer and non-author merge.
- This repository ships this with local e2e via
  `scripts/check-bare-metal-readiness.py`, so changes cannot be merged without
  passing at least manifest validation and report synthesis.

## Manifest contract (for this slice)

- `schema_version`: integer contract version; new changes require migration review.
- `profiles`: list of hardware or platform profiles used for planning.
- `architecture`: required lane graph and migration order:
  - `control_plane_lane`
  - `migration_order`
  - `lanes`
- Each profile defines:
  - `name`, `scope`, `stability_tier`, `target_os`, `target_arch`
  - `resource_contract` envelope with hard minimum and bounded resource assumptions. Min/max constraints are enforced by the host-aware check.
  - `accelerators` entries with required and fallback semantics
  - `optimization_targets` with concrete measurable limits for p95 and throughput.

## Language and optimization baseline

Use Rust for control plane/kernel-like code. Use C only for stable ABI,
platform/driver boundaries, or measured hot-path interop after profiling. Use Python
for this manifest and readiness checker slice because its purpose is to keep the
e2e program honest and lightweight.

## Next concrete slice in progress

- Add script-backed manifest validation (`scripts/check-bare-metal-readiness.py`).
- Add local-e2e gate to ensure the readiness contract is always checked.
- Extend tests for schema shape + host-compatibility checks.
- Add deterministic bare-metal matrix coverage via `scripts/run-bare-metal-e2e-matrix.py`.
- Keep the manifest authoritative for any machine-class claim in this lane.
