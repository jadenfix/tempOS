# Optimization Evidence Runbook

Status: required guidance for performance, language-boundary, accelerator, and
close-to-metal PRs.

This runbook turns the SOTA systems-engineering doctrine into a repeatable
evidence packet. It is intentionally small: the goal is to make optimization
claims replayable without creating a benchmark bureaucracy.

## First-Principles Start

An optimization PR starts by naming what scarce resource is being protected:
latency, throughput, memory, IO, storage, accelerator residency, power, model
spend, payment spend, human attention, or trust.

Before editing code, write down:

- invariant: what must never break
- hot path: the smallest execution path whose budget matters
- cold path: diagnostics, formatting, summarization, reporting, or cleanup that
  can be moved out of line
- bottleneck class: contract, algorithm, layout, copy/encoding, syscall/IO,
  concurrency, scheduler/platform, accelerator, or provider/runtime
- budget: p95/p99 latency, throughput, allocation, copy, syscall, queue, device
  memory, or spend target
- authority boundary: which capability, policy, journal, receipt, memory, model,
  tool, or payment boundary proves the action was allowed

If the bottleneck cannot be named, the next action is instrumentation, not a
rewrite.

## Replay Packet

Each serious optimization PR includes the smallest packet that lets a reviewer
replay the claim:

| Field | Required content |
| --- | --- |
| Workload | Command, scenario, trace, fixture, benchmark input, or host profile |
| Baseline | Current p50/p95/p99, throughput, memory, syscalls, copies, queue depth, model/tool calls, device occupancy, or provider latency |
| Budget | Target threshold and timeout behavior |
| Profile | Instruments, `sample`, `spindump`, allocation count, syscall count, trace spans, Rust benchmark output, Xcode GPU tools, Nsight, TPU/GPU provider metrics, or equivalent |
| Change | Why the diff attacks the measured bottleneck |
| Safety | Preserved capability, policy, receipt, journal, memory, model, payment, and rollback story |
| Portability | macOS path, Linux path where applicable, feature gate, and fallback |
| Toolchain | `rustc -vV`, `cargo -vV`, compiler/runtime/backend versions, target triple, feature flags, and primary-source version links when current-version claims matter |
| Regression | Unit, property, scenario, benchmark, matrix, or CI gate that catches the regression |

The packet can be short for a narrow PR, but every field should be explicit.
Docs-only PRs can mark the optimization packet `N/A`.

## Language Boundary Review

The default implementation language for beaterOS authority and hot control-plane
work is Rust. A different language is acceptable only when the boundary is the
reason:

| Boundary | Acceptable reason | Required proof |
| --- | --- | --- |
| Rust | Default for authority, policy, journal, receipt, daemon, CLI, scheduler-facing, memory, and service work | Repo-pinned toolchain, tests, profile when performance is claimed |
| C | Stable ABI, kernel/platform API, driver, hypervisor, sandbox primitive, existing vetted C library, or measured hot-path interop | Safe Rust wrapper, ownership rules, error mapping, fuzz/property test where practical |
| C++ | Vendor SDK, browser/embedder, compiler/runtime extension, or existing library where isolation is lower risk than replacement | Exception/allocation/threading policy, isolated ownership, explicit failure behavior |
| Assembly | Boot, context switch, register, syscall veneer, atomics, CPU feature probe, or vetted primitive | Minimal surface, platform guard, scalar/native fallback, code-owner review |
| Swift | Apple-native UI or platform integration | Rust authority boundary remains authoritative |
| TypeScript | Tempo/browser UI, dashboards, agent ergonomics | Generated contracts and native policy/journal/receipt termination |
| Python | Bounded validation, audit, research, and reproducibility scripts | No ambient authority, bounded runtime, deterministic fixtures |
| CUDA/Metal/XLA/shaders | Accelerator kernel or backend behind beaterOS contracts | Admission, queue bounds, memory budget, cancellation, telemetry, fallback, receipts |

If the tradeoff is close, choose Rust.

## Accelerator Packet

GPU, TPU, LPU, NPU, Apple Silicon, media-engine, enclave, and future ASIC paths
are OS resources. They must be scheduled and audited like CPU, IO, memory,
network, model spend, and payments.

An accelerator PR records:

- device class and backend identity: CUDA, Metal, XLA/TPU, LPU provider, NPU,
  Apple GPU/ANE-style framework, media engine, enclave, or future registered
  class
- model or artifact digest, runtime/compiler version, driver/framework version,
  target/device capability, precision, quantization, deterministic seed where
  meaningful, and feature flags
- memory split: host RAM, device memory, pinned memory, unified memory,
  HBM/VRAM/SRAM, cache residency, spill path, and data-sensitivity constraints
- host-device copy/map/sync count and bytes, launch count, queue delay,
  execution time, occupancy/provider metric where available, and throttling
- isolation: MIG, VM/pod slice, process, sandbox, microVM, device ACL, or
  conservative single-tenant scheduling
- cancellation and timeout behavior that releases scarce device work
- fallback route when the accelerator is missing, overloaded, revoked, noisy, or
  too expensive
- receipts proving placement, backend version, partition/slice identity where
  available, input/output digests, observed side effects, and replay evidence

Do not make a vendor SDK the beaterOS contract. Vendor APIs implement the
contract; they do not define authority.

## Reviewer Questions

Reviewers should block or request changes when:

- the PR says "optimized" without a baseline and replay command
- the hot path includes avoidable JSON churn, clones, string formatting,
  syscalls, lock handoffs, model/tool calls, or host-device copies
- the change adds FFI, unsafe code, assembly, or accelerator dependencies before
  simpler contract, algorithm, layout, batching, caching, or indexing fixes were
  considered
- the macOS path is absent, a Linux-only primitive is hidden behind a no-op, or
  fallback behavior is undefined
- an accelerator path lacks queue bounds, memory/residency limits,
  cancellation, telemetry, or receipts
- current language/compiler/backend claims lack a primary source and verification
  date

## Local Gate

Run the doc-health gate before opening a performance-sensitive PR:

```sh
python3 scripts/check-optimization-docs.py
```

The full local e2e wrapper runs this gate as part of:

```sh
TMPDIR=/private/tmp python3 scripts/local-e2e.py
```
