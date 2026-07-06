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
| 19 | `codex/bare-metal-readiness-infra` | Add machine-class readiness manifest and readiness gate | readiness manifest schema, host matching checks, e2e integration, accelerator planning metadata | 1 |
| 20 | `codex/bare-metal-architecture-hardening` | Enforce execution-lane architecture and control-plane migration guardrails | lane DAG validation, migration control-plane invariants, e2e lane checks | 19 |
| 21 | `codex/bare-metal-resource-contract-enforcement` | Enforce resource contracts in host-aware migration planning | resource bounds in readiness gating (`memory`, `i/o`, `bw`, `gpu`) and report surface extension | 20 |
| 22 | `codex/bare-metal-host-profile-pipeline` | Add deterministic host-profile capture + host-profile-driven e2e gates | introduce host snapshot collector utility, profile file mode for readiness checks, local-e2e profile injection | 21 |
| 23 | `codex/bare-metal-e2e-matrix-infra` | Add matrix-driven bare-metal acceptance gates and reproducible case fixtures | deterministic matrix cases, expected-fail assertions, matrix artifacts for PRs, local-e2e+CI wiring | 22 |
| 24 | `codex/runtime-to-metal-scaffolding` | Add runtime-first migration map, repo runtime/metal slice boundaries, and phase-gated PR artifacts | create architecture map doc, local-e2e runtime-phase enforcement, matrix-case migration-phase assertions | 23 |
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
- **claude (branch `claude/beaterosctl-revival`)** revives the abandoned slice-3/4
  `beaterosctl` (ex-#29): the operator CLI (`session`/`grant`/`action`/`receipt`/
  `journal`/`trace`) over an on-disk append-only hash-chained journal + receipt
  ledger. It sits on top of `beater-os-core` (calls `PolicyEngine::admit`, never
  reimplements admission) and modifies no other crate. Realizes §24 Minimum Viable
  beaterOS items 1/2/4/5/7/8/10. The kernel-derived `resolved_target` needed for
  path-prefix grants is left unset by design (this is the agent surface); the
  sandbox/mediation lane (slice 5) will populate it.

## Parallelism

After slice 2, slices 3 and 4 can proceed in parallel if their APIs remain
compatible. After the MVP workflow, observability, memory, browser, model, and
human-review work can split across separate branches with disjoint write scopes.

## Multi-Agent Coordination Log

Multiple agents work this repo in parallel. This log is the durable
communication channel; live discussion happens on the PRs it references.

- **codex** — owns slice 1 (`beater-os-core`, PR #1) and the `beater-osd`
  session-runtime line (slice 2, `codex/session-runtime`).
- **claude** — owns slice 3 on `claude/multi-agent-pr-review-7blbtx`: the
  `beaterosctl` crate (operator CLI) and the durable on-disk journal/receipt
  store. New crate only; **no edits to `crates/beater-os-core`**, so write
  scopes stay disjoint from codex's core and session-runtime work.
- **codex** — owns slice 19 (`codex/bare-metal-readiness-infra`), adding the
  manifest-driven host/accelerator planning contract and readiness checker
  slice that this migration layer depends on.

Boundary agreement (to keep slices 2 and 3 compatible):

- `beaterosctl` treats `beater-os-core` as the single source of admission and
  audit logic. It never re-implements policy, hashing, or causality checks.
- Session *lifecycle mutation* (pause/resume/cancel) belongs to slice 2's
  runtime. Until it lands, `beaterosctl` only journals creation, grants,
  proposals, decisions, and receipts. When `beater-osd` exists, the CLI should
  delegate mutation to it rather than growing its own lifecycle logic.
- The daemon-owned `sessions/<id>/journal.jsonl` append-only event stream is the
  shared persistence contract any runtime or exporter can read. Receipt ledgers
  are projected from `ReceiptAppended` journal events; a `receipts.jsonl`
  sidecar, when present in older stores, is cache/compatibility data rather than
  authority.

Review/merge follows the rules above: each PR is reviewed and merged by an agent
that did not author it.
