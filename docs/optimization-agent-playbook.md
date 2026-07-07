# Optimization Agent Playbook

Status: operating guidance for beaterOS agents working on performance,
language-boundary, compiler, scheduler, runtime, accelerator, and
close-to-metal changes.

This playbook supplements `final.md` and `docs/sota-systems-engineering.md`.
It does not replace measured implementation work. An optimization claim is not
accepted until a reviewer can replay the workload, identify the bottleneck, and
see that authority, receipts, and macOS compatibility were preserved.
Use `docs/engineering/optimization-evidence-runbook.md` for the compact packet
shape that PR authors and reviewers should fill out.
Use `docs/engineering/metal-os-blueprint.md` when the optimization changes the
hosted compatibility, Linux add-on, or future metal lane boundary.

## Current Toolchain Discipline

Toolchain facts change. At the start of any performance-sensitive PR, record the
date and the primary source used for the current compiler/runtime version.
`docs/source-matrix.md` keeps the latest repo-maintained snapshot for Rust,
LLVM, Zig, Swift, Go, Python, CUDA, and accelerator inputs. Do not treat that
snapshot as a pin; treat it as proof that agents must verify freshness before
making language or compiler claims.

Rules:

- Use the repo-pinned Rust toolchain in `rust-toolchain.toml` for baseline
  builds unless the PR is explicitly about a toolchain change.
- If a newer compiler is claimed to be faster, safer, or required, include the
  release source, local benchmark delta, compatibility result, and rollback
  plan.
- Do not chase nightly/dev compilers for TCB code unless a specific bug,
  target, sanitizer, or backend requires it and the fallback is documented.
- Record CPU architecture, OS version, compiler version, target triple,
  feature flags, and relevant environment variables with every benchmark.
- Record `rustc -vV` and `cargo -vV` for Rust benchmark packets so LLVM backend,
  host triple, and commit provenance are replayable.
- Treat compiler optimizations as part of the evidence chain. A result without
  command line, profile mode, and input fixture is not evidence.

## Language Boundary Rules

Default to Rust for authority, hot control-plane services, scheduler-facing
paths, model/tool/payment lanes, journals, receipts, memory projection,
conformance tooling that ships with the product, and native IPC.

Use other languages only for a named reason:

- C: stable ABI, driver, hypervisor, kernel/platform API, sandbox primitive,
  existing high-quality C library, or measured hot-path interop. Wrap it in a
  safe Rust API with explicit ownership and failure invariants.
- C++: vendor SDK, browser/embedder integration, compiler/runtime extension, or
  existing library where replacing it would add more risk than isolating it.
  Keep templates, exceptions, RTTI, allocation ownership, and thread ownership
  out of the authority boundary unless reviewed explicitly.
- Assembly: hardware entry, register work, context-switch stub, atomics,
  syscall veneer, or vetted cryptographic/platform primitive only.
- Zig: freestanding build experiments, cross-compilation glue, or isolated
  low-level probes. Do not make it a TCB language before the toolchain stability
  and reviewer depth are proven.
- Swift: Apple-native UI or platform integration where Swift is the platform
  boundary. Keep policy and receipts in Rust.
- Go: non-TCB infrastructure daemons where fast iteration and static deploys
  matter more than lowest-latency ownership control. Do not use it for the
  kernel authority path.
- Python: bounded audit, validation, and research scripts only.
- TypeScript: UI, browser ergonomics, and agent authoring only. It is a client
  of beaterOS authority, not the authority boundary.
- CUDA, Metal, SYCL, XLA, MLIR, shader languages, and vendor graph compilers:
  accelerator kernels and compilation backends behind beaterOS admission,
  scheduling, memory, telemetry, and receipt contracts.

When proposing a boundary, answer: why this language, why not Rust, what crosses
the boundary, who owns memory, how errors propagate, how cancellation works, how
the boundary is fuzzed or property-tested, and how macOS remains supported.

## Bottleneck Taxonomy

Before editing code, classify the suspected bottleneck. Pick the first matching
class, because lower classes often vanish after the higher class is fixed.

1. Contract work: unnecessary action, receipt, model call, serialization pass,
   hash, scan, retry, or approval loop.
2. Algorithm: wrong complexity, repeated lookup, missing index, unbounded
   fan-out, missing batch, poor cache key, or avoidable recomputation.
3. Data layout: cold fields in hot structs, pointer chasing, allocation churn,
   poor locality, oversized records, or branch-heavy enums in tight loops.
4. Copy/encoding: JSON churn, clone storms, string formatting, buffer growth,
   host-device transfer, DOM/screenshot duplication, or needless compression.
5. Syscall and IO: fsync count, stat/read/write loops, process spawn,
   descriptor churn, network round trips, DNS, TLS setup, or storage flushes.
6. Concurrency: lock contention, wakeup storms, queue backlog, priority
   inversion, cancellation delay, async task leaks, or retry amplification.
7. Scheduler and platform: CPU affinity, context switches, NUMA, page faults,
   power state, thermal throttling, timer granularity, or kernel feature gap.
8. Accelerator: kernel launch overhead, under-occupancy, model residency miss,
   HBM/VRAM/SRAM pressure, DMA/pinned-memory cost, partition contention,
   precision conversion, batch-size mismatch, or fallback route delay.
9. Provider/runtime: model cold start, token streaming latency, rate limit,
   SDK retry, remote queue, browser engine behavior, or cloud control-plane
   delay.

Each PR should state the class, the measured baseline, the target, and the
artifact that will catch regression.

Evidence map:

| Bottleneck class | Minimum useful evidence |
| --- | --- |
| Contract work | manifest/action count, receipt count, policy-decision trace, model/tool call count |
| Algorithm | complexity argument, input-size sweep, lookup/index profile, before/after benchmark |
| Data layout | allocation profile, cache/locality profile, struct-size check, clone count |
| Copy/encoding | copy/clone count, serialized bytes, buffer growth, JSON/binary encode timing |
| Syscall and IO | syscall count, fsync count, descriptor count, network round trips, storage latency trace |
| Concurrency | queue-depth spans, lock/wakeup profile, cancellation latency, retry count |
| Scheduler and platform | context-switch/page-fault profile, CPU affinity note, power/thermal state, timer latency |
| Accelerator | launch count, occupancy or provider metric, residency/copy bytes, queue delay, throttling |
| Provider/runtime | SDK trace, rate-limit/cold-start evidence, token latency, browser/cloud control-plane timing |

## Required Optimization Packet

A performance PR needs the smallest packet that proves the claim:

- workload: command, scenario, trace, fixture, or benchmark input
- baseline: current p50/p95/p99, throughput, memory, syscalls, copies, queue
  depth, model/tool calls, device occupancy, or other relevant metric
- budget: explicit success threshold and timeout behavior
- profile: Instruments, `sample`, `spindump`, Rust benchmark output, allocation
  count, syscall count, trace spans, `perf`, eBPF, Xcode GPU tools, Nsight,
  TPU/GPU provider metrics, or equivalent
- change: why the diff attacks the measured bottleneck
- safety: authority boundary, fail-closed behavior, receipt/audit replay, and
  rollback story
- portability: macOS path, Linux path if applicable, feature gate, and fallback
- regression: unit/property/scenario/benchmark/CI gate that would fail if the
  bottleneck or security bug returns

Do not accept "faster" without the baseline and the replay command. Do not
accept "more optimized language" without the boundary and safety packet.

Benchmark hygiene:

- run release/profile-mode builds unless the claim is explicitly about debug
  tooling
- record warmup count, sample count, variance/noise, machine load, CPU/GPU
  architecture, OS version, power mode, thermal state, and input size
- keep before/after commands, fixtures, feature flags, and environment variables
  identical except for the intended change
- report when results are too noisy to claim; do not round noise into evidence
- include p95/p99 for latency-sensitive paths and throughput plus tail latency
  for batched paths

Stop conditions:

- do not optimize cold paths unless they block correctness, security, or a
  measured user-facing workflow
- do not add FFI, unsafe code, assembly, or accelerator/vendor dependencies for
  marginal gains
- prefer deleting work, batching, caching, indexing, or layout fixes before
  adding a new abstraction
- require a rollback path for any change that increases TCB size, build
  complexity, operational state, or platform-specific behavior
- stop when the measured bottleneck moves outside the changed subsystem and open
  a follow-up instead of widening the PR

## Optimization Review Infrastructure

Optimization work should leave enough structure for the next agent to reproduce
and challenge the result without guessing.

Required infrastructure for serious performance work:

- benchmark manifest: workload name, input fixture, command, warmup, sample
  count, timeout, target machine class, and expected metric
- trace schema: spans for admission, queue wait, execution, journal append,
  receipt emission, model/tool/provider call, and accelerator enqueue/start/end
- profile artifact: Instruments, `sample`, Rust benchmark output, allocation
  counts, syscall counts, Nsight/Xcode GPU tools, TPU/GPU provider metrics, or
  equivalent
- regression gate: unit/property/scenario/benchmark/CI check tied to the
  bottleneck, with a tolerance that avoids noise while catching meaningful
  regressions
- review checklist: hot path, cold path, authority boundary, copy/allocation
  budget, syscall budget, queue bounds, cancellation, fallback, and rollback
- source note: current compiler/runtime/backend versions and links when the
  performance claim depends on toolchain or accelerator behavior
- lane note: hosted compatibility, Linux add-on, or metal-touching lane, with
  the reason this work belongs at that layer and the fallback path if the layer
  is unavailable

Expected repository locations as this infrastructure grows:

- `docs/engineering/optimization-evidence-runbook.md`: human replay packet and
  reviewer questions.
- `docs/engineering/metal-os-blueprint.md`: first-principles OS target, lane
  split, language boundaries, accelerator fabric, and optimization infra.
- `scripts/check-optimization-docs.py`: lightweight doctrine/source-matrix
  health gate.
- `benchmarks/manifest.json`: future benchmark registry for workload, fixture,
  warmup, sample count, timeout, and target budget.
- `scripts/run-optimization-benchmarks.py`: future replay runner for benchmark
  manifests and budget validation.
- `contracts/schema/performance-trace.schema.json`: future admission, queue,
  execution, journal, receipt, syscall, copy, allocation, p95/p99, and provider
  telemetry schema.
- `contracts/schema/accelerator-telemetry.schema.json`: future GPU/TPU/LPU/NPU,
  Apple Silicon, media-engine, enclave, and custom-silicon telemetry schema.
- `scripts/host-probes/`: future optional probes for macOS Apple Silicon, Linux
  CUDA, provider TPU, provider LPU, and other accelerator hosts.

Agents should create a small reusable skill, script, fixture, or checklist when
the same profiling or review task appears in more than one PR. Keep the artifact
boring: one command to reproduce, one place to read results, and one failure
mode that tells reviewers what changed.

## Agent Workflow

1. Read `AGENTS.md`, `docs/sota-systems-engineering.md`, and this playbook.
2. Name the invariant that must not break.
3. Name the hot path and cold path.
4. Gather a baseline before editing unless the task is only documentation.
5. Spawn subagents only for disjoint review, research, or implementation
   slices. Give each one a concrete file scope or question.
6. Make the smallest change that attacks the measured bottleneck.
7. Add or update a regression artifact.
8. Run the local gate and record what passed.
9. Ask for independent review of the authority and performance claims.

Useful subagent prompts:

- "Find every caller of this hot path and identify which ones are user-facing,
  authority-facing, or test-only."
- "Review this patch only for allocation/copy/syscall regressions and provide
  file:line findings."
- "Review this accelerator plan for host-device copies, queueing, residency,
  cancellation, telemetry, and fallback gaps."
- "Compare the language boundary against the playbook and identify any missing
  ownership, error, cancellation, or macOS story."

## Portable Accelerator Contract Sketch

Every accelerator backend should map to the same beaterOS shape before any
vendor API is called:

- `DeviceClass`: `cpu`, `gpu`, `tpu`, `lpu`, `npu`, `apple_gpu`, `apple_ane`,
  `media_engine`, `secure_enclave`, or future registered class
- `Backend`: vendor/runtime/compiler identity, version, driver/framework, target
  triple or device capability, and feature flags
- `AcceleratorJob`: manifest digest, principal, data class, model/artifact
  digest, precision, quantization, batch/streaming mode, expected p95/p99,
  timeout, cancellation token, and fallback route
- `MemoryPolicy`: host bytes, device bytes, pinned bytes, unified-memory bytes,
  HBM/VRAM/SRAM residency, spill policy, cache key, eviction rule, and
  sensitivity/residency constraints
- `QueuePolicy`: bounded depth, admission class, priority/fairness rule, maximum
  batch wait, tenant isolation, overload behavior, and retry limit
- `Telemetry`: enqueue/dequeue/start/finish timestamps, queue delay, launch
  count, copy/map/sync bytes, execution time, occupancy where available,
  throttling, errors, and fallback reason
- `Receipt`: placement, backend version, partition/slice identity when
  available, input/output digests, observed side effects, and replay evidence

Conformance tests should exercise at least one macOS backend path and one Linux
or provider backend path once those implementations exist. A backend that cannot
report a field must record that limitation explicitly rather than silently
pretending the value is zero.

## Accelerator Review Packet

GPU, TPU, LPU, NPU, Apple Silicon, media-engine, enclave, and custom ASIC work
must include:

- accelerator class and vendor backend, with a vendor-neutral beaterOS contract
- model/artifact digest, runtime/compiler version, precision, quantization, and
  deterministic seed where meaningful
- memory budget split across host RAM, device memory, pinned memory, HBM/VRAM,
  SRAM, cache, and spill paths
- host-device copy count and bytes, launch count, queue delay, execution time,
  and observed throttling where available
- isolation story: MIG, VM/pod slice, process, sandbox, microVM, device ACL, or
  conservative single-tenant scheduling
- cancellation and timeout behavior that actually releases scarce device work
- data-class and residency policy for weights, embeddings, prompts, traces, and
  outputs
- fallback route when the accelerator is unavailable, overloaded, revoked, or
  too expensive
- receipt fields that prove placement, version, model digest, input/output
  digest, timing, queueing, and observed effects

Vendor SDKs can provide execution. They cannot provide authority.

Apple Silicon-specific checks:

- identify whether the path uses CPU SIMD, Metal GPU, Metal Performance Shaders,
  Core ML, ANE/Core AI-style framework routing, media engines, or the secure
  enclave
- account for unified memory as shared pressure, not free transfer: bandwidth,
  cache pressure, synchronization/fence cost, page migration, mapped-vs-copied
  buffers, and total RSS impact
- provide fallback when ANE/GPU placement is unavailable, opaque, revoked by the
  platform, thermally throttled, or not observable enough for the risk class
- use Xcode/Instruments/Metal tooling or framework telemetry where available,
  and record when placement or timing is hidden by the framework

SIMD-specific checks:

- document feature detection, target features, compiler flags, alignment,
  scalar fallback, vector-width assumptions, and precision/determinism drift
- benchmark compiler auto-vectorization before adding intrinsics
- keep unsafe vector code behind a small safe API with property tests comparing
  scalar and vector paths
