# beaterOS Multi-Agent Operating Protocol

This repository is built by **several autonomous agents working in parallel**
(e.g. `codex/*` and `claude/*` branch families). This file is the shared
**communication loop**: how agents claim work, avoid collisions, review each
other, and merge safely. It complements — and does not replace — `final.md`
(the design source of truth) and `docs/implementation-backlog.md` (codex's
PR-sized slice plan).

If you are an agent starting work on this repo: **read this file first**, then
`final.md`, then check open PRs before writing any code.

## 1. Non-negotiable review rules

These are lifted from the user's standing instructions and `final.md` §26.

1. **No self-merge.** The agent (or person) who authored a PR must never merge
   it. Merge is always performed by a *different* agent.
2. **Independent review required.** Every PR is reviewed by an agent/person who
   did not author it, before merge. High-risk or contract-changing PRs get a
   second independent review after fixes.
3. **All reviewers are empowered.** Any reviewer may read, run, critique, fix,
   approve, or block *any* PR — authority is not limited to the author. Code
   must be legible enough for that to be true (clear names, docs, tests).
4. **Draft-first / intent-first.** Open a **draft PR stating your intended
   scope as early as possible**, before large implementation, so other agents
   can see what you are claiming and pick non-overlapping work.
5. **Deconflict on overlap.** If your intended slice is substantially similar to
   another open PR, do **not** race it. Comment on the other PR, compare
   approaches, agree who owns it (better approach / further along wins), and the
   loser takes a different slice. Record the outcome in the slice registry below.
6. **`final.md` is never shortened or weakened** as part of implementation.

## 2. Communication channels

| Channel | Purpose |
| --- | --- |
| Draft PRs | Announce intent + current scope. The canonical "who is doing what". |
| PR comments | Deconfliction, review, approve/merge handoff between agents. |
| This file (`AGENTS.md`) | Durable slice-ownership registry + protocol. |
| `docs/implementation-backlog.md` | codex-family sequential slice plan (§ from `final.md`). |

Before starting: `list_pull_requests(state=all)` and read every open PR's
scope. After claiming: update the registry below in your own PR.

## 3. Collision avoidance: disjoint write scopes

Parallel agents must write to **disjoint file scopes** so branches merge without
conflict while multiple PRs are in flight.

Reserved / owned scopes (update when you claim new scope):

| Scope (paths) | Owner family | Notes |
| --- | --- | --- |
| `Cargo.toml`, `Cargo.lock`, `.gitignore`, root `README.md` | codex | Rust workspace root. Do not edit from other families without coordination. |
| `crates/beater-os-core/**` | codex | Core contracts, journal, receipts, policy (PR #1). |
| `.github/workflows/ci.yml`, `.github/PULL_REQUEST_TEMPLATE.md` | codex | Rust CI + PR template. |
| `docs/implementation-backlog.md` | codex | Slice plan. |
| `AGENTS.md` | shared | Append-only registry; coordinate edits, avoid clobbering rows. |
| `contracts/**` | claude | Language-neutral JSON Schemas for the core contracts. |
| `examples/traces/**` | claude | Golden end-to-end trace corpus. |
| `scenarios/**` | claude | Adversarial security / eval scenario corpus. |
| `tools/conformance/**` | claude | Dependency-free conformance gate (schemas + invariants). |
| `docs/threat-model.md`, `docs/glossary.md` | claude | Phase-0/1 doc deliverables. |
| `.github/workflows/contracts.yml` | claude | Conformance CI (separate file from `ci.yml`). |

Rule of thumb: a new slice should either create a **new top-level directory** or
a **new file**, never edit another family's file, unless coordinated in a PR
comment first.

## 4. Slice registry

| Slice | Branch | Family | Status | Depends on | Scope summary |
| --- | --- | --- | --- | --- | --- |
| Core contracts | `codex/agent-kernel-contracts` | codex | PR #1 open (draft) | — | Rust `beater-os-core`: 6 contracts, journal, receipts, policy admission. |
| Conformance suite + multi-agent governance | `claude/multi-agent-pr-review-a3bwl1` | claude | this PR | reads #1 (no build dep) | JSON Schemas + example traces + adversarial scenarios + Python eval gate + threat model + this protocol. |

codex's slices 2–17 (`docs/implementation-backlog.md`) remain owned by the
codex family. The conformance suite is designed to *serve* those slices: as new
contracts/behaviours land, schemas + scenarios here become their release gate
(`final.md` §5.9, §14.6).

## 5. Why the conformance slice does not collide with the core crate

- It adds only new top-level dirs (`contracts/`, `examples/`, `scenarios/`,
  `tools/conformance/`) plus new files under `docs/` and a **separately named**
  CI workflow. It edits **no** file owned by the core crate.
- It has **no build dependency** on the (still-unmerged) Rust core, so it can
  land in parallel. It instead re-derives the core's admission/causality/hash
  invariants independently in Python, acting as a cross-language differential
  oracle for the Rust implementation (`final.md` §15.6: verifier separate from
  executor).

## 6. Open cross-implementation coordination items

1. **Canonical hashing must converge.** The Rust core hashes values using
   serde's struct-declaration field order (`crates/beater-os-core/src/hash.rs`).
   That is not canonical across languages, so a TypeScript/Python re-implementation
   cannot reproduce the same digests. Recommendation: all implementations adopt a
   JSON Canonicalization Scheme (JCS, RFC 8785) layout — sorted keys, compact
   separators — which the conformance harness (`tools/conformance/canonical.py`)
   already uses. Owner: TBD (needs a joint codex+claude PR touching `hash.rs`).
2. **Shared enum/field vocabulary.** The JSON Schemas in `contracts/` are pinned
   to the Rust field names and snake_case enum values as of PR #1. If the core
   crate renames a field or enum variant, update the matching schema in the same
   or a follow-up PR and note it here.
