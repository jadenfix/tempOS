# beaterOS Reviewer Guide

Every reviewer — human or agent — has **full authority over every PR**,
regardless of who authored it. Authorship confers no special power once a
change is proposed. A reviewer may read, run, critique, request changes,
approve, and (if a non-author) merge **any** PR in this repository.

This guide exists so that a reviewer who did not write the code can still
fully understand and operate the change. If a PR cannot be reviewed by
someone who did not write it, the PR is too opaque — request changes.

## 1. Reviewer Authority (equal power, by design)

- Any reviewer may approve or request changes on any PR.
- Any non-author reviewer may merge an approved PR.
- Any reviewer may check out the branch, run the tests, and reproduce the
  results independently — PRs must make this possible (see §3).
- No reviewer is bound by the author's framing; verify against `final.md`,
  not against the PR's self-description.

The only hard limit: **you may not approve or merge your own PR** (protocol
§2, enforced by `.github/workflows/pr-governance.yml`).

## 2. What Every Review Must Check

1. **Scope discipline** — the diff only writes files inside the branch's
   claimed write scope (`docs/ownership-registry.md`). Out-of-scope edits →
   request changes.
2. **`final.md` fidelity** — the change implements the named `final.md`
   sections it claims, and weakens none of the §26 non-negotiables.
3. **No ambient authority** — nothing grants blanket file/network/shell/
   payment access; authority is explicit, scoped, and revocable.
4. **Contracts stay typed and versioned** — new/changed contracts are
   typed, documented, and covered by tests or examples.
5. **Side effects have manifests + receipts** — any side-effecting path is
   represented, not implicit.
6. **Policy is outside the model** — capability/policy checks are
   deterministic code, never model output.
7. **Tests exist and are runnable by a non-author** — see §3.
8. **Review routing** — the PR names a non-author reviewer and a non-author
   merger; the checklist is honest.

## 3. Reproduce It Yourself

A change is only reviewed when you have observed it work, not when you have
read that it works. For each PR type:

- **Rust slices:** `cargo fmt --all -- --check`,
  `cargo test --workspace --locked`,
  `cargo clippy --workspace --all-targets --locked -- -D warnings`.
- **Contract schemas / scenarios:** `python3 contracts/validate.py` (or the
  script named in the PR) — it must validate every example/fixture and
  exit non-zero on a deliberately broken input.
- **Governance / scripts:** run the script's own test file, e.g.
  `python3 scripts/test_check_review_routing.py`.

If you cannot reproduce the claimed result, request changes and say what
you observed.

## 4. How To Approve And Merge (non-author)

1. Complete the checks in §2–§3.
2. Approve with a review that states *what you verified*, not just "LGTM".
3. Merge (squash preferred) — only if you are not the author.
4. Update the PR's row in `docs/ownership-registry.md` to `merged`.
5. Delete the branch.
6. If the merge unblocks another agent's slice, comment on that slice's
   tracking PR/issue to close the communication loop.

## 5. How To Request Changes

- Be specific and reproducible: file, line, the invariant at risk, and the
  concrete failing input or scenario.
- Prefer one consolidated review over a stream of comments.
- Tie every requested change to `final.md` or this protocol, so the author
  (and any future reviewer) understands *why*, not just *what*.

## 6. Reviewing Another Agent's Work (the loop in practice)

Agents cannot chat; the PR thread is the channel. When you review a peer
agent's PR:

- State your identity and that you are a non-author reviewer.
- Summarize what you verified and how (commands + observed output).
- If you approve, say a non-author may now merge; if you merge, say so.
- If you block, give the exact remediation so the author agent can act
  without a second round-trip.
