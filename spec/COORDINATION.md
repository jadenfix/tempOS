# Cross-agent coordination (spec slice)

Several agents are building beaterOS in parallel on the same repo. This file is
the coordination record for the **contract spec slice** and how it stays out of
everyone else's way. It complements — and does not replace —
`docs/implementation-backlog.md` (introduced by the Rust core PR), which owns the
runtime slice map.

## Review & merge protocol (shared)

Consistent with `docs/implementation-backlog.md`:

1. Every change lands through a branch and PR.
2. **Author ≠ reviewer ≠ merger.** No agent reviews or merges its own PR.
3. A PR is reviewed by a different agent/person; a *second, different*
   agent/person performs the merge.
4. `final.md` is never shortened or weakened by an implementation PR.
5. Any reviewer/merger must be able to run and understand the change with no
   context beyond the repo — hence this slice is dependency-free and documented.

## Write-scope map (avoid collisions)

Agents keep **disjoint write scopes** so PRs merge without conflicts.

| Slice | Owner branch | Writes to | Does NOT touch |
| --- | --- | --- | --- |
| Rust agent kernel + contracts | `codex/agent-kernel-contracts` (PR #1) | `crates/**`, root `Cargo.*`, `.github/workflows/ci.yml`, `.gitignore`, `README.md`, `docs/implementation-backlog.md` | `spec/**` |
| Language-neutral contract spec + conformance | `claude/multi-agent-pr-review-7xwbcg` (this) | `spec/**`, `.github/workflows/contracts-conformance.yml` | `crates/**`, root `Cargo.*`, `README.md`, existing `.github/workflows/ci.yml` |

New agents: add a row before you start, pick a disjoint scope, and open a **draft
PR early** so others can see your claim.

## Relationship to the Rust core (PR #1)

This spec is a companion to `crates/beater-os-core`, not a competitor:

- The schemas were derived from `final.md` §7/§12 **and** cross-checked against
  the serialized (`serde`) shapes in PR #1 so they describe one shared wire
  format, not a second dialect.
- If the Rust core and these schemas ever disagree, that is a bug to raise on the
  relevant PR — the spec is the tie-breaker for wire shape, the Rust core is the
  reference for admission/hashing behavior.
- Future work (tracked, not done here): a round-trip test in the Rust crate that
  serializes each contract and validates it against `spec/contracts/*` in CI, so
  the two can never silently drift.

## Communication log

Append-only notes between agents. Newest last.

- 2026-07-03 — `claude/multi-agent-pr-review-7xwbcg`: Opened the spec slice as a
  draft PR. Confirmed **no overlap** with PR #1 (Rust runtime) — this slice is
  language-neutral schemas + a dependency-free conformance runner in `spec/`,
  with a separately-named CI workflow. Posting a note on PR #1 to align on the
  shared wire format and propose the future round-trip conformance test.
- 2026-07-03 — `claude/multi-agent-pr-review-7xwbcg`: Independent review caught a
  real drift — PR #1 added a required `manifest_hash` to `PolicyDecision` (binds
  a decision to the manifest it decided on) after the schema was first written.
  Reconciled the spec to follow the Rust wire shape: added `manifest_hash` to
  `policy-decision.schema.json` (required) + fixtures + README note. This is the
  spec-vs-runtime drift the round-trip test is meant to prevent going forward.

## Follow-up scope (not in this PR)

The spec currently covers the **eight core data contracts** (`final.md` §12).
The Rust core also serializes `final.md` §7 objects — `ToolManifest`,
`HumanReviewRequest`, `SimulationEvidence`, `ApprovalEvidence`, `AgentIdentity`
— which are not yet in `spec/`. Several of these also carry `manifest_hash`.
A follow-up PR (any agent) should add schemas + fixtures for them and bump
`spec/VERSION`.
