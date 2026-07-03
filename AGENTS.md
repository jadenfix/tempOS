# beaterOS Agent Context

Use this file as startup context for Codex, Claude Code, Cursor, Copilot, and
other coding agents working in this repository.

## What beaterOS Is

beaterOS is an agent-first operating-system research and implementation repo.
The source-of-truth product plan is [final.md](final.md). Implementation must
turn that plan into reviewed, measurable, macOS-compatible slices without
shortening or weakening the plan.

The first implementation layer is a Rust workspace with kernel-facing contracts:
agent sessions, capability grants, action manifests, policy decisions, receipts,
and append-only journals. Future runtime work must preserve those contracts as
authority and audit boundaries, not merely as serialization types.

## Repo Shape

- `Cargo.toml` is the Rust workspace.
- `crates/beater-os-core` contains core contracts, policy admission, hashing,
  journal verification, and receipt-chain logic.
- `docs/implementation-backlog.md` maps `final.md` into PR-sized slices and
  review rules.
- `docs/sota-systems-engineering.md` is the performance, language, security, and
  macOS engineering doctrine for this project.
- `.codex/skills/beateros-systems-engineering/SKILL.md` packages that doctrine
  as a reusable Codex skill.
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
- Prefer Rust for most implementation. Use C only for stable ABI, boot/platform,
  driver, hypervisor, or measured hot-path interop needs. Use assembly only at
  the hardware boundary. Isolate and review all unsafe code.

## SOTA Systems Engineering Checklist

Before designing or reviewing a substantial change, read
[docs/sota-systems-engineering.md](docs/sota-systems-engineering.md). At minimum,
be able to answer:

- What is the critical path, and what is explicitly off the critical path?
- What are the data ownership, lifetime, and copy rules?
- Which queue, cache, journal, network, filesystem, or model call can back up?
- What is bounded by construction: memory, CPU, IO, tool calls, model spend,
  payment spend, retries, and wall-clock time?
- Which authority boundary does this touch, and what evidence proves it?
- What benchmark, trace, property test, or scenario would catch a regression?
- Why is the chosen language boundary Rust, C, or assembly?

## Common Commands

```sh
cargo fmt --all -- --check
cargo test --workspace --locked
cargo clippy --workspace --all-targets --locked -- -D warnings
git diff --check
```
