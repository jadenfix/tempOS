# beaterOS Multi-Agent Coordination Protocol

Several agents (and people) build beaterOS in parallel on the same repository.
This document is the **communication loop** that keeps that safe: how work is
claimed, how collisions are avoided, how reviews are requested, and who is
allowed to merge. It complements — and does not replace — the implementation
backlog (`docs/implementation-backlog.md`, which lands with the Rust workspace PR
and lists the feature slices) and [`final.md`](../final.md), which is the plan.

It is fitting that an agent-first OS practices what it preaches: no ambient
authority (no one merges their own work), everything journaled (every change is
a reviewed PR), and legible authority (this document).

## 1. Roles

Any agent or person may take any of these roles on a given PR, with one hard
rule: **the author of a PR may not be its approver or its merger.**

- **Author** — writes a feature slice on a branch and opens a PR.
- **Reviewer** — an agent/person who did *not* author the PR; reads the diff,
  runs the checks, files findings, approves or requests changes.
- **Merger** — an agent/person who did *not* author the PR; performs the merge
  once it is approved and green.

A single PR therefore involves at least two distinct principals. This is the
non-ambient-authority principle applied to the development process itself.

## 2. Claiming work without collisions

1. Pick a slice from the backlog (or propose a new one).
2. Use a branch named `<agent>/<slice>` (e.g. `codex/session-runtime`,
   `claude/multi-agent-pr-review`). The prefix identifies the author fleet.
3. Keep write scopes **disjoint**. Before starting, check open PRs and branches
   for overlapping paths. If two slices must touch the same file, coordinate in
   a PR comment first; do not race the file.
4. Prefer additive, layered contributions over edits to another agent's
   in-flight files. New directories and new files never conflict; shared root
   files (`Cargo.toml`, `.gitignore`, `README.md`, workflow files) do — treat
   them as coordination points, not free-for-alls.

### Known ownership (snapshot — keep current)

| Area | Owner branch prefix | Notes |
| --- | --- | --- |
| Rust workspace, `crates/beater-os-core`, backlog | `codex/*` | reference implementation of the contracts |
| Language-neutral contracts (`contracts/`), coordination + review governance (`docs/`, `AGENTS.md`), `final.md` integrity guard (`tools/`) | `claude/*` | schema source of truth every implementation conforms to |

## 3. The review loop (required for every PR)

```
author: open PR  ->  reviewer (non-author): review + findings
                 ->  author: address findings, push
                 ->  reviewer (non-author): re-review
                 ->  merger (non-author): merge when approved + green
```

- Reviews use [`docs/review-checklist.md`](review-checklist.md).
- Findings are posted on the PR so the loop is on the record, not in a side
  channel. Anyone in the fleet can read why a change was made.
- If a finding is ambiguous or architectural, escalate to the human owner rather
  than guessing.
- Re-review after fixes is by a non-author (ideally the same reviewer, for
  continuity). The **merge is always by a non-author**.

## 4. Cross-fleet communication

- **Contracts are the interface.** If your component talks to another (Rust
  crate ⇄ Python service ⇄ MCP gateway), it does so through the schemas in
  `contracts/`. Divergences are logged in `contracts/README.md` and raised on the
  relevant PR, not fixed silently on one side.
- **`final.md` is immutable-by-default.** It may grow, never shrink. The guard in
  `tools/final_integrity.py` fails CI if a pinned section disappears or the doc
  is truncated. If you intend an additive edit, re-pin and say so in the PR.
- **Leave the repo legible.** Every file should be understandable — and
  changeable — by any reviewer in the fleet, not only its author. Prefer clear
  names, docstrings, and tests over cleverness.

## 5. Shared invariants no PR may weaken

These come straight from `final.md` and hold across every fleet:

1. No ambient authority — every side effect needs an explicit capability grant.
2. Capability checks happen outside model output.
3. Side-effecting actions are represented by manifests and receipts.
4. Policy decisions are journaled before execution.
5. `final.md` is never shortened or weakened.
6. No agent merges its own PR.

If a change appears to require weakening one of these, that is a signal to stop
and ask the human owner, not to proceed.
