# AGENTS.md — operating manual for the beaterOS fleet

This repository is built by multiple agents (and people) working in parallel.
Read this before making changes so the fleet stays coordinated and the repo stays
legible to everyone, not just whoever wrote a given file.

## The one rule that is never bent

**No agent merges its own PR.** Author, approver, and merger are distinct
principals. This is the non-ambient-authority principle from `final.md` applied
to our own process.

## Where things live

- [`final.md`](final.md) — the plan and source of truth. Grows, never shrinks.
- [`contracts/`](contracts/) — language-neutral JSON Schemas for the core data
  contracts (final.md §12). Every implementation conforms to these.
- [`docs/multi-agent-coordination.md`](docs/multi-agent-coordination.md) — how
  agents claim work, avoid collisions, and run the review loop.
- [`docs/review-checklist.md`](docs/review-checklist.md) — how to review a PR you
  did not author.
- `docs/implementation-backlog.md` — the feature slices and their branches
  (lands with the Rust workspace PR).
- [`tools/`](tools/) — repo guards and validators (`final_integrity.py`,
  `contracts_validate.py`).
- `crates/` — the reference Rust implementation of the contracts.

## Before you start a slice

1. Pick/propose a slice; branch as `<agent>/<slice>`.
2. Check open PRs/branches for path collisions; keep write scopes disjoint.
3. Prefer new files/dirs over editing another agent's in-flight files.

## Before you open a PR

- Run the checks for your slice type:
  - Rust: `cargo fmt --check && cargo test --workspace && cargo clippy --workspace --all-targets`
  - Contracts/tooling: `python3 -m unittest discover -s tests && python3 tools/contracts_validate.py && python3 tools/final_integrity.py`
- Fill in the PR template, including the **review routing** section.
- Include negative tests, not only happy paths.

## Review and merge

- Request review from a non-author using `docs/review-checklist.md`.
- Address findings on the branch; re-review is by a non-author.
- A non-author merges once approved and green, and records the merge on the PR.

## Communication

- Talk through PR comments and `contracts/README.md`, not side channels, so the
  whole fleet can see decisions.
- Escalate ambiguous or architectural questions to the human owner.
- Never weaken a `final.md` invariant to make a change fit; stop and ask.
