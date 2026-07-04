# beaterOS

beaterOS is an agent-first operating-system layer: a local-first agent kernel
for explicit authority, deterministic policy, sandboxed execution, receipts,
memory provenance, eval gates, and auditable side effects. It is not trying to be
a bare-metal macOS replacement today. The immediate target is a close-to-the-metal
agent runtime that works on macOS, especially Apple Silicon, while keeping a path
open for lower-level kernel, hypervisor, and sandbox integrations.

Start with [AGENTS.md](AGENTS.md) for agent-facing repo context. The
close-to-the-metal systems engineering rules are in
[docs/sota-systems-engineering.md](docs/sota-systems-engineering.md).

## Status

This repo is in early implementation. The design source of truth is
[final.md](final.md); the current codebase has the first Rust contract crate,
language-neutral contract schemas, threat model, and systems-engineering
guidance. It is not yet a bootable OS or a usable end-user runtime.

## Navigation

| Area | Start here | Why it matters |
| --- | --- | --- |
| Product thesis | [final.md](final.md) | Full first-principles plan for an agent-first OS layer |
| Agent startup context | [AGENTS.md](AGENTS.md) | Repo map, non-negotiables, and common commands |
| Implementation sequence | [docs/implementation-backlog.md](docs/implementation-backlog.md) | PR-sized slices and no-self-merge review rules |
| Systems engineering | [docs/sota-systems-engineering.md](docs/sota-systems-engineering.md) | Hot-path, Rust/C/assembly, security, and macOS doctrine |
| Threat model | [docs/threat-model.md](docs/threat-model.md) | Assets, trust boundaries, attacks, mitigations, residual risk |
| Wire contracts | [spec/README.md](spec/README.md) | Language-neutral JSON Schema and conformance suite |
| Rust core | [crates/beater-os-core](crates/beater-os-core) | Agent sessions, grants, manifests, decisions, receipts, journals |
| Source audit | [docs/source-matrix.md](docs/source-matrix.md) | Citation verification and source-maintenance rules |

Important `final.md` sections:

| Section | Topic |
| --- | --- |
| [1](final.md#1-executive-thesis) | Executive thesis |
| [4](final.md#4-first-principles-of-operating-systems) | First principles of operating systems |
| [7](final.md#7-what-agents-should-have-in-an-os) | What agents should have in an OS |
| [8](final.md#8-big-design-choices) | Big design choices |
| [12](final.md#12-core-data-contracts) | Core data contracts |
| [13](final.md#13-security-model) | Security model |
| [14](final.md#14-simulation-and-evals) | Simulation and evals |
| [20](final.md#20-critical-open-questions) | Critical open questions |
| [21](final.md#21-non-goals) | Non-goals |
| [24](final.md#24-minimum-viable-beateros) | Minimum viable beaterOS |
| [27](final.md#27-source-matrix) | Source matrix |

## Non-Goals

The near-term project is not a Linux distribution, a macOS replacement, a crypto
network, a desktop shell, or a general chatbot UI. The first useful artifact is
an auditable agent kernel with narrow authority, reproducible policy decisions,
and macOS-compatible developer workflows.

## Development

beaterOS follows the neighboring Beater Rust workspace style:

```sh
cargo fmt --all -- --check
cargo test --workspace --locked
cargo clippy --workspace --all-targets --locked -- -D warnings
python3 spec/conformance/validate.py --quiet
```

## Contributing

Use GitHub issues for design gaps, citation problems, security concerns, and
implementation tasks. Every feature should land through a scoped PR, get
reviewed by an agent or person who did not author it, and be merged by a
non-author. Do not shorten or weaken [final.md](final.md) while implementing it.

## License

beaterOS is licensed under the [Apache License 2.0](LICENSE).
