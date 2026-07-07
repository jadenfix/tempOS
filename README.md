# tempOS / beaterOS

tempOS is the long-horizon, agent-first operating-system program currently
implemented in this repository under the beaterOS crate and binary names. The
end state is not an app shell or agent framework. It is an OS stack that can
touch metal where agents need native scheduler, memory, IO, device, isolation,
authority, audit, accelerator, payment, and recovery boundaries. The first
target is the hosted Rust agent kernel that proves those contracts on macOS,
Linux, containers, browsers, and cloud VMs before any low-level component moves
into a Linux add-on, kernel, hypervisor, library OS, microVM, or
hardware-backed appliance.

The project has three explicit lanes:

- **Compatibility lane:** a local-first Rust agent kernel for explicit
  authority, deterministic policy, sandboxed execution, receipts, memory
  provenance, eval gates, and auditable side effects on existing operating
  systems.
- **Linux add-on lane:** optional Linux-native scheduler, IO, sandbox,
  observability, microVM, and accelerator experiments using mechanisms such as
  `sched_ext`, cgroups, namespaces, seccomp, eBPF/XDP, `io_uring`, DPDK, SPDK,
  KVM, and Rust-for-Linux when traces prove they help.
- **Metal lane:** a measured, multi-year path toward first-principles OS
  components for agent workloads: policy-aware scheduling, bounded memory and
  context tiers, zero-copy evidence paths, high-assurance sandboxing,
  accelerator fabric, crash recovery, and hardware-backed authority.

Start with [AGENTS.md](AGENTS.md) for agent-facing repo context. The
close-to-the-metal systems engineering rules are in
[docs/sota-systems-engineering.md](docs/sota-systems-engineering.md).

## Status

This repo is in early implementation. The design source of truth is
[final.md](final.md); the current codebase has the first Rust contract crate,
language-neutral contract schemas, threat model, and systems-engineering
guidance. It is not yet a bootable OS or a usable end-user runtime, but the repo
is intentionally setting the authority, performance, and ecosystem contracts
that a real metal-touching agent OS would need.

## Navigation

| Area | Start here | Why it matters |
| --- | --- | --- |
| Product thesis | [final.md](final.md) | Full first-principles plan for hosted and metal-touching beaterOS |
| Agent startup context | [AGENTS.md](AGENTS.md) | Repo map, non-negotiables, and common commands |
| Repository map | [docs/repo-map.md](docs/repo-map.md) | Runtime ownership boundaries and migration frontier |
| Review skill | [beateros-pr-review SKILL](.codex/skills/beateros-pr-review/SKILL.md) | Non-author review and repetitive infra/docs duty flow |
| Systems skill | [beateros-systems-engineering SKILL](.codex/skills/beateros-systems-engineering/SKILL.md) | Runtime and systems engineering doctrine |
| Implementation sequence | [docs/implementation-backlog.md](docs/implementation-backlog.md) | PR-sized slices and no-self-merge review rules |
| Runtime-to-metal architecture | [docs/architecture-runtime-to-metal-path.md](docs/architecture-runtime-to-metal-path.md) | Runtime-first migration map, layer boundaries, and migration-gate expectations |
| Metal OS blueprint | [docs/engineering/metal-os-blueprint.md](docs/engineering/metal-os-blueprint.md) | First-principles 2026 OS shape, Linux add-on split, accelerator fabric, and optimization evidence |
| Systems engineering | [docs/sota-systems-engineering.md](docs/sota-systems-engineering.md) | Hot-path, Rust/C/assembly, security, and macOS doctrine |
| Optimization infrastructure | [docs/optimization-agent-playbook.md](docs/optimization-agent-playbook.md) | Bottleneck taxonomy, benchmarks, language baselines, and accelerator review gates |
| Optimization evidence | [docs/engineering/optimization-evidence-runbook.md](docs/engineering/optimization-evidence-runbook.md) | Replay packet, language-boundary review, and accelerator evidence requirements |
| Threat model | [docs/threat-model.md](docs/threat-model.md) | Assets, trust boundaries, attacks, mitigations, residual risk |
| Wire contracts | [spec/README.md](spec/README.md) | Language-neutral JSON Schema and conformance suite |
| Rust core | [crates/beater-os-core](crates/beater-os-core) | Agent sessions, grants, manifests, decisions, receipts, journals |
| Tool gateway | [crates/beater-os-tool-gateway](crates/beater-os-tool-gateway) | Registered-tool resolution, daemon admission, sandbox execution, and receipts |
| Source audit | [docs/source-matrix.md](docs/source-matrix.md) | Citation verification and source-maintenance rules |

Important `final.md` sections:

| Section | Topic |
| --- | --- |
| [1](final.md#1-executive-thesis) | Executive thesis |
| [4](final.md#4-first-principles-of-operating-systems) | First principles of operating systems |
| [7](final.md#7-what-agents-should-have-in-an-os) | What agents should have in an OS |
| [8](final.md#8-big-design-choices) | Big design choices |
| [8.1.1](final.md#811-what-an-operating-system-built-in-2026-should-look-like) | What an OS built in 2026 should look like |
| [8.1.2](final.md#812-ecosystem-runtime-contract) | Ecosystem runtime contract for Tempo, beater.js, beatbox, and memory |
| [12](final.md#12-core-data-contracts) | Core data contracts |
| [13](final.md#13-security-model) | Security model |
| [14](final.md#14-simulation-and-evals) | Simulation and evals |
| [20](final.md#20-critical-open-questions) | Critical open questions |
| [21](final.md#21-non-goals) | Non-goals |
| [24](final.md#24-minimum-viable-beateros) | Minimum viable beaterOS |
| [27](final.md#27-source-matrix) | Source matrix |

## First-Principles Direction

tempOS starts from scarce resources and trust boundaries, not from app features.
Every serious subsystem should state its hot path, allocation budget, copy
budget, syscall budget, queue bounds, p95/p99 target, authority boundary, and
regression test before it claims to be optimized.
Agents doing performance-sensitive work should use
[docs/optimization-agent-playbook.md](docs/optimization-agent-playbook.md) for
toolchain freshness checks, bottleneck classification, benchmark packets,
language-boundary review, and accelerator evidence.

The default implementation language is Rust because the authority path needs
native performance, explicit ownership, strong concurrency checks, and a small
unsafe surface. C is for stable ABI, driver, hypervisor, browser/embedder
interop, existing high-quality C libraries, or a measured hot path where safe
Rust cannot meet the requirement. C++ is for isolated vendor SDK and embedder
surfaces when replacement is riskier than containment. Assembly is for
unavoidable hardware entry points. TypeScript, Swift, Go, and Python are useful
at product, platform, and tooling seams, but not as the policy, journal,
receipt, scheduler, payment, or memory-provenance authority boundary.

Accelerators are first-class OS resources. GPU, TPU, LPU, NPU, Apple
Silicon-style local accelerators, media engines, enclaves, and future agent ASICs
must sit behind the same admission, scheduling, memory, telemetry, and receipt
contracts as CPU and IO. The project should optimize model residency,
host-device copies, batching, accelerator partitioning, and fallback routing
without making any one vendor SDK the operating-system boundary.

Tempo and the rest of the ecosystem should run on beaterOS contracts: browser
actions, sandboxed tools, model calls, memory projections, and receipts all flow
through native policy, journal, and audit services. The UI can stay high-level;
the OS boundary stays explicit, typed, measured, and replayable.

The Linux add-on path and the true metal path are deliberately separate. Linux
experiments can use the best current kernel primitives to learn quickly, but a
Linux-specific API never becomes the portable OS contract unless macOS,
fallback, and replay behavior are explicit. True metal work begins only after
hosted traces prove the required boundary cannot be expressed cleanly above an
existing kernel.

## Non-Goals

The near-term project is not a broad hardware driver stack, a macOS replacement,
a crypto network, a polished desktop shell, or a general chatbot UI. A
metal-touching beaterOS is in scope only when hosted traces and benchmarks prove
which low-level OS boundaries need to exist.

## Development

beaterOS follows the neighboring Beater Rust workspace style:

```sh
cargo fmt --all -- --check
cargo test --workspace --locked
cargo clippy --workspace --all-targets --locked -- -D warnings
python3 scripts/check-optimization-docs.py
python3 spec/conformance/validate.py --quiet
```

## Contributing

Use GitHub issues for design gaps, citation problems, security concerns, and
implementation tasks. Every feature should land through a scoped PR, get
reviewed by an agent or person who did not author it, and be merged by a
non-author. Do not shorten or weaken [final.md](final.md) while implementing it.

Performance-sensitive PRs must include an optimization packet: workload, replay
command, bottleneck class, baseline, target budget, profile or trace artifact,
compiler/runtime/backend versions when relevant, authority boundary, macOS path,
fallback, regression gate, and independent reviewer sign-off for both
performance and authority claims.

## License

beaterOS is licensed under the [Apache License 2.0](LICENSE).

## Ecosystem

beaterOS is part of the [ecosystem](https://github.com/jadenfix/ecosystem) — a family of Rust-first, local-first agent-infrastructure projects. It is fully standalone: the kernel contracts, policy engine, and conformance suite are usable by any agent runtime. Within the family it is the governance spine, with designed-for connections (each lands only with a real consumer) for:

- policy and authority over agents running in [beater.js](https://github.com/jadenfix/beater.js) and browsing via [tempo](https://github.com/jadenfix/tempo)
- sandboxed side effects through [beatbox](https://github.com/jadenfix/beatbox) and memory provenance through [beater-memory](https://github.com/jadenfix/beater-memory)
- emitting receipts and audit journals into [beater](https://github.com/jadenfix/beater) for observation and eval gating, with optional on-chain attestation anchoring on [aether](https://github.com/jadenfix/aether) at the frontier
