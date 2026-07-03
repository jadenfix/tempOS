# beaterOS Systems Review Checklist

Use this checklist for design reviews, code reviews, and architecture changes.

## Critical Path

- Name the hot path and cold path.
- State p95/p99 or throughput expectations when relevant.
- Count likely allocations, copies, serializations, syscalls, locks, and network
  round trips.
- Confirm expensive explanation, summarization, formatting, and diagnostics are
  outside the hot path unless required for safety.

## Resource Bounds

- CPU, memory, IO, queue depth, retries, wall-clock time, model calls, tool
  calls, browser contexts, and payment spend are bounded.
- Cancellation and timeout behavior are explicit.
- Overload fails closed for security-critical work.
- Background work cannot inherit ambient authority.

## Authority And Evidence

- Capability grants are least-privilege and attenuable.
- Resources are normalized before authorization.
- Approvals, simulations, policy decisions, receipts, and journal entries bind to
  the exact manifest digest.
- Memory records include provenance and cannot grant authority by themselves.
- Payment and external side effects require specific mandates and receipts.

## Language Boundary

- Rust is used by default.
- C usage is justified by ABI, platform, driver, hypervisor, sandbox, or measured
  hot-path constraints.
- Assembly usage is justified by unavoidable hardware interaction.
- Unsafe/C/assembly boundaries are small, documented, and wrapped in safe Rust.
- Fuzz, property, or boundary tests cover malformed inputs and failure paths when
  practical.

## macOS

- macOS and Apple Silicon build/test paths still work.
- Linux-only mechanisms are abstracted or documented as future-target work.
- Filesystem, process, eventing, sandbox, and profiler assumptions are
  platform-aware.

## Verification

- The PR includes the smallest useful proof: unit test, property test, scenario,
  benchmark, trace, or CI gate.
- Performance claims include a measurement plan or benchmark.
- Security claims include a negative test or threat-model link.
