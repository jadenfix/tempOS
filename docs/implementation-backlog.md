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
- Local checks should include `cargo fmt --check`, `cargo test --workspace`, and
  `cargo clippy --workspace --all-targets`.

## Slices

| Slice | Branch | PR Title | Scope | Depends On |
| --- | --- | --- | --- | --- |
| 1 | `codex/agent-kernel-contracts` | Bootstrap agent kernel contracts | Cargo workspace, core contracts, policy admission, hash-chained journal and receipts, PR template | none |
| 2 | `codex/session-runtime` | Wire beater-osd session lifecycle | session create/pause/resume/cancel, grant binding, journaled transitions | 1 |
| 3 | `codex/beaterosctl-contracts` | Add beaterosctl for sessions, grants, manifests, and journal inspect | CLI commands and golden output for contracts | 1 |
| 4 | `codex/sandbox-shell-lane` | Run scoped shell actions through sandbox lane | safe local execution lane, filesystem diff receipts, no inherited secrets | 1, 2, 3 |
| 5 | `codex/mvp-coding-workflow` | Prove MVP coding workflow end to end | granted repo read/edit/test workflow with trace and receipts | 4 |
| 6 | `codex/scenario-runner` | Implement scenario manifests and eval runner | deterministic fixtures, oracle ladder, trace-property checks | 5 |
| 7 | `codex/observability-export` | Expose traces, receipts, and audit export | OpenTelemetry-compatible spans and redaction-safe audit export | 2, 6 |
| 8 | `codex/tool-registry` | Add signed tool registry and action normalization | tool manifests, version pinning, risk metadata, quarantine | 1 |
| 9 | `codex/mcp-gateway` | Gate MCP tools through capabilities and receipts | MCP adapter, no token passthrough, grant checks, receipts | 8 |
| 10 | `codex/browser-service` | Add safe browser service primitives | isolated contexts, navigation/form/download/upload receipts | 6, 8 |
| 11 | `codex/memory-provenance` | Build accountable memory projection from journal | memory records, provenance query, expiry, redaction, rebuild | 6 |
| 12 | `codex/human-review` | Add human review queue and approval receipts | review model, reviewer auth, approval/denial journal events | 2, 7 |
| 13 | `codex/model-router` | Add policy-aware model routing metadata | provider, retention, cost, sensitivity, local/cloud constraints | 7 |
| 14 | `codex/payment-mandates` | Implement bounded payment mandates with fake rail | spend checks, fake rail, idempotency, payment receipts | 12 |
| 15 | `codex/release-gates` | Make eval gates mandatory for beaterOS releases | smoke/core/security/cost/latency gates, incident replay hook | 6, 9, 10, 11 |
| 16 | `codex/distribution-hardening` | Package local beaterOS runtime safely | installable local runtime, templates, signed release plan | 15 |
| 17 | `codex/high-assurance-track` | Document and prototype high-assurance security path | formal invariants, crypto agility, TEE/PQC/seL4/CHERI notes | 1, 6 |

## Parallelism

After slice 1, slices 2 and 3 can proceed in parallel if their APIs remain
compatible. After the MVP workflow, observability, memory, browser, model, and
human-review work can split across separate branches with disjoint write scopes.
