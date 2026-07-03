# Multi-Agent Coordination

beaterOS's `final.md` is implemented by several agents (e.g. `codex`, `claude`)
working the same repository in parallel. This document describes the
coordination kernel that keeps that parallel work safe, and how to use it.

The kernel is not a new idea bolted on: it applies beaterOS's own operating
principles — bounded authority, policy outside the model, tamper-evident
journals, and independent review — to the project's *own development process*.
It is implemented in the [`beater-os-coordination`](../crates/beater-os-coordination)
crate and driven by the [`beater-coord`](../crates/beater-coord) CLI.

## Why

Parallel agents on one repo fail in three predictable ways:

1. **Clobbering.** Two agents edit the same files and silently overwrite each
   other. → *Solution: disjoint write scopes per claim.*
2. **Self-approval.** An agent reviews and merges its own work, so nothing is
   independently checked. → *Solution: a deterministic merge gate enforcing
   author ≠ reviewer ≠ merger.*
3. **Lost context.** Coordination happens in ephemeral chat, so no agent can
   reconstruct who did what. → *Solution: an append-only, hash-chained ledger.*

These map directly to `final.md`: no ambient authority (13.2), capability
attenuation and delegation (5.2, 20.7), human/independent review (7.9, 13.14),
policy outside the model (8.12), tamper-evident logs (13.11), and explainable
denials (22.9).

## Model

| Concept | Type | Role |
| --- | --- | --- |
| Principal | `AgentPrincipal` | Who may author, review, merge. Equal authority over work it did not author. |
| Claim | `SliceClaim` + `WriteScope` | One backlog slice, owned by one principal, bounding the repo paths it may write. |
| Review | `ReviewAttestation` | Commit-bound evidence that a *non-author* reviewed a slice. Self-review is rejected at construction. |
| Merge gate | `MergePolicy` → `MergeGateDecision` | Deterministic admission: independent approval(s), green CI, merged dependencies, non-author merger. |
| Ledger | `CoordinationLedger` | Append-only, SHA-256 hash-chained record of every step. |

`Coordinator` composes these into one replayable source of truth. Every
mutating call is journaled; `verify` checks the whole chain.

### Equal authority

Authority is **not** role-gated. Any registered principal may review or merge
any slice it did not author. The only hard rules are independence
(reviewer ≠ author) and non-self-merge (merger ≠ author). This realizes the
requirement that reviewers "have the power to do everything with it, not just
the one that wrote it."

### Write-scope disjointness

A claim owns a set of repo-relative path prefixes. Directory prefixes end in
`/` (`crates/foo/`); a prefix without a trailing slash matches an exact file
(`Cargo.toml`). Two active claims must be disjoint — overlap in *either*
direction (nesting or equality) is a conflict, journaled as `ConflictDetected`
and refused. Sibling directories that merely share text (`crates/foo/` vs
`crates/foobar/`) do **not** conflict, because matching respects path segments.

`Cargo.toml`, `Cargo.lock`, and files under `docs/` are shared coordination
surfaces; treat edits to them as append-only and low-conflict, and expect to
resolve the occasional merge on the workspace member list.

## The loop

```
init ──▶ register ──▶ claim ──▶ (build) ──▶ status in_review
                                                   │
                                     review (by a non-author)
                                                   │
                                    gate ──▶ Allowed?
                                       │        │
                                    Denied   merge (by the approver / any non-author)
                                       │        │
                              fix & re-review   status merged
```

## CLI usage

```sh
# One-time, per checkout:
beater-coord init --policy-version coord-policy-v1
beater-coord register --id codex  --name "Codex agent"
beater-coord register --id claude --name "Claude agent"

# Claim disjoint work:
beater-coord claim --slice agent-kernel-contracts --by codex \
  --branch codex/agent-kernel-contracts --scope crates/beater-os-core/,Cargo.toml

beater-coord claim --slice coordination-kernel --by claude \
  --branch claude/multi-agent-pr-review --scope crates/beater-os-coordination/,crates/beater-coord/

# Open for review, get an INDEPENDENT review, and check the gate:
beater-coord status --slice coordination-kernel --to in_review
beater-coord review --slice coordination-kernel --by codex --commit <sha> --verdict approve
beater-coord gate   --slice coordination-kernel --merger codex --commit <sha> --ci-green

# If the gate authorizes, record the merge (never by the author). The commit
# must match the exact commit the gate authorized:
beater-coord merge  --slice coordination-kernel --merger codex --decision <decision_id> --commit <sha>

# Inspect and audit at any time:
beater-coord list
beater-coord conflicts
beater-coord journal
beater-coord verify
```

The store defaults to `.beater/coordination.json` (git-ignored; it is runtime
state, not source). Pass `--store <path>` to override.

## Guarantees

- A claim cannot be created that overlaps an active claim's write scope.
- A review cannot be attributed to its own author.
- The merge gate denies unless: the merger is not the author, at least
  `min_independent_approvals` distinct non-author approvals exist **at the exact
  reviewed commit**, CI is green, and every declared dependency is merged. A
  stale `RequestChanges` on an older commit does not block a fixed one.
- `mark_merged` requires a prior `Allowed` gate decision that named that merger
  **and that exact commit**, and the claim must still be `Approved` for that
  commit. A stale authorization cannot merge a later, unreviewed commit;
  returning a claim to `InReview` (e.g. on a new push) clears its approval.
- Any edit to a journaled record is detected by `verify`.

Path prefixes are canonicalized (`//`, `/./`, and leading `./` collapse) before
overlap checks, so redundant spellings cannot dodge disjointness. Comparison is
case-sensitive, matching Git's view of paths.
