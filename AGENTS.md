# AGENTS.md — Multi-Agent Contribution Contract

This file is the **canonical, shared contract that every agent working on
beaterOS must read before touching the repository**, whether it is Codex,
Claude, or any future automated contributor. Human contributors follow the same
rules (see `CONTRIBUTING.md`, which points here).

Tools read different files by convention (`AGENTS.md`, `CLAUDE.md`, `.github`),
so this is the single source of truth and other agent files should link to it
rather than restate it.

> **One-line rule:** No agent merges its own work. Every change is proposed by
> one agent, reviewed by a *different* agent, and merged by a *third party* who
> is neither the author nor blocked from acting on it.

---

## 1. Why this exists

Several agents run **in parallel on this one repository**. Without a protocol
they collide on files, duplicate slices, and — most dangerously — an agent can
rubber-stamp and merge its own work, which defeats the point of review.

The dev process deliberately mirrors the safety philosophy of beaterOS itself
(see `final.md`). The same principles that make agent *work* safe make agent
*development* safe:

| beaterOS principle (`final.md`) | How this repo's process applies it |
| --- | --- |
| No ambient authority (§13.2) | No agent has standing authority to merge its own PR. |
| Policy is enforced outside the model (§8.12) | Merge rules are enforced by CI + CODEOWNERS, not by an agent's good intentions. |
| Journal before side effects (§4.5) | Agents record intent in `docs/coordination.md` *before* starting work. |
| Receipts after side effects (§7.6) | Reviews and merges are recorded as durable artifacts (PR reviews, ledger updates). |
| Least authority / attenuation (§8.2) | Agents claim the **narrowest** slice with a disjoint write scope. |
| Human-legible authority (§3.3) | Anyone can read the ledger and know who is doing what, and why. |

---

## 2. Principals

- **Human owner** — `@jadenfix`. Final authority; can override any rule.
- **Author agent** — the agent that writes a change on a branch and opens a PR.
- **Reviewer agent** — a *different* agent that performs an independent review
  ("DPR" = deep PR review).
- **Merger** — a party that is **not the author agent**: a reviewer agent that
  approved, another agent, or the human owner.

An "agent" is identified by its **agent id** (e.g. `codex`, `claude`), declared
in branch names, PR trailers, and the coordination ledger. Note the honesty
boundary in §7: GitHub sees all agents as the same human account, so agent
identity is an **attested** property, not something GitHub can cryptographically
enforce here. The protocol is built to make violations visible, not impossible.

---

## 3. The lifecycle of a change

```
 claim  ->  branch  ->  build  ->  PR  ->  independent review  ->  independent merge  ->  release claim
   |          |           |         |             |                        |                    |
 ledger    <agent>/    disjoint   uses PR      reviewer != author     merger != author     ledger
 entry      <slice>    write      template     (DPR + verdict)        (CI-checked)          updated
```

### 3.1 Claim (communication loop — do this first)

Before writing code, add or update your row in **`docs/coordination.md`** with:
your agent id, the slice you are taking, the branch name, the files/paths you
expect to write (your **write scope**), and any dependency on another agent's
in-flight branch. This is the primary channel of the communication loop — read
it before you start so you do not collide with another agent.

If your intended write scope overlaps another agent's active claim, **do not
start**. Pick a different slice, narrow your scope, or coordinate via a PR/issue
comment on their thread.

### 3.2 Branch

Branch name: `<agent-id>/<slice-kebab-case>` (e.g. `claude/multi-agent-pr-review`,
`codex/session-runtime`). Never commit directly to `main`.

### 3.3 Build

Keep the change **contract-focused and small**, mapping to named section(s) of
`final.md`. Match the surrounding code's style. Do not shorten or weaken
`final.md`. Do not introduce ambient authority.

### 3.4 PR

Open a PR into `main` using `.github/PULL_REQUEST_TEMPLATE.md`. Fill in the
**Agent routing trailer** (§4) truthfully. Link the `final.md` section(s) and
any coordination-ledger row.

### 3.5 Independent review (DPR)

A **different agent** reviews. The review must:
- Check the change against the `final.md` sections it claims to implement.
- Verify the contract checklist (no ambient authority, policy-outside-model,
  journal/receipt coverage where applicable, tests).
- Leave a verdict: `APPROVE`, `REQUEST_CHANGES`, or `COMMENT`, with reasons.

### 3.6 Independent merge

The merge is performed by someone who is **not the author agent** — normally the
reviewer agent that approved, or the human owner. The author agent must never
merge its own PR. If you authored it, hand it off.

### 3.7 Release claim

After merge, update `docs/coordination.md` (mark the slice `merged`), and delete
the branch/worktree. Freeing your write scope lets the next agent proceed.

---

## 4. Agent routing trailer

Every PR body must contain this block (the PR template includes it). It makes
authorship and hand-off legible and is what the governance workflow checks:

```
<!-- agent-routing -->
Author-Agent: <agent-id>
Reviewer-Agent: <agent-id or "pending">
Merged-By: <agent-id / "human:@jadenfix" / "pending">
```

Invariant: `Merged-By` must differ from `Author-Agent` at merge time. A PR where
they are equal is a self-merge and must be rejected.

---

## 5. Shared ownership — every reviewer has full power

No file is owned by only the agent that wrote it. `.github/CODEOWNERS` routes
review to the shared owner, and this contract grants **every** listed agent the
authority to read, review, request changes on, and (when not the author) merge
**any** part of the tree. There is no "only the author can touch this" code.
Write code and docs that any reviewer can fully understand and modify:

- Explain non-obvious decisions in comments or the PR body.
- Prefer clear names and small functions over cleverness.
- Keep public contracts documented where they live.

---

## 6. Collision avoidance rules

- **Disjoint write scopes.** Two active branches must not write the same files.
  The ledger is how you check.
- **Additive over edit.** When practical, add new files rather than editing
  shared ones (e.g. `README.md`, `final.md`, `Cargo.toml`, shared workflows), to
  reduce merge conflicts between parallel agents.
- **Declare dependencies.** If your slice depends on another agent's unmerged
  branch, record it in the ledger and note it in your PR; do not silently fork
  their work.
- **Talk on the thread.** Cross-agent questions go on the relevant PR/issue as
  comments — that is a durable, shared channel every agent and the human can see.

---

## 7. Honesty boundary (what is and isn't enforced)

Be precise about the trust model so no one over-trusts it:

- GitHub authenticates the **human account**, not the agent. All agents here act
  as `@jadenfix`, so GitHub's own "author cannot approve their own PR" and
  identity checks **cannot** separate one agent from another.
- Therefore agent identity is **attested** (declared in the routing trailer and
  ledger) and enforced **socially + by convention + by the governance workflow's
  structural checks**, not cryptographically.
- The governance workflow (`.github/workflows/pr-governance.yml`) checks what is
  *structurally* checkable: the routing trailer exists, is filled in, and does
  not declare a self-merge. It cannot prove the declared agent ids are truthful.
- This is a real limitation, not a bug to paper over. If beaterOS later issues
  per-agent signing identities (see `final.md` §7.1 Agent Identity), those can
  sign PRs/merges and upgrade this from attested to verifiable.

Additional boundaries (raised in independent review of PR #19):

- **The pre-merge check is bypassable.** `pr-governance.yml` runs on
  `pull_request` *body* events, not at merge time, so it never sees who actually
  clicked merge. An author who self-merges can simply leave `Merged-By: pending`
  (a placeholder, which passes) and merge anyway. The check raises the floor; it
  is not a gate on its own.
- **The real gate is branch protection.** The green check is only enforcing if
  `main` is a protected branch that *requires* a passing `pr-governance` check
  **and** a CODEOWNERS review before merge. Without protected `main`, this
  workflow is **advisory** — do not over-trust the green check. Enabling branch
  protection on `main` is the owner action that turns this contract from
  convention into enforcement.
- **After-the-fact receipt.** `pr-governance.yml` also has a merge-receipt job
  (`on: pull_request_target: [closed]`) that, when a PR is actually merged,
  records the real `merged_by.login` back on the thread — a durable receipt
  (§4.5/§7.6 analogy). It still cannot separate agent-ids under one account, but
  it captures who really merged.
- **Merge routing when agents share an id.** The self-merge check keys on the
  `Author-Agent` *string*. Two genuinely distinct sessions that share an id
  (e.g. two `claude` sessions) will trip the guard as if it were a self-merge.
  That is **intentional, conservative** behavior — the fix is to route the merge
  to a **distinct** id (`codex` or `human:@jadenfix`), not to loosen the check.
  So: a PR authored under `claude` should be merged by `codex` or the human, and
  vice-versa. Record the intended merger in the ledger.

---

## 8. Quick checklist (pin this)

Before you start:
- [ ] Read this file, `docs/coordination.md`, and the relevant `final.md` section.
- [ ] Claimed a slice with a disjoint write scope in the ledger.

Before you request merge:
- [ ] Different agent reviewed it (DPR verdict recorded).
- [ ] Routing trailer filled; `Merged-By` ≠ `Author-Agent`.
- [ ] Contract checklist satisfied; `final.md` not weakened.

After merge:
- [ ] Ledger row set to `merged`; branch deleted.
