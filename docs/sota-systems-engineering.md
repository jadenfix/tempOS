# SOTA Systems Engineering Doctrine

This document defines how beaterOS agents should design and review systems work.
It applies to architecture docs, Rust code, future C/assembly boundaries,
sandboxing, model routing, payment rails, journals, memory, and release gates.
For detailed optimization-agent workflow, bottleneck taxonomy, toolchain
freshness rules, and accelerator review packets, also use
`docs/optimization-agent-playbook.md`.

## First Principles

The operating system exists to allocate scarce resources safely: CPU time,
memory, IO, storage, network, credentials, money, model calls, human attention,
and trust. Agent-first OS design adds one more rule: every meaningful side
effect must be attributable to an explicit authority grant and replayable from
evidence.

Design from these questions:

- What is the smallest authority needed for this action?
- What must be true before the action can execute?
- What changes after it executes?
- Which resource can be exhausted?
- Which state must survive crash, replay, audit, and redaction?
- Which path is hot, and which path can be slow, batched, or asynchronous?
- What measurement would prove this got faster, safer, or more predictable?

If an implementation cannot answer those questions, it is not ready to be
optimized.

## Performance Model

Every performance-sensitive change should name its budget:

- latency: p50, p95, p99, and timeout behavior
- throughput: steady-state and burst target
- memory: peak resident set, per-session overhead, allocation rate
- CPU: expected core use, lock contention, context switches
- IO: syscalls, fsyncs, network round trips, bytes copied
- model/tool cost: calls, tokens, wall-clock, retries, and spend
- accelerator cost when relevant: host-device copies, device-memory residency,
  queue delay, launch count, occupancy, partition contention, throttling, and
  fallback latency

Optimization order:

1. Remove work with a better protocol, contract, cache, batch, or algorithm.
2. Move work off the hot path with precomputation or asynchronous execution.
3. Reduce copies, allocations, serialization, syscalls, and lock handoffs.
4. Improve layout for cache locality and predictable branch behavior.
5. Specialize code only when measurement proves the general path is too slow.
6. Add hardware/platform-specific paths behind a portable contract.

Do not introduce clever low-level code without a measurable bottleneck, a simpler
fallback, and tests that protect correctness.

Before optimizing, classify the bottleneck:

- contract work: unnecessary action, receipt, model call, hash, retry, or scan
- algorithm: wrong complexity, missing index, avoidable recomputation, or
  unbounded fan-out
- data layout: poor locality, cold fields in hot structs, allocation churn, or
  pointer chasing
- copy/encoding: clone storms, JSON churn, string formatting, buffer growth, or
  host-device transfer
- syscall/IO: process spawn, fsync, descriptor churn, network round trip, or
  storage flush
- concurrency: lock contention, queue backlog, priority inversion, async leak,
  or retry amplification
- scheduler/platform: context switch, page fault, CPU affinity, timer, power, or
  kernel feature gap
- accelerator: launch overhead, model residency miss, memory pressure,
  partition contention, precision conversion, or batch mismatch
- provider/runtime: model cold start, SDK retry, rate limit, remote queue, or
  browser/cloud control-plane delay

## Language And Boundary Policy

Use the best language for the subsystem, boundary, and measurement target. When
the tradeoff is close, prefer Rust because it gives predictable native
performance with strong ownership and concurrency checks.

The goal is metal-grade performance, not performative low-level code. Start from
the contract, measure the hot path, and choose the lowest-risk boundary that can
meet the latency, throughput, memory, syscall, and security budget. Moving a
component closer to the metal is justified by evidence: a trace, benchmark,
profile, proof obligation, or platform boundary that the current layer cannot
satisfy.

Toolchain facts are temporal. For a performance-sensitive PR, record the date
and primary source for the compiler/runtime version if the change depends on
language or compiler behavior. `docs/source-matrix.md` keeps the
repo-maintained current-version snapshot for language and accelerator inputs.
Those entries are not beaterOS pins; they are a reminder that agents must
verify current versions before making current-version claims.
Operational replay packets live in
`docs/engineering/optimization-evidence-runbook.md` and the detailed workflow
lives in `docs/optimization-agent-playbook.md`; this file owns doctrine, not
per-PR evidence format.

Default choices:

- Rust for the kernel/control plane, policy engine, sandbox orchestration,
  journaling, receipts, tool gateway, model router, payment enforcement, CLIs,
  local daemons, and performance-sensitive services.
- C for stable ABI surfaces, kernel/driver/hypervisor/platform APIs, existing
  high-quality C libraries, or hot paths where measurement proves Rust is not
  meeting the requirement.
- C++ for vendor SDKs, browser/embedder integration, compiler/runtime extension,
  or existing libraries where replacement is riskier than isolation. Keep it out
  of the authority boundary unless ownership, exceptions, allocation, threading,
  and failure behavior are reviewed explicitly.
- Assembly for unavoidable hardware boundaries only.
- Zig for isolated freestanding experiments and cross-compilation probes, not
  TCB authority paths until toolchain stability and reviewer depth are proven.
- Swift for Apple-native UI or platform integration where it is the platform
  boundary, not for beaterOS policy or receipts.
- Go for non-TCB infrastructure daemons where static deployment and iteration
  matter more than lowest-latency ownership control.
- Python for small validation/audit/research scripts where startup latency,
  dependency control, and runtime authority are bounded.
- TypeScript for browser/dashboard surfaces when a web UI is the product
  boundary, with generated contracts rather than hand-maintained schema drift.
- SQL or purpose-built query languages for durable/query-heavy state, with
  migrations and conformance tests as contracts.

Ecosystem boundary rule:

- Tempo and browser-facing surfaces may use TypeScript for UI and browser
  integration, but authority, policy admission, receipt writing, trace
  verification, and scheduler-facing operations must terminate in native
  beaterOS contracts.
- Agent SDKs may be ergonomic in TypeScript or another host language, but they
  are clients of the Rust authority boundary, not substitutes for it.
- High-volume trace, screenshot/DOM metadata, journal, memory, sandbox, and
  tool-gateway paths should avoid avoidable JSON churn. Use generated schemas,
  stable binary encodings, mmap/shared-memory, ring buffers, or zero-copy
  handoff only when measurements show the simpler path is too slow.
- Platform-specific acceleration is allowed behind portable contracts. Linux
  paths may use `io_uring`, eBPF, XDP, cgroups, namespaces, seccomp, and KVM
  when available. macOS paths must keep a real implementation or explicit
  abstraction, not a silent no-op.
- GPU, TPU, LPU, NPU, media-engine, secure-enclave, and custom-silicon paths are
  accelerator backends behind beaterOS contracts. They must not bypass
  admission, data-class policy, receipts, cancellation, queue bounds, or spend
  limits.

Accelerator engineering rules:

- Model accelerator work as schedulable jobs with device class, model/artifact
  digest, runtime/compiler version, memory budget, precision/quantization,
  batch/streaming mode, tenant isolation, timeout, and fallback route.
- Count host-device copies, HBM/VRAM/SRAM residency, pinned memory, DMA, memory
  bandwidth, cache pressure, synchronization/fence cost, page migration, queue
  delay, kernel launch overhead, and thermal/power throttling as part of the hot
  path.
- Prefer keeping weights and hot embeddings resident when the authority and
  data-sensitivity boundary allows it. Eviction and cache reuse are policy
  decisions, not hidden SDK behavior.
- Treat discrete-device memory and Apple-style unified memory differently.
  Discrete accelerators pay explicit copy/DMA/pinning costs; unified-memory
  systems still pay bandwidth, cache, synchronization, page-migration, and
  shared-RSS costs. A copy-vs-map decision needs evidence.
- Accelerator queues need bounded depth, admission class, priority/fairness
  rules, maximum batch wait, cancellation-drain behavior, tenant isolation,
  overload behavior, and enqueue/dequeue/start/finish receipt evidence.
- Use hardware partitioning when available, such as GPU MIG or VM/pod-level
  accelerator slices. When unavailable, isolate with processes, sandboxes,
  microVMs, device ACLs, and conservative scheduling.
- Never hard-code one accelerator vendor as the OS contract. GPU, TPU, LPU,
  NPU, Apple Silicon, and future ASIC paths must implement the same admission,
  receipt, telemetry, and fallback shape.
- On Apple Silicon, name whether a path uses CPU SIMD, Metal GPU, Metal
  Performance Shaders, Core ML, ANE/Core AI-style framework routing, media
  engines, or the secure enclave. If placement, timing, or throttling is hidden
  by the framework, record that limitation and provide a fallback.

Use C only when at least one condition is true:

- a stable C ABI is the platform contract
- boot, firmware, kernel, driver, hypervisor, or sandbox interfaces require it
- a proven library or hardware interface is C-only
- measurement shows an isolated hot path cannot meet requirements in safe Rust
- cross-language embedding requires a C-compatible boundary

Use assembly only for unavoidable hardware work: bootstrapping, context switch
stubs, atomics, CPU feature probes, register access, system-call veneers, or
cryptographic/platform primitives where a vetted implementation requires it.

Rules for non-Rust surfaces:

- Keep the boundary small and documented.
- State why the chosen language is better than Rust for that slice, or state
  that Rust was chosen because the tradeoff was close.
- Expose a safe Rust wrapper with explicit invariants.
- Avoid sharing ownership across FFI. Prefer handles, borrowed slices with clear
  lifetimes, or copied POD structs.
- Make failure explicit; never encode authority or policy errors as unchecked
  integer status codes.
- Test boundary behavior on macOS and Linux where relevant.
- Fuzz or property-test parsers, binary formats, policy evaluation, and unsafe
  wrappers when practical.

## Data Layout And Memory

Fast systems make ownership and layout boring:

- Prefer append-only logs and immutable records for audit-critical state.
- Keep hot structs compact and stable; move cold strings, diagnostics, and large
  blobs out of hot records.
- Use arenas, slabs, pools, or interning only when allocation profiles justify
  them.
- Avoid accidental clones of manifests, receipts, spans, token buffers, and file
  contents.
- Stream large artifacts. Do not load whole traces, screenshots, archives, or
  model transcripts into memory unless bounded by policy.
- Design serialization as a contract. Schema evolution, version fields, digests,
  and canonical encodings matter.
- Prefer deterministic hashes and content-addressed references for evidence.

## Concurrency, Scheduling, And Backpressure

Agents can create work faster than systems can safely execute it. Every queue,
task group, model route, sandbox lane, browser lane, payment lane, and journal
writer must have:

- a bounded capacity
- admission control
- cancellation semantics
- retry limits
- timeout behavior
- overload telemetry
- a fail-closed policy for security-critical work

Avoid unbounded fan-out. Prefer structured concurrency, explicit cancellation,
and resource tokens. Do not let background work inherit ambient credentials,
environment variables, filesystem access, or payment authority.

## Security And Integrity

Security is part of the performance model because an insecure fast path is not a
valid fast path.

Core rules:

- Deny by default.
- Bind approval, simulation, policy decision, and receipt evidence to the exact
  action manifest digest.
- Keep capability grants attenuable, revocable, scoped, and auditable.
- Normalize paths and resources before policy evaluation.
- Never trust model output as authority.
- Treat memory as evidence with provenance, not as privileged truth.
- Separate simulation from execution.
- Record enough receipts to answer who authorized what, using which grant, over
  which resource, with what observed result.

Cryptography guidance:

- Do not invent cryptographic protocols.
- Use standard, audited primitives and libraries.
- Prefer SHA-256/SHA-384-family hashes for interoperability and audit proofs.
- Consider BLAKE3 only for documented high-throughput integrity use cases where
  interoperability is not the primary constraint.
- Use AEADs for encryption; keep key management, rotation, and crypto-shredding
  explicit.
- Keep crypto agility in schemas so post-quantum or stronger primitives can be
  introduced without rewriting the evidence model.
- Merkle trees are useful for batched audit proofs, redaction-preserving
  inclusion proofs, and log checkpoints, but they do not replace capability
  checks or sandboxing.

## Observability And Verification

A systems change is incomplete without a way to detect regressions.

Use the smallest relevant proof:

- unit tests for invariants and edge cases
- property tests for attenuation, parser, journal, and receipt behavior
- scenario tests for agent workflows and security boundaries
- benchmarks for hot-path changes
- traces for queueing, syscall, model/tool, sandbox, and journal latency
- flamegraphs or platform profilers for unclear CPU bottlenecks
- accelerator telemetry for device occupancy, memory residency, host-device
  copies, queue delay, launch count, throttling, and fallback behavior

On macOS, prefer Instruments, `sample`, `spindump`, `dtrace` where available,
and Rust-level benchmarks. On Linux future targets, add `perf`, eBPF, and
io_uring-specific checks behind platform gates.

For optimization-heavy work, also apply
[`docs/optimization-agent-playbook.md`](optimization-agent-playbook.md). That
document is the operational review gate for bottleneck classification,
language-version review, accelerator backends, benchmark evidence, and profiler
selection.

## macOS And Apple Silicon

macOS is a first-class development and runtime host for early beaterOS work.
Every PR should keep `cargo fmt`, `cargo test --workspace --locked`, and
`cargo clippy --workspace --all-targets --locked -- -D warnings` passing on
macOS.

Important constraints:

- Do not assume Linux-only `/proc`, cgroups, namespaces, seccomp, epoll, or
  io_uring in cross-platform crates.
- Use platform abstractions for filesystem, process, sandbox, networking, and
  eventing behavior.
- Apple Silicon is `aarch64`; consider alignment, atomics, SIMD, endian
  assumptions, and page-size differences.
- SIMD work must document feature detection, target features, compiler flags,
  alignment, scalar fallback, vector-width assumptions, precision/determinism
  drift, and benchmarks showing that auto-vectorization or intrinsics help.
- Bare-metal boot on modern Macs is not the first target. Prefer a macOS-hosted
  runtime, simulator, or hypervisor-backed path first, then define separate
  hardware bring-up targets.
- If a Linux mechanism is the best long-term kernel path, document the macOS
  equivalent, stub, or test strategy before merging.

## Review Checklist

Use this checklist on architecture and implementation PRs:

- The critical path and resource budgets are stated.
- The bottleneck class, baseline, target, and replay command are stated.
- Compiler/runtime versions are recorded when they are part of the claim.
- No unbounded queues, retries, memory growth, fan-out, or model/tool spend were
  introduced.
- Authority is explicit and least-privilege.
- Evidence binds to exact manifests and survives replay.
- Hot-path data avoids unnecessary allocation, clone, serialization, and syscall
  churn.
- C/assembly/unsafe code is justified, isolated, and tested.
- macOS support is preserved.
- Accelerator paths retain admission, scheduling, data-class policy, receipts,
  cancellation, telemetry, and portable fallback.
- The change adds or identifies the benchmark, trace, property test, scenario, or
  CI gate that would catch the most likely regression.
