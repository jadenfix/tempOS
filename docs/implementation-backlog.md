# beaterOS Implementation Backlog

This backlog maps `final.md` into PR-sized implementation slices. It is a
coordination artifact, not a replacement for `final.md`.

## Review And Merge Rules

- Every feature lands through a branch and PR.
- Every PR must be reviewed by an agent or person who did not author it.
- No PR is merged by the agent or person who authored it.
- PRs should stay contract-focused and map to named sections of `final.md`.
- `final.md` must not be shortened or weakened as part of implementation.
- Branches and worktrees should be cleaned up after merge.
- Architecture and implementation PRs should apply
  `docs/sota-systems-engineering.md`, including explicit hot-path budgets,
  resource bounds, language-boundary justification, and macOS impact.
- Local checks should include `cargo fmt --check`, `cargo test --workspace`, and
  `cargo clippy --workspace --all-targets`.

## Slices

| Slice | Branch | PR Title | Scope | Depends On |
| --- | --- | --- | --- | --- |
| 1 | `codex/agent-kernel-contracts` | Bootstrap agent kernel contracts | Cargo workspace, core contracts, policy admission, hash-chained journal and receipts, PR template | none |
| 2 | `codex/sota-systems-agent-guidance` | Add SOTA systems engineering agent guidance | repo agent context, Claude/Cursor rules, Codex skill, performance/security/macOS doctrine | 1 |
| 3 | `codex/session-runtime` | Wire beater-osd session lifecycle | session create/pause/resume/cancel, grant binding, journaled transitions | 1, 2 |
| 4 | `codex/beaterosctl-contracts` | Add beaterosctl for sessions, grants, manifests, and journal inspect | CLI commands and golden output for contracts | 1, 2 |
| 5 | `codex/sandbox-shell-lane` | Run scoped shell actions through sandbox lane | safe local execution lane, filesystem diff receipts, no inherited secrets | 1, 2, 3, 4 |
| 6 | `codex/mvp-coding-workflow` | Prove MVP coding workflow end to end | granted repo read/edit/test workflow with trace and receipts | 5 |
| 7 | `codex/scenario-runner` | Implement scenario manifests and eval runner | deterministic fixtures, oracle ladder, trace-property checks | 6 |
| 8 | `codex/observability-export` | Expose traces, receipts, and audit export | OpenTelemetry-compatible spans and redaction-safe audit export | 3, 7 |
| 9 | `codex/tool-registry` | Add signed tool registry and action normalization | tool manifests, version pinning, risk metadata, quarantine | 1, 2 |
| 10 | `codex/mcp-gateway` | Gate MCP tools through capabilities and receipts | MCP adapter, no token passthrough, grant checks, receipts | 9 |
| 11 | `codex/browser-service` | Add safe browser service primitives | isolated contexts, navigation/form/download/upload receipts | 7, 9 |
| 12 | `codex/memory-provenance` | Build accountable memory projection from journal | memory records, provenance query, expiry, redaction, rebuild | 7 |
| 13 | `codex/human-review` | Add human review queue and approval receipts | review model, reviewer auth, approval/denial journal events | 3, 8 |
| 14 | `codex/model-router` | Add policy-aware model routing metadata | provider, retention, cost, sensitivity, local/cloud constraints | 8 |
| 15 | `codex/payment-mandates` | Implement bounded payment mandates with fake rail | spend checks, fake rail, idempotency, payment receipts | 13 |
| 16 | `codex/release-gates` | Make eval gates mandatory for beaterOS releases | smoke/core/security/cost/latency gates, incident replay hook | 7, 10, 11, 12 |
| 17 | `codex/distribution-hardening` | Package local beaterOS runtime safely | installable local runtime, templates, signed release plan | 16 |
| 18 | `codex/high-assurance-track` | Document and prototype high-assurance security path | formal invariants, crypto agility, TEE/PQC/seL4/CHERI notes | 1, 7 |
| A1 | `claude/multi-agent-pr-review-4cfv9t` | Add beater-os-audit independent verifier and trace viewer | offline independent journal/receipt re-verification, human-legible trace render, audit metrics, redaction-safe audit bundle, `beateros-audit` CLI | 1 |

## Cross-Agent Coordination Log

This section is the communication channel between agents working on this repo in
parallel. Append here; do not rewrite others' entries.

- **claude (branch `claude/multi-agent-pr-review-4cfv9t`)** is taking slice **A1**
  (`beater-os-audit`): an *offline, independent* audit surface (re-verify a
  journal snapshot, render a legible trace, score audit coverage, export a
  redaction-safe bundle). It depends only on slice 1's contracts and adds a new
  crate with a disjoint write scope, so it can proceed in parallel.
- **Boundary vs. slice 8 (`observability-export`)**: slice 8 owns *live*
  OpenTelemetry span emission wired into the session runtime; slice A1 owns
  *offline* post-hoc verification/rendering with no runtime dependency. If the
  owner of slice 8 sees overlap, please comment on the slice A1 PR (#27) and we
  will settle who takes which half before either lands.
- Slice A1 (PR #27) is submitted by its author as ready for review. The
  authoritative review verdict is recorded by the non-author reviewer in
  `docs/governance/coordination-ledger.md` (not self-attested here), and the
  final merge is performed by a non-author principal per the no-self-merge rule
  and the single-account constraint (GitHub blocks self-`APPROVE`; the author
  never reviews or merges their own PR).

## Parallelism

After slice 2, slices 3 and 4 can proceed in parallel if their APIs remain
compatible. After the MVP workflow, observability, memory, browser, model, and
human-review work can split across separate branches with disjoint write scopes.
