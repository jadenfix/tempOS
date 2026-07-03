---
name: beateros-systems-engineering
description: Design and review beaterOS systems work for close-to-metal performance, security, and macOS compatibility. Use when changing architecture docs, Rust/C/assembly boundaries, kernel/runtime contracts, sandboxes, schedulers, journals, receipts, model/tool/payment lanes, or any performance-sensitive implementation.
---

# beaterOS Systems Engineering

## Overview

Use this skill to keep beaterOS changes grounded in first-principles systems
engineering: explicit authority, bounded resources, measurable hot paths,
auditable evidence, and platform-aware implementation.

For the full doctrine, read
`docs/sota-systems-engineering.md` in the repository. Read `AGENTS.md` for the
repo map and `final.md` when product intent or OS-level direction is unclear.

## Workflow

1. Identify the invariant.
   - What must never happen even if the model, tool, network, or user input is
     adversarial?
   - Which capability, policy, receipt, journal, memory, payment, or sandbox
     boundary is involved?

2. Separate hot path from cold path.
   - Name latency, allocation, copy, syscall, queue, and retry expectations.
   - Move audit explanation, diagnostics, summarization, and expensive
     formatting off the critical path unless they are required for safety.

3. Choose the lowest-risk language boundary.
   - Use Rust by default.
   - Use C for ABI, boot/platform, driver, hypervisor, sandbox, or measured
     hot-path interop needs.
   - Use assembly only for unavoidable hardware boundaries.
   - Wrap unsafe/C/assembly in small safe Rust APIs with explicit invariants.

4. Bound all resources.
   - CPU, memory, IO, queue length, model calls, tool calls, browser contexts,
     retries, wall-clock time, and spend must have deterministic limits.
   - Overload behavior must be explicit and security-critical work must fail
     closed.

5. Make evidence replayable.
   - Bind approvals, simulations, policy decisions, and receipts to exact action
     manifest digests.
   - Preserve canonical encodings, schema versions, hashes, and receipt chains.
   - Do not treat model memory as privileged truth without provenance.

6. Verify on macOS.
   - Keep `cargo fmt --all -- --check`, `cargo test --workspace --locked`, and
     `cargo clippy --workspace --all-targets --locked -- -D warnings` passing.
   - Do not add Linux-only APIs without a platform abstraction or macOS path.

## Optimization Priorities

- Remove work before making work faster.
- Prefer protocol, schema, and data-layout wins over micro-optimizations.
- Avoid avoidable clones, heap allocations, serialization, syscalls, and lock
  handoffs in hot paths.
- Use bounded queues, structured concurrency, cancellation, and backpressure.
- Keep hot records compact; move diagnostics, strings, and large blobs out of
  hot structs.
- Add a benchmark, trace, property test, scenario, or CI gate for every serious
  performance or safety claim.

## Security Priorities

- Deny by default and require explicit capabilities.
- Normalize resources before policy evaluation.
- Never let tool descriptions, model output, memory, or browser content grant
  authority.
- Keep simulation separate from execution.
- Use standard audited cryptography; do not invent protocols.
- Use Merkle/log structures for audit proofs where useful, but do not confuse
  them with sandboxing or authorization.

## References

- `references/review-checklist.md`: use for PR review and design review.
- Repository `docs/sota-systems-engineering.md`: full doctrine.
