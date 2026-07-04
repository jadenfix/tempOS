# SOTA Systems Engineering Doctrine

This document defines how beaterOS agents should design and review systems work.
It applies to architecture docs, Rust code, future C/assembly boundaries,
sandboxing, model routing, payment rails, journals, memory, and release gates.

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

Optimization order:

1. Remove work with a better protocol, contract, cache, batch, or algorithm.
2. Move work off the hot path with precomputation or asynchronous execution.
3. Reduce copies, allocations, serialization, syscalls, and lock handoffs.
4. Improve layout for cache locality and predictable branch behavior.
5. Specialize code only when measurement proves the general path is too slow.
6. Add hardware/platform-specific paths behind a portable contract.

Do not introduce clever low-level code without a measurable bottleneck, a simpler
fallback, and tests that protect correctness.

## Language And Boundary Policy

Use the best language for the subsystem, boundary, and measurement target. When
the tradeoff is close, prefer Rust because it gives predictable native
performance with strong ownership and concurrency checks.

Default choices:

- Rust for the kernel/control plane, policy engine, sandbox orchestration,
  journaling, receipts, tool gateway, model router, payment enforcement, CLIs,
  local daemons, and performance-sensitive services.
- C for stable ABI surfaces, kernel/driver/hypervisor/platform APIs, existing
  high-quality C libraries, or hot paths where measurement proves Rust is not
  meeting the requirement.
- Assembly for unavoidable hardware boundaries only.
- Python for small validation/audit/research scripts where startup latency,
  dependency control, and runtime authority are bounded.
- TypeScript for browser/dashboard surfaces when a web UI is the product
  boundary, with generated contracts rather than hand-maintained schema drift.
- SQL or purpose-built query languages for durable/query-heavy state, with
  migrations and conformance tests as contracts.

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

On macOS, prefer Instruments, `sample`, `spindump`, `dtrace` where available,
and Rust-level benchmarks. On Linux future targets, add `perf`, eBPF, and
io_uring-specific checks behind platform gates.

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
- Bare-metal boot on modern Macs is not the first target. Prefer a macOS-hosted
  runtime, simulator, or hypervisor-backed path first, then define separate
  hardware bring-up targets.
- If a Linux mechanism is the best long-term kernel path, document the macOS
  equivalent, stub, or test strategy before merging.

## Review Checklist

Use this checklist on architecture and implementation PRs:

- The critical path and resource budgets are stated.
- No unbounded queues, retries, memory growth, fan-out, or model/tool spend were
  introduced.
- Authority is explicit and least-privilege.
- Evidence binds to exact manifests and survives replay.
- Hot-path data avoids unnecessary allocation, clone, serialization, and syscall
  churn.
- C/assembly/unsafe code is justified, isolated, and tested.
- macOS support is preserved.
- The change adds or identifies the benchmark, trace, property test, scenario, or
  CI gate that would catch the most likely regression.
