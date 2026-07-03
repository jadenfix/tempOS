# beaterOS

Agent-first operating system research and planning.

Start with [AGENTS.md](AGENTS.md) for agent-facing repo context. The
close-to-the-metal systems engineering rules are in
[docs/sota-systems-engineering.md](docs/sota-systems-engineering.md).

## Development

beaterOS follows the neighboring Beater Rust workspace style:

```sh
cargo fmt --check
cargo test --workspace
cargo clippy --workspace --all-targets
```

The product plan is in [final.md](final.md). The PR sequencing and review rules
are in [docs/implementation-backlog.md](docs/implementation-backlog.md).
