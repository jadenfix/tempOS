## What does this PR do?

<!-- Describe the beaterOS feature slice and the section(s) of final.md it implements. -->

## Type of change

- [ ] Agent kernel contract
- [ ] Capability or policy enforcement
- [ ] Journal, receipt, or audit trail
- [ ] Sandbox, tool, browser, memory, eval, payment, or model service
- [ ] Performance, language boundary, compiler/runtime, accelerator, or close-to-metal
- [ ] Docs / process only
- [ ] Refactor / internal
- [ ] CI / tooling

## Contract checklist

- [ ] New or changed contracts are versioned, typed, and covered by tests.
- [ ] Side-effecting actions are represented by manifests and receipts.
- [ ] Capability checks happen outside model output.
- [ ] No ambient authority is introduced.
- [ ] `final.md` was not shortened or weakened.

## Optimization packet

Complete this section for performance, language-boundary, unsafe/FFI, scheduler,
runtime, accelerator, or close-to-metal changes. For docs/process-only PRs, state
`N/A` in the reviewer notes.

- [ ] Hot path and cold path are named.
- [ ] Bottleneck class is identified (contract, algorithm, layout, copy/encoding,
      syscall/IO, concurrency, scheduler/platform, accelerator, provider/runtime).
- [ ] Baseline, target budget, replay command, workload/fixture, and regression
      gate are included.
- [ ] Compiler/runtime/backend versions are recorded; Rust builds use the
      repo-pinned `rust-toolchain.toml` unless this PR explicitly changes it.
- [ ] Authority boundary, receipt/audit replay, macOS path, fallback, and rollback
      story are preserved.
- [ ] Source links and dates are included for claims about current language,
      compiler, accelerator, or OS behavior.

## Tests

- [ ] `cargo fmt --check`
- [ ] `cargo test --workspace`
- [ ] `cargo clippy --workspace --all-targets`

## Review routing

- [ ] Reviewed by an agent/person who did not author the PR.
- [ ] Merge performed by an agent/person who did not author the PR.

## Notes for reviewers

<!-- Risks, follow-ups, and any areas needing deeper review. -->
