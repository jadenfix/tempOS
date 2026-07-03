# beaterOS Review Gate

This directory is the **review gate** for beaterOS PRs: a concrete, reusable
checklist a *non-author* reviewer runs against every PR, plus a linter that
enforces the "no self-merge / independent review" rule at the agent-identity
layer.

It is deliberately **scoped to review tooling** so it does not duplicate the
broader multi-agent contribution contract. For the contribution process itself
(claiming work, lanes, `CONTRIBUTING`, `CODEOWNERS`, the CI enforcement
workflow), see the governance PR that owns that backbone
(`AGENTS.md` / `docs/coordination.md`, PR #19). This gate is designed to be
*called by* that workflow.

## Contents

- **[review-checklist.md](review-checklist.md)** — turns `final.md` §26
  (never-compromise), §12 (contracts), and §13 (security) into a per-PR checklist
  with a reviewer sign-off block. This is what a non-author reviewer fills in.
- **[coordination-ledger.md](coordination-ledger.md)** — the agent-layer record
  of who authored / reviewed / merged each PR (approvals can't live in GitHub's
  "Approve" because all agents share one account). Proposed to merge into the
  single canonical ledger owned by the governance-backbone PR.
- **[../../scripts/check-governance.py](../../scripts/check-governance.py)** — a
  dependency-free linter that fails if a `merged` PR's merger equals its author,
  or an in-review PR's reviewer equals its author.

## Why review lives at the agent layer

Every agent (codex, claude, sub-agents) authenticates as the same GitHub account,
so GitHub cannot tell them apart: it blocks a formal **Approve** on your own PR
and cannot enforce "a different agent merged this." Approval is therefore a
COMMENT review that names the reviewer agent + an explicit verdict, recorded in
the ledger; merge-by-a-different-agent is the enforcement lever; and the linter
below makes violations visible.

## Run it

```sh
python3 scripts/check-governance.py [path/to/ledger.md]
```

Exit 0 = clean, exit 1 = a self-merge or self-review slipped in.

## Recording a merge

Record a completed merge in the ledger through a **follow-up PR** (or fold it
into the next PR), reviewed and merged by a non-author — do **not** push the
ledger update straight to `main`. A direct-to-`main` commit bypasses the very
review this gate exists to guarantee; the only reason a merge-record can't ride
along in its own PR is the self-reference (a PR can't record that it was merged
before it is), and a small follow-up PR resolves that cleanly.
