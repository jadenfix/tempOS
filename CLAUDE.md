# tempOS Claude Code Rules

Start with [AGENTS.md](AGENTS.md). It is the compact repo map and cross-agent
context. The full plan is [final.md](final.md), and the systems-engineering
doctrine is [docs/sota-systems-engineering.md](docs/sota-systems-engineering.md).
Optimization workflow lives in
[docs/optimization-agent-playbook.md](docs/optimization-agent-playbook.md),
the full OS lane blueprint lives in
[docs/engineering/metal-os-blueprint.md](docs/engineering/metal-os-blueprint.md),
replay packet rules live in
[docs/engineering/optimization-evidence-runbook.md](docs/engineering/optimization-evidence-runbook.md),
and temporal language/backend source snapshots live in
[docs/source-matrix.md](docs/source-matrix.md).

## Contract Rule

`final.md` is the source-of-truth plan. Do not shorten it, dilute it, or replace
it with implementation notes. Add focused docs, tests, contracts, and code around
it, then keep changes PR-scoped and reviewed by a non-author.

## SOTA Systems Rule

tempOS should be designed like close-to-metal systems software:

- Start from invariants, budgets, and bottlenecks, not framework preference.
- Prefer simple data layouts, bounded queues, explicit ownership, and measurable
  hot paths.
- Keep the policy/audit path deterministic and replayable.
- Never trade away capability safety, receipt integrity, or sandbox isolation for
  cosmetic speed.
- Use the best language for the subsystem and boundary. When tradeoffs are
  close, prefer Rust. Use C when ABI, platform, driver, hypervisor, existing C
  library, or measured hot-path constraints require it. Use assembly only for
  boot, atomics, context switching, register access, or similarly unavoidable
  hardware boundaries.
- Any unsafe, C, or assembly surface must be tiny, documented, fuzzable or
  property-tested where practical, and wrapped in safe Rust APIs.
- Keep macOS and Apple Silicon as first-class development targets. Linux-specific
  mechanisms need an abstraction, a macOS fallback, or an explicit future-target
  label.
- Keep hosted compatibility, Linux add-on, and true metal-lane work separate.
  Linux primitives are allowed when measured; they are not the portable
  authority contract.
- GPU, TPU, LPU, NPU, Apple Silicon, media-engine, enclave, and future ASIC paths
  are OS resources behind tempOS admission, queues, telemetry, receipts, and
  fallback contracts.

## Before Coding

For non-trivial changes, state or encode:

- critical path and non-critical path
- allocation/copy/syscall expectations
- queue and retry bounds
- failure mode under overload
- security boundary and required evidence
- language choice and Rust tie-breaker analysis
- macOS impact
- local verification command
- current compiler/runtime/backend source when the claim depends on "latest" or
  current-version behavior
