# beaterOS Claude Code Rules

Start with [AGENTS.md](AGENTS.md). It is the compact repo map and cross-agent
context. The full plan is [final.md](final.md), and the systems-engineering
doctrine is [docs/sota-systems-engineering.md](docs/sota-systems-engineering.md).

## Contract Rule

`final.md` is the source-of-truth plan. Do not shorten it, dilute it, or replace
it with implementation notes. Add focused docs, tests, contracts, and code around
it, then keep changes PR-scoped and reviewed by a non-author.

## SOTA Systems Rule

beaterOS should be designed like close-to-metal systems software:

- Start from invariants, budgets, and bottlenecks, not framework preference.
- Prefer simple data layouts, bounded queues, explicit ownership, and measurable
  hot paths.
- Keep the policy/audit path deterministic and replayable.
- Never trade away capability safety, receipt integrity, or sandbox isolation for
  cosmetic speed.
- Use Rust by default. Use C when ABI, platform, driver, hypervisor, or measured
  hot-path constraints require it. Use assembly only for boot, atomics, context
  switching, register access, or similarly unavoidable hardware boundaries.
- Any unsafe, C, or assembly surface must be tiny, documented, fuzzable or
  property-tested where practical, and wrapped in safe Rust APIs.
- Keep macOS and Apple Silicon as first-class development targets. Linux-specific
  mechanisms need an abstraction, a macOS fallback, or an explicit future-target
  label.

## Before Coding

For non-trivial changes, state or encode:

- critical path and non-critical path
- allocation/copy/syscall expectations
- queue and retry bounds
- failure mode under overload
- security boundary and required evidence
- macOS impact
- local verification command
