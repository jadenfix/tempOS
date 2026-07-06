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
- [ ] Migration-phase impact is explicit: this PR preserves runtime compatibility or proves why migration is required.
- [ ] If this PR claims metal-level progress, `require_migration_phase`/matrix assertions were updated and backed by a reproducible case.
- [ ] Target runtime layer is explicitly stated: Runtime-only / Runtime+Service / Metal-adjacent.
- [ ] This PR preserves/advances the mandatory layer map in `docs/repo-map.md`.
- [ ] Repetition duties completed: PR review checklist, coordination ledger entry point, and README/architecture links updated when the PR changes repo/process/docs boundaries.

## Repetition duties for infra/architecture PRs

- [ ] `scripts/run-beater-osd-runtime-smoke.py --json` result (or equivalent host-side runtime proof) is included in the PR description when touching daemon/contract code.
- [ ] `scripts/check-bare-metal-readiness.py --require-migration-phase runtime` is included when touching runtime-layer code.
- [ ] If touching optional metal lanes, include `--require-migration-phase metal-ready` and matrix evidence.
- [ ] One-sentence migration impact statement is added to `docs/architecture-runtime-to-metal-path.md` when applicable.

## Optimization packet

Complete this section for performance, language-boundary, unsafe/FFI, scheduler,
runtime, accelerator, or close-to-metal changes. For docs/process-only PRs, state
exactly `N/A` as the only content under this heading.
Use `docs/engineering/optimization-evidence-runbook.md` and
`docs/optimization-agent-playbook.md` as the canonical packet and review
workflow.

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
- [ ] `python3 scripts/check-optimization-docs.py` when this PR changes
      optimization doctrine, source-matrix rows, agent instructions, or review
      packet structure.

## Review routing

- [ ] Reviewed by an agent/person who did not author the PR.
- [ ] Merge performed by an agent/person who did not author the PR.

## Notes for reviewers

<!-- Risks, follow-ups, and any areas needing deeper review. -->
