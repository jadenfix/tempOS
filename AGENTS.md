# beaterOS Agent Context

Use this file as startup context for Codex, Claude Code, Cursor, Copilot, and
other coding agents working in this repository.

## What beaterOS Is

beaterOS is an agent-first operating-system research and implementation repo.
The source-of-truth product plan is [final.md](final.md). Implementation must
turn that plan into reviewed, measurable, macOS-compatible slices without
shortening or weakening the plan.

The project has two explicit lanes:

- Compatibility lane: a hosted Rust agent kernel and runtime that makes Linux,
  macOS, containers, browsers, tools, models, memory, and payments safe for
  agents now.
- Metal lane: a long-horizon, first-principles OS stack that can touch scheduler,
  memory, IO, devices, isolation, authority, audit, and recovery boundaries when
  hosted traces prove those boundaries need to move closer to hardware.

The first implementation layer is a Rust workspace with kernel-facing contracts:
agent sessions, capability grants, action manifests, policy decisions, receipts,
and append-only journals. Future runtime work must preserve those contracts as
authority and audit boundaries, not merely as serialization types.

Tempo, beater.js, beatbox, beater-memory, and future ecosystem components should
run on these contracts. UI and agent ergonomics may live in higher-level
languages, but authority, admission, journaling, receipt verification, memory
projection, and scheduler-facing paths terminate in native beaterOS services.

GPU, TPU, LPU, NPU, Apple Silicon-style local accelerators, enclaves, media
engines, and future agent ASICs are first-class OS resources. Accelerator work
must stay behind beaterOS admission, scheduling, memory, receipt, telemetry,
data-class, and fallback contracts; do not let a vendor SDK become the authority
boundary.

## Repo Shape

- `Cargo.toml` is the Rust workspace.
- `crates/beater-os-core` contains core contracts, policy admission, hashing,
  journal verification, and receipt-chain logic.
- `crates/beater-os-sandbox` is the scoped local execution lane: canonicalized
  filesystem confinement, scrubbed environment, bounded execution, and
  filesystem-diff receipts (final.md §8, §13.8).
- `crates/beater-os-tool-gateway` is the runtime mediation layer that resolves
  registered tools, derives manifests, asks `beater-osd` for admission, executes
  admitted local shell tools through the sandbox, and records receipts.
- `docs/implementation-backlog.md` maps `final.md` into PR-sized slices and
  review rules.
- `docs/sota-systems-engineering.md` is the performance, language, security, and
  macOS engineering doctrine for this project.
- `docs/optimization-agent-playbook.md` is the agent workflow for
  performance-sensitive implementation, language-boundary decisions, compiler
  freshness checks, bottleneck analysis, accelerator review packets, and
  benchmark/trace evidence.
- `.codex/skills/beateros-systems-engineering/SKILL.md` packages that doctrine
  as a reusable Codex skill.
- `.codex/skills/beateros-pr-review/SKILL.md` packages review and repo-governance
  tasks for non-author review and repetitive infra/docs obligations.
- `CLAUDE.md` and `.cursor/rules/beateros.mdc` keep equivalent guidance close to
  Claude Code and Cursor.

## Non-Negotiables

- Keep PRs scoped and reviewed. Every feature lands through a PR, and no author
  merges their own PR.
- Do not weaken `final.md`. Add clarifying docs or implementation artifacts
  around it unless the user explicitly asks to edit it.
- Treat performance as an architectural property. Identify the hot path, syscall
  budget, allocation budget, copy budget, queue bounds, and p95/p99 target before
  optimizing syntax.
- Treat security as a systems invariant. Capabilities, receipts, policy
  decisions, memory, payments, tools, and model calls must fail closed and be
  replayable from evidence.
- Make macOS work. The repo must build and test on macOS, including Apple
  Silicon. Do not introduce Linux-only assumptions without an abstraction and a
  macOS path.
- Use the best language for the subsystem and boundary. When tradeoffs are
  close, prefer Rust. Use C for stable ABI, boot/platform, driver, hypervisor,
  existing C library, or measured hot-path interop needs. Use assembly only at
  the hardware boundary. Isolate and review all unsafe code.
- For performance-sensitive work, verify current compiler/runtime facts from
  primary sources, record toolchain versions with benchmark evidence, and follow
  `docs/optimization-agent-playbook.md`.
- Optimize from first principles: remove work, batch/cache, reduce copies and
  syscalls, improve layout, then specialize. Move closer to metal only with a
  trace, benchmark, profile, or security proof that the current boundary cannot
  satisfy.
- For accelerator paths, account for host-device copies, HBM/VRAM/SRAM
  residency, pinned memory, queue delay, kernel launch overhead, model/artifact
  digests, partitioning, thermals, power, cancellation, and fallback routes.

## SOTA Systems Engineering Checklist

Before designing or reviewing a substantial change, read
[docs/sota-systems-engineering.md](docs/sota-systems-engineering.md). At minimum,
be able to answer:

- What is the critical path, and what is explicitly off the critical path?
- What are the data ownership, lifetime, and copy rules?
- Which queue, cache, journal, network, filesystem, or model call can back up?
- Which accelerator queue, model residency cache, host-device transfer, or
  silicon partition can back up?
- What is bounded by construction: memory, CPU, IO, tool calls, model spend,
  payment spend, retries, and wall-clock time?
- Which authority boundary does this touch, and what evidence proves it?
- Which compiler/runtime versions were used, and are they repo-pinned or part of
  the claim?
- Which bottleneck class is being addressed: contract work, algorithm, layout,
  copy/encoding, syscall/IO, concurrency, scheduler/platform, accelerator, or
  provider/runtime?
- What benchmark, trace, property test, or scenario would catch a regression?
- Why is the chosen language boundary the best fit, and if the tradeoff was
  close, why not Rust?

## Common Commands

```sh
cargo fmt --all -- --check
cargo test --workspace --locked
cargo clippy --workspace --all-targets --locked -- -D warnings
git diff --check
TMPDIR=/private/tmp python3 scripts/local-e2e.py
```

## Performance-Sensitive PR Packet

Paste this into any PR that claims a performance, language-boundary, compiler,
runtime, accelerator, or close-to-metal improvement:

```md
### Optimization Packet

- Workload:
- Replay command:
- Bottleneck class:
- Baseline:
- Target budget:
- Profile/trace artifact:
- Compiler/runtime/backend versions:
- Authority boundary preserved:
- Copy/allocation/syscall/queue/device budget:
- macOS path and fallback:
- Regression gate:
- Independent reviewer for performance + authority:
```
