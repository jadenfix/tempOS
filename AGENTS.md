# AGENTS.md — beaterOS multi-agent operating contract

beaterOS is being built by **several autonomous agents working in parallel on
the same repository**, alongside human maintainers. This file is the contract
that keeps that safe and coherent. It is intentionally short and normative; the
detailed process lives in
[`docs/multi-agent-review-protocol.md`](docs/multi-agent-review-protocol.md)
and the reviewer rubric in [`docs/review-checklist.md`](docs/review-checklist.md).

The product design this repository implements is [`final.md`](final.md). Nothing
in this file overrides `final.md`; it governs *how* we land changes against it.

## 0. Why this file exists

Multiple probabilistic agents editing one repo is exactly the class of problem
`final.md` says beaterOS itself must solve for the outside world: work must be
**observable, reproducible, permissioned, and reviewed outside the actor**. We
hold our own development to that standard. Concretely:

- No agent silently merges its own work.
- Every change is reviewed by a *different* agent or person than the author.
- Any qualified reviewer may review, approve, and merge *any* PR they did not
  author. Ownership is **shared**, not per-author.
- Agents coordinate through durable GitHub artifacts (branches, PRs, reviews,
  and the coordination log), not through hidden state.

## 1. Shared ownership (no gatekeeping)

There is **no exclusive owner** of any file, crate, or module. The agent that
first wrote a component does not own it. Every agent and maintainer has full
authority to read, refactor, extend, review, approve, and merge any part of the
codebase they did not author, subject to the review rules below.

Corollary: write code any reviewer can understand. Prefer clarity over
cleverness, document non-obvious invariants inline, and keep public contracts
typed and tested so a reviewer who has never seen the code can reason about it.

## 2. Branch and identity conventions

- Each agent works on branches prefixed with its identity, e.g. `codex/*`,
  `claude/*`, plus a short slice name: `codex/session-runtime`,
  `claude/multi-agent-pr-review`.
- One coherent feature slice per branch and per PR. Keep slices small enough to
  review in one sitting and mapped to a named section of `final.md`.
- Rebase onto the latest `main` before opening or updating a PR.

## 2a. Agent identity vs GitHub account (read this before relying on "author")

All agents currently push commits and open PRs under the **same GitHub account**
(the repo owner). GitHub therefore sees one author and one merger for everything,
so the "no one merges their own PR" rule **cannot** be enforced by GitHub-account
identity or by CODEOWNERS. Independence is tracked at the **agent** level:

- The **branch prefix** (`codex/*`, `claude/*`, …) identifies the authoring agent.
- The PR body's `Review routing` section names who reviewed and who merged.
- The [coordination log](docs/agent-coordination-log.md) is the durable record.

CI can machine-check repository invariants (e.g. `final.md` not gutted) but it
**cannot** prove that an independent agent reviewed or merged a change — that is a
social invariant. The routing checklist and coordination log are the auditable
evidence; falsifying them defeats the purpose. `tools/pr_governance_check.py` can
verify author≠merger only when you pass the two **agent** identities to it.

## 3. The review-and-merge rule (non-negotiable)

For **every** PR:

1. The author opens the PR using the shared
   [`PULL_REQUEST_TEMPLATE.md`](.github/PULL_REQUEST_TEMPLATE.md).
2. A **different** agent or person performs a Deep PR Review (DPR) against
   [`docs/review-checklist.md`](docs/review-checklist.md) and records it as a
   GitHub review (`APPROVE`, `REQUEST_CHANGES`, or `COMMENT`).
3. The author addresses blocking findings; re-review follows the same rule.
4. Merge is performed by an agent or person who **did not author** the PR, only
   after an `APPROVE` from an independent reviewer.

An agent may spawn sub-agents to perform the DPR and the merge on its behalf, as
long as the reviewing/merging sub-agent is given the diff fresh and reasons
independently. The invariant is *independence of review from authorship*, not
the particular process that produces it.

High-risk PRs (see the checklist's risk tiers — anything touching capability
issuance, policy admission, the journal/receipt chain, secrets, or payments)
require **two** independent approvals before merge.

## 4. Coordination / communication loop

Parallel agents avoid collisions and stay in sync through two durable channels:

- **[`docs/agent-coordination-log.md`](docs/agent-coordination-log.md)** — an
  append-only ledger. Before starting a slice, add a row claiming your branch,
  scope, and the files you expect to write. This is how agents discover who is
  touching what and keep **disjoint write scopes**.
- **GitHub PRs, reviews, and comments** — the message bus between agents. If
  your change depends on, overlaps with, or supersedes another open PR, say so
  in your PR body and (if needed) as a comment on the other PR.

The implementation roadmap and slice dependencies live in
[`docs/implementation-backlog.md`](docs/implementation-backlog.md). Treat the
`Depends On` column as a scheduling hint: do not build on an unmerged slice's
internals unless you coordinate in the log first.

## 5. What must not be compromised

These mirror `final.md` Section 26 and are review blockers regardless of slice:

- No ambient authority — every dangerous action needs an explicit capability grant.
- Journal before side effects; receipts after side effects.
- Policy is evaluated outside the model, deterministically.
- Memory carries provenance; grants are session- and identity-bound.
- Eval gates, tool identity, revocation, and human-legible authority are preserved.
- Standard cryptography only — no invented primitives.
- `final.md` is never shortened or weakened to make an implementation "pass"
  (in the spirit of §26; enforced mechanically by `tools/pr_governance_check.py`).

## 6. Local checks before you request review

- Rust workspace: `cargo fmt --all -- --check`, `cargo test --workspace --locked`,
  `cargo clippy --workspace --all-targets --locked -- -D warnings`.
- Repo governance: `python3 tools/pr_governance_check.py --repo .` and, for the
  governance tool itself, `python3 -m unittest discover -s tools -p 'test_*.py'`.

CI runs both the Rust job ([`.github/workflows/ci.yml`](.github/workflows/ci.yml))
and the governance job ([`.github/workflows/governance.yml`](.github/workflows/governance.yml)).
