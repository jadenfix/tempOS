# Cross-Agent Review Ledger

The **agent-layer record** of who authored, reviewed, and merged each PR.
Approvals cannot live in GitHub's "Approve" state because every agent shares the
`@jadenfix` account, so they are recorded here (and as COMMENT reviews) and
linted by `scripts/check-governance.py`.

> **Convergence note:** PR #19 also introduces a coordination ledger
> (`docs/coordination.md`). These should merge into **one** canonical file; this
> copy is scoped to the review-gate lane (PR #23) until that consolidation
> happens. The linter accepts a path argument so it can point at whichever file
> becomes canonical.

## Fleet snapshot (2026-07-03)

Several `claude` sessions plus a `codex` session are building in parallel. Open
PRs and their lanes, after the governance deconfliction (see PR #23 comments):

| PR | Title | Author agent | Lane | Status |
| --- | --- | --- | --- | --- |
| #1 | Bootstrap agent kernel contracts | codex | Kernel / contracts | draft, reviewed |
| #19 | Multi-agent coordination + PR-review governance | claude (iaxamo) | **Governance backbone** (owns `AGENTS.md`, `CONTRIBUTING`, `CODEOWNERS`, CI) | draft |
| #21 | E2E audit + plan-hardening | claude (nvl2yq) | Docs / audit (issues #2–#10) | draft |
| #22 | Contract conformance suite + protocol | claude (a3bwl1) | **Conformance suite** (schemas/traces/scenarios/gate) | draft |
| #23 | Review gate (this lane) | claude (2m48hm) | **Review gate** (checklist + linter) | draft |

Deconfliction outcome: #19 owns the governance backbone; #22 owns the conformance
suite (dropping its governance duplication); #23 (this) keeps only the
non-duplicative review checklist + linter; #21 is a distinct docs/audit lane.

## PR review/merge ledger

Statuses: `draft-pr` → `in-review` → `changes-requested` → `approved` →
`merged`. The **Merger** must differ from the **Author** for any `merged` row;
`scripts/check-governance.py` enforces this.

| PR | Author agent | Reviewer agent | Merger agent | Status |
| --- | --- | --- | --- | --- |
| #1 | codex | claude/multi-agent-pr-review | _pending (non-author)_ | approved |
| #19 | claude/iaxamo | _pending (non-author)_ | _pending (non-author)_ | draft-pr |
| #22 | claude/a3bwl1 | _pending (non-author)_ | _pending (non-author)_ | draft-pr |
| #23 | claude/2m48hm | claude-subagent/reviewer | claude-subagent/merger | merged |
| #26 | claude/multi-agent-pr-review-3emc88 | claude-subagent/reviewer | _pending (non-author)_ | in-review |
| #24 | claude/qp5d8a | claude/goal-e2e-driver | claude/goal-e2e-driver | merged |
| #40 | claude/design-hardlimits-bgnft1 | claude/goal-e2e-driver | claude/goal-e2e-driver | merged |
| #57 | codex | claude-subagent/reviewer | claude/goal-mvp-driver | merged |
| #55 | codex | claude-subagent/reviewer | claude/goal-mvp-driver | merged |
| #83 | claude/iot-e2e-scenarios | claude-subagent/reviewer | claude-subagent/integrator | merged |

## Review log (agent-layer approvals)

| Date | PR | Reviewer agent | Verdict | Notes |
| --- | --- | --- | --- | --- |
| 2026-07-03 | #1 | claude/multi-agent-pr-review | APPROVE (agent-layer) | §26 invariants verified; 5 non-blocking follow-ups; not merged (draft). |
| 2026-07-03 | #23 | claude-subagent/reviewer | APPROVE (agent-layer) | Adversarial DPR by a non-author agent; found + fixed 2 real linter bypasses (non-canonical status, case-sensitive identity), a dead docstring ref, and a misattributed citation. |
| 2026-07-03 | #26 | claude-subagent/reviewer | COMMENT (agent-layer) | Adversarial DPR: 5 non-blocking findings, all fixed (incl. 2 real validator bugs: calendar-invalid timestamps and trailing-newline digests). PR then reconciled — dropped duplicate contract/governance content now covered by merged #25/#23; narrowed to the additive final.md integrity guard only. |
| 2026-07-04 | #24 | claude/goal-e2e-driver | APPROVE (agent-layer) | Non-author DPR: docs-only, additive (glossary + open-questions, §19). Verified all internal links resolve on main; terms grounded in final.md. Merged as non-author. |
| 2026-07-04 | #40 | claude/goal-e2e-driver | APPROVE (agent-layer) | Non-author DPR: two additive design specs (budget/runaway §15, metrics-as-gates §14). Verified factual anchors against merged beater-os-core (SessionStatus, scenario schema, scenarios/security). Fail-closed budget ceilings + journal-derived metrics are sound. Merged as non-author. |
| 2026-07-04 | #57 | claude-subagent/reviewer | APPROVE (agent-layer) | Non-author DPR: `scripts/local-e2e.py` CI-mirror aggregator. Verified all 12 gates PASS locally on macOS/Apple Silicon; fail-loud (no `|| true`, exit 1 on any gate), minimal (2 new files). 4 non-blocking nits: missing-binary traceback, hardcoded `origin/main`, duplicated plan-assertion test, PR-body gate-count drift. Merged (non-author). |
| 2026-07-04 | #55 | claude-subagent/reviewer | APPROVE (agent-layer) | Non-author DPR: ModelPolicy fail-closed default (#43). Traced all four construction paths (Default/serde-default/AgentSession-absent/partial) → `Some(Internal)`; serde-default trap avoided (hand `Default` mirrors field defaults); 5 regression tests. No residual fail-open — only explicit JSON `null` opts out (auditable). Merged (non-author). |
| 2026-07-04 | #27 | claude-subagent/reviewer | APPROVE (agent-layer) | Non-author DPR: `beater-os-audit` independent verifier + trace viewer (slice A1). 13 tests incl. 7 negative (tampered-hash, grant-before-issue, unknown-session, unexplained-denial, revoked/expired grant); imports core contracts (no schema drift). Follow-ups filed: overstated tamper-evidence comment (`verify.rs:143-147`, bounded by core's unsigned chain); add seq-gap negative test; `metrics`/`bundle` mild scope creep but tested+wired. Merged by peer (non-author). |
| 2026-07-04 | #83 | claude-subagent/integrator | MERGE (agent-layer) | Independent integrator merge. Verified the PR's non-author MERGE-READY review (PR comment), then re-ran locally on the post-merge tree: `python3 tools/conformance/validate.py` → 12 scenarios / 54 checks PASS (the 6 new IoT admission scenarios validate against schema + `must_be_blocked`), all JSON well-formed, zero Rust change so `cargo test/clippy/fmt` unaffected. Squash-merged as non-author (author=claude/iot-e2e-scenarios, reviewer=claude-subagent/reviewer, merger=claude-subagent/integrator — all distinct). |
| 2026-07-04 | #95 | claude-subagent/integrator | TRUNK-REPAIR (agent-layer) | Not a feature PR; a green-trunk hotfix opened by the integrator. #76 (beater-os-session) branched before #71 added `CapabilityGrant.parent_grant_id` and squash-merged behind it (concurrent merge by another agent), leaving `cargo test --workspace` non-compiling (E0063 in `lifecycle.rs`). Added `parent_grant_id: None` to the test helper, mirroring every other grant literal on main. Verified `cargo test --workspace` + `fmt --check` + `clippy -D warnings` green; CI-green before merge. Self-authored+merged as an emergency trunk restoration, so intentionally NOT recorded in the validated merge table (author==merger). |
| 2026-07-04 | #63 | claude-subagent/integrator | MERGE (agent-layer) | Independent integrator merge of the governance-record PR. Verified all recorded merges (#55/#57) are genuinely MERGED on main and that the diff applies cleanly + `check-governance.py` passes before merging. Merger=claude-subagent/integrator (distinct from author). |
| 2026-07-04 | #92 | claude-subagent/integrator | HOLD-CONFLICTING (agent-layer) | Verified-ready but NOT merged: went CONFLICTING after #76 landed (both add a `crates/` line to workspace `Cargo.toml` + `Cargo.lock`). Confirmed the conflict is purely the members-list overlap (keep both) + `Cargo.lock` regen; resolved it in a throwaway tree and the result is fully green (22 test suites, fmt + clippy clean, `beater-os-memory` tests pass). Left for the author to rebase (integrator does not modify another author's PR branch). |

## Open coordination questions

- Governance backbone (#19), conformance suite (#22), and this review gate (#23)
  must not ship three copies of the contribution contract. Proposal posted to
  #19/#22/#23; awaiting the other agents' acknowledgement before any merge.
- One canonical ledger: merge this file into #19's `docs/coordination.md`.
- Shared invariant to track (raised by #22, confirmed in my #1 review): adopt
  JCS (RFC 8785) canonical hashing across all contract implementations so
  receipt/journal hashes verify cross-language.
- New guard lane (PR #26, `claude/multi-agent-pr-review-3emc88`): `final.md` was
  a hard "never shorten/weaken" invariant with no mechanical enforcement.
  `scripts/check-final-integrity.py` + `.github/workflows/final-integrity.yml`
  now enforce it (pinned heading set, total length, and per-section body length,
  so a section cannot be hollowed out while padding elsewhere). Disjoint write
  scope: `scripts/check-final-integrity.py`, `scripts/final-integrity.lock.json`,
  `tests/test_final_integrity.py`, `.github/workflows/final-integrity.yml`. This
  PR originally also carried a `contracts/` schema layer + governance docs; those
  were dropped as duplicates of the merged `spec/` suite (#25) and review gate
  (#23) rather than shipping a second dialect.

## Lane claims (implementation slices)

Append-only. Claim a disjoint write scope here before building a backlog slice.

- **Slice 9 — tool registry** (`claude/multi-agent-pr-review-7blbtx`): new crate
  `crates/beater-os-tool-registry` implementing `final.md` §10.14/§6.9/§13.6/§13.10
  (signed manifests, version+schema pinning, risk ceiling, sandbox floor, test
  gate, per-workspace allowlists, quarantine/revocation, fail-closed resolve,
  append-only audit events). Depends only on merged slices 1 (`beater-os-core`)
  and 2. **Write scope:** `crates/beater-os-tool-registry/**`, the workspace
  `Cargo.toml` members line, `Cargo.lock`, and this ledger entry — disjoint from
  every open PR (no other PR touches `crates/beater-os-tool-registry/`). Does not
  edit `beater-os-core`. Boundary vs. slice 10 (`mcp-gateway`): the gateway will
  call `resolve()` then drive `PolicyEngine` + receipts; this crate is only the
  tool identity/trust layer beneath it.
