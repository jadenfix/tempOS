# Contributing to beaterOS

beaterOS is built by **multiple agents working in parallel** alongside human
contributors. To keep that safe and collision-free, everyone — human or agent —
follows one shared contract.

## Start here

1. **Read [`AGENTS.md`](AGENTS.md) → "Multi-Agent Contribution & Review
   Contract".** It is the canonical process: claiming work, independent review,
   no self-merge, shared ownership, and the honesty boundary on attested agent
   identity.
2. **Claim a disjoint write scope in [`docs/coordination.md`](docs/coordination.md)**
   (the work-claiming board) *before* you start, so you don't collide with
   another agent.
3. **Read the relevant section of [`final.md`](final.md)** — the design your
   change implements. Never shorten or weaken `final.md`.

## The non-negotiable rules

- **No self-merge.** The agent/person who authored a PR never merges it. A
  *different* party reviews, and a party who is *not the author* merges.
- **Independent review is required.** Every PR gets a deep review (DPR) from a
  non-author, recorded in [`docs/governance/`](docs/governance/) — the review
  gate (checklist + linter).
- **Shared ownership.** Every reviewer has full authority over the whole tree
  (`.github/CODEOWNERS`). Write code any reviewer can understand and change.
- **Claim before you build.** Register a disjoint write scope to avoid colliding
  with other agents.
- **Policy outside the actor.** The rules are also enforced by CI
  (`.github/workflows/pr-governance.yml`) and the ledger linter
  (`scripts/check-governance.py`), not by good intentions alone.
- **Fail-closed pre-merge gate.** Once a PR is **ready (not a draft)**, the
  governance check *fails* (not warns) unless it declares a concrete
  `Reviewer-Agent` distinct from `Author-Agent` and a non-self `Merged-By`. A
  draft may leave the reviewer `pending`. Recording the merge later in the ledger
  is audit evidence, not the gate — the gate is this red/green check made
  merge-blocking by **branch protection** on `main`. (Residual limit: the check
  can't prove a declared agent-id is truthful under one shared GitHub account —
  the documented honesty boundary.)

## Opening a PR

- Branch as `<agent-id>/<slice>` (e.g. `claude/multi-agent-pr-review`).
- Fill in the PR template **and add the agent routing trailer** to the PR body
  (the shared template does not yet include it — a follow-up will add it):

  ```
  Author-Agent: <agent-id>
  Reviewer-Agent: <agent-id or "pending">
  Merged-By: <agent-id / "human:@jadenfix" / "pending">
  ```

  Fill it truthfully; `Merged-By` must differ from `Author-Agent`. The
  `pr-governance.yml` check reads this block.
- Keep the change small and contract-focused; link the `final.md` section(s).
- For Rust changes run `cargo fmt --check`, `cargo test --workspace`, and
  `cargo clippy --workspace --all-targets`.

## Recording review & merge

- A non-author reviewer fills the checklist in
  [`docs/governance/review-checklist.md`](docs/governance/review-checklist.md)
  and leaves a COMMENT review with an explicit verdict (GitHub blocks a formal
  Approve on your own account).
- The completed merge is recorded in
  [`docs/governance/coordination-ledger.md`](docs/governance/coordination-ledger.md)
  — the canonical review/merge audit ledger — through a follow-up PR, not a
  direct push to `main`. `python3 scripts/check-governance.py` must stay green.

## Trust model, honestly

All agents currently act as the same GitHub account, so GitHub cannot tell one
agent from another. Agent identity is **attested**, and the rules above are
enforced by convention + CI structural checks + the linter, not
cryptographically. See the honesty boundary in `AGENTS.md`. The real gate is
branch protection on `main`; per-agent signing identities (`final.md` §7.1)
would make it verifiable.
