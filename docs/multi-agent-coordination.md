# beaterOS Multi-Agent Coordination Protocol

Status: process contract. This document governs *how* multiple autonomous
agents (and humans) build beaterOS on the same repository at the same time
without colliding, and how they stay in a live communication loop.

It complements — it does not replace — `docs/implementation-backlog.md`
(the *what* to build) and `final.md` (the *why*). Where this document and
the backlog disagree on process, this document wins; where they disagree on
scope, `final.md` wins.

## 0. Why This Exists

Several agents work this repository in parallel (e.g. `codex/*` slices and
`claude/*` slices). Parallel autonomous edits are the fastest way to
destroy each other's work: two agents rewriting `Cargo.toml`, both adding
`.github/workflows/ci.yml`, or both claiming the same slice.

The rules below make parallel work *boring*: disjoint write scopes, a single
source of truth for who-owns-what, mandatory non-author review, and a
merge order that never surprises another agent.

## 1. Roles

- **Author agent** — writes a slice on its own branch and opens a PR.
- **Reviewer agent** — a *different* agent (or human) that reviews the PR
  against `final.md` and this protocol.
- **Merger** — a *different* agent (or human) than the author, who merges
  after an approving non-author review.
- **Coordinator** — any agent may act as coordinator: reconcile the
  ownership registry, resolve claim conflicts, and keep the loop alive.

No role is privileged by *who wrote the code*. Any reviewer has full
authority to review, request changes, approve, and merge **any** PR,
regardless of author. Authorship grants no special power over a change once
it is proposed. See `docs/reviewer-guide.md`.

## 2. The Non-Negotiable Rules

1. **Every change lands through a branch and a PR.** No direct pushes to
   `main`.
2. **No self-merge.** The agent or person who authored a PR must never
   approve or merge it. Merge requires an approving review from a
   non-author. This is enforced in CI by
   `.github/workflows/pr-governance.yml`.
3. **Disjoint write scopes.** A PR only writes files inside the paths its
   slice claims in the ownership registry. Shared files (see §5) require an
   explicit coordination note.
4. **`final.md` is immutable during implementation.** It may be referenced
   and split into docs, but never shortened, weakened, or contradicted.
5. **Claim before you build.** Register a slice in
   `docs/ownership-registry.md` (via its own PR or an existing coordination
   PR) before opening feature PRs for it.
6. **Fail closed.** If two agents may be touching the same file, stop and
   coordinate in a comment before pushing.

## 3. Branch Naming

`<agent-namespace>/<slice-name>[-<suffix>]`

- `codex/*` — reserved for the codex agent's slices.
- `claude/*` — reserved for the claude agent's slices.
- Human contributors use `human/<name>/<slice>`.

An agent must not push to another namespace's branches.

## 4. The Communication Loop

Agents cannot see each other's chat. The repository *is* the shared memory.
The loop has four channels, in order of durability:

1. **Ownership registry** (`docs/ownership-registry.md`) — durable,
   authoritative claims and status. Read it first, every session.
2. **Implementation backlog** (`docs/implementation-backlog.md`) — the
   slice map and dependency graph.
3. **PR bodies + the review checklist** — per-change intent, review
   routing, and test evidence.
4. **PR/issue comments** — live negotiation: claim conflicts, handoffs,
   "I depend on your unmerged branch," review feedback.

### Loop procedure (every agent, every session)

1. **Sync**: fetch `main` and read the ownership registry + open PRs.
2. **Detect collisions**: does any open PR or claimed slice overlap the
   paths you intend to write? If yes → comment and negotiate before coding.
3. **Claim**: add/adjust your rows in the ownership registry.
4. **Build** on your namespace branch, within your claimed paths only.
5. **Announce**: open the PR; if you depend on another agent's *unmerged*
   branch, say so explicitly in the PR body and link it.
6. **Request review** from a non-author (spawn a reviewer agent or request
   a human/other-agent).
7. **On approval**, a non-author merges; update the registry status to
   `merged`.
8. **Close the loop**: if your change unblocks another agent's slice, leave
   a comment on their tracking PR/issue.

## 5. Shared Files (handle with care)

These files are written by more than one agent and are the most likely
merge-conflict points. Touching them requires a coordination note in the PR
body naming every other open PR that also touches them:

- `README.md`
- `Cargo.toml`, `Cargo.lock` (Rust workspace root — owned by the core
  slice; other agents must not edit without coordination)
- `.github/PULL_REQUEST_TEMPLATE.md`
- `.github/workflows/*` (use *distinct filenames per concern*, never edit
  another agent's workflow file)
- `docs/ownership-registry.md` and `docs/implementation-backlog.md` (append
  your rows; do not rewrite others' rows)

Rule of thumb: **add a new file rather than edit a shared one.** Distinct
filenames merge cleanly; edits to the same lines do not.

## 6. Depending On Unmerged Work

Slices often depend on another agent's branch that has not merged yet
(e.g. everything depends on the core-contracts slice). Options, preferred
first:

1. **Wait** for the dependency to merge, then branch from `main`.
2. **Build against the contract, not the code**: depend on the stable
   *serialization contract* (`contracts/schemas/*`) instead of another
   agent's in-flight source. Language-neutral schemas let a Python or TS
   slice proceed while the Rust core is still in review.
3. **Branch from the dependency branch** only as a last resort, and declare
   it loudly in the PR body. Rebase onto `main` once the dependency merges.

## 7. Conflict Resolution

- **Claim conflict** (two agents want the same slice): earliest registry
  claim wins; the later agent picks a disjoint slice or offers to review.
- **Merge conflict**: the *author of the not-yet-merged PR* resolves by
  rebasing onto `main`. Never force-push another agent's branch.
- **Design disagreement**: resolve in the PR thread with reference to
  `final.md` sections. If unresolved, escalate to the human owner via an
  `AskUserQuestion`-style comment; do not merge contested changes.

## 8. Definition Of Done For A Slice

- Code + tests within the claimed paths.
- PR body maps the change to named `final.md` sections.
- Review-routing checklist complete; approving review from a non-author.
- Merged by a non-author.
- Ownership registry status set to `merged`.
- Branch deleted after merge.

## 9. Invariants This Protocol Protects (from `final.md`)

Process exists to defend the product's non-negotiables (`final.md` §26):
no ambient authority, journal before side effects, receipts after side
effects, policy outside the model, memory provenance, eval gates, tool
identity, revocation, human-legible authority, and standard cryptography.
A process change that would weaken any of these is out of scope here.
