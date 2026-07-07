# Metal OS Blueprint

Status: first-principles architecture blueprint for the tempOS/beaterOS
long-horizon operating-system program.

This document is not a promise to skip the hosted runtime. It is the map that
keeps the hosted runtime, Linux add-on work, and future metal-touching OS work
pointed at the same target: an agent-first OS whose authority, scheduling,
memory, IO, accelerator, audit, and recovery boundaries are native to the
system rather than bolted onto an app framework.

## First-Principles OS Shape

An operating system built in 2026 for agent workloads should start from scarce
resources, adversarial inputs, and replayable authority:

- **Authority before execution.** A side effect is not executable until it has a
  typed action manifest, scoped capability, deterministic policy decision, and
  expected receipt requirements.
- **Receipts before stories.** Logs and model explanations are secondary. The
  OS-level truth is a replayable chain of proposals, decisions, receipts,
  memory projections, payment evidence, and audit roots.
- **Schedulers understand risk.** CPU time is only one dimension. The scheduler
  must also account for model spend, payment spend, data sensitivity, tool risk,
  human-review queues, accelerator residency, rollback cost, and p95/p99
  latency budgets.
- **Memory has provenance.** Agent memory is not an opaque vector store. Hot
  context, working memory, embeddings, durable traces, redacted archives, and
  policy memory are separate tiers with source, sensitivity, expiry, and
  rebuild rules.
- **IO is completion-oriented and bounded.** Hot IO paths use batching, rings,
  completion queues, stable buffers, backpressure, and explicit fsync/network
  budgets where the platform supports them.
- **Accelerators are OS resources.** GPU, TPU, LPU, NPU, Apple Silicon local
  accelerators, media engines, enclaves, and future ASICs are scheduled devices
  with queues, memory residency, isolation, cancellation, telemetry, and
  receipt evidence.
- **The trusted core stays small.** Policy admission, principal identity,
  capability checks, queue admission, journal integrity, and receipt verification
  are core. Browser UI, dashboards, model-provider SDKs, and workflow ergonomics
  are clients.

The compatibility runtime proves these contracts first. The metal lane only
receives work when hosted traces show that the host OS cannot express the right
authority, latency, isolation, memory, IO, accelerator, or audit contract.

## Three Engineering Lanes

### Hosted Compatibility Lane

The default shipping lane is a Rust agent kernel on macOS, Linux, containers,
browsers, and cloud VMs. It owns sessions, grants, action manifests, policy
decisions, receipts, journals, memory provenance, payment mandates, eval gates,
and sandbox/tool mediation.

This lane must stay healthy because it is the evidence generator for every
lower-level move.

### Linux Add-On Lane

The Linux add-on lane is separate from the whole-OS metal lane. It experiments
with Linux-native primitives when they improve agent workloads without changing
the beaterOS authority contract:

- `sched_ext` and BPF scheduler experiments for policy-aware scheduling.
- cgroups, namespaces, seccomp, LSMs, and microVMs for containment.
- `io_uring`, zero-copy paths, XDP/eBPF, DPDK, and SPDK for measured IO paths.
- Rust-for-Linux or C kernel/module boundaries only when the platform contract
  requires them and the hosted/runtime contract remains the source of authority.

Linux add-ons are accelerators for learning and deployment. They are not a
license to make Linux-specific behavior the portable OS contract.

### Metal-Touching OS Lane

The metal lane builds new OS pieces only where the compatibility and Linux lanes
produce evidence that existing kernels cannot satisfy the contract cleanly. Good
first targets are narrow and measurable:

- policy-aware task admission and scheduling,
- bounded context and memory-tier management,
- zero-copy trace, journal, and receipt transport,
- high-assurance sandbox or microkernel appliances,
- accelerator queue and residency management,
- crash recovery with append-only evidence,
- capability hardware or CHERI/seL4-style high-assurance research targets.

Broad desktop replacement, universal driver coverage, and a polished shell are
not first targets.

## Language And Toolchain Baseline

Use the best language for the subsystem and boundary:

- Rust for authority paths, daemons, CLIs, journals, receipts, policy,
  scheduler-facing services, memory projection, native IPC, and hot control
  plane code.
- C for stable ABI, kernel/platform APIs, driver, hypervisor, sandbox, or
  measured interop boundaries.
- C++ for vendor SDKs, browser/embedder work, compiler/runtime extension, or
  existing libraries when isolation is lower risk than rewriting.
- Assembly for boot, register work, context switching, syscall veneers, atomics,
  CPU feature probes, or vetted hardware primitives only.
- CUDA, Metal, XLA/StableHLO, shader languages, and vendor graph compilers for
  accelerator backends behind beaterOS admission and receipt contracts.
- TypeScript, Swift, Go, and Python only where their platform or iteration
  advantage outweighs their unsuitability for the authority boundary.

Toolchain facts are temporal. Agents must use `docs/source-matrix.md` and
primary sources before claiming "latest", "current", "GPU optimized",
"TPU optimized", "LPU optimized", or "Apple Silicon optimized". The repo-pinned
toolchain remains the baseline until a PR proves a migration with compatibility,
performance, rollback, and review evidence.

## Accelerator Fabric

An accelerator backend is valid only if it implements a portable beaterOS job
shape before calling a vendor API:

- device class, backend, driver/framework/compiler version, and target features,
- model or artifact digest, data class, precision, quantization, and seed where
  meaningful,
- host RAM, device memory, pinned memory, unified-memory, HBM/VRAM/SRAM,
  cache-residency, and spill budgets,
- bounded queue depth, priority/fairness policy, maximum batch wait,
  cancellation, timeout, overload behavior, and fallback route,
- copy/map/sync bytes, launch count, queue delay, execution time, occupancy or
  provider metric where available, throttling, and errors,
- receipts that bind placement, backend version, partition/slice identity,
  input/output digests, timing, and observed side effects.

Discrete GPU memory, TPU pod memory, LPU provider queues, Apple unified memory,
ANE/Core ML-style opaque placement, media engines, and secure enclaves all need
different measurements. They still share one admission, telemetry, receipt, and
fallback contract.

## Optimization Infrastructure For Agents

Optimization-heavy work must leave a replay trail for the next agent:

- bottleneck taxonomy: contract, algorithm, layout, copy/encoding, syscall/IO,
  concurrency, scheduler/platform, accelerator, or provider/runtime,
- benchmark or scenario manifest with workload, fixture, warmup, sample count,
  timeout, target machine class, and expected metric,
- trace spans for admission, queue wait, execution, journal append, receipt
  emission, model/tool/provider call, and accelerator enqueue/start/end,
- profile artifacts from Instruments, `sample`, Rust benchmarks, allocation
  counters, syscall counters, Nsight, Xcode GPU tools, TPU/GPU provider
  metrics, or equivalent,
- regression gate with a tolerance that catches meaningful regressions without
  pretending noisy microbenchmarks are proof,
- independent review for both performance and authority claims.

If the same optimization review appears twice, create or update a small skill,
script, fixture, or checklist. Keep it boring: one command to reproduce, one
place to read results, and one failure mode that tells reviewers what changed.

## Review Gate

A PR that claims metal, language, compiler, runtime, or accelerator progress
must answer:

- What scarce resource is protected?
- What is the hot path and what is explicitly off-path?
- What is the bottleneck class?
- What baseline and p95/p99, throughput, memory, syscall, copy, queue,
  accelerator, or spend budget were measured?
- What primary source and version date support current toolchain/backend claims?
- What authority boundary is preserved and what receipt proves it?
- If the boundary is network-facing or model/tool-facing, what validates the
  concrete endpoint after DNS, proxy resolution, redirects, and retries?
- If the boundary is local control plane, loopback, browser, or IPC, what
  unguessable same-user capability backs Host and Origin checks?
- What live-state quota bounds repeated valid commands, frames, sessions,
  screenshots, logs, DOM data, tool results, provider calls, and queue entries?
- Where is the size cap enforced while reading, collecting, diffing, or
  serializing remote-driven data, rather than after full materialization?
- How are untrusted remote tool catalogs classified so missing side-effect
  metadata cannot weaken local side-effect policy?
- How is page-controlled or provider-controlled metadata framed as untrusted
  provenance before it enters model prompts, traces, or review packets?
- If durability changes, what checksum, recovery, corruption detection, or
  self-heal rule preserves append-only journal and receipt integrity?
- What macOS path exists?
- What Linux or provider path exists, if applicable?
- What fallback and rollback path exists?
- What test, benchmark, trace, matrix case, or CI gate catches regression?

No vendor SDK, compiler release, or language choice replaces this evidence.
