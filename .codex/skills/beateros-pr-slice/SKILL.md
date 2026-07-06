---
name: beateros-pr-slice
description: Ship scoped beaterOS issue fixes through clean, reviewed, green pull requests. Use when turning a GitHub issue, review follow-up, security hardening task, systems contract change, CLI/runtime fix, audit gap, or performance-sensitive beaterOS change into an implementation branch and PR.
---

# beaterOS PR Slice

## Workflow

1. Establish current state from evidence.
   - Read `AGENTS.md`; for systems/security/performance changes, also use `beateros-systems-engineering`.
   - Inspect `git status --short --branch`, open PRs, and the target issue.
   - Treat `main`, current GitHub state, and the working tree as authoritative.

2. Choose a minimal independent slice.
   - Prefer one issue or one coherent follow-up per branch.
   - Avoid stacking on open PRs unless the dependency is intentional and stated.
   - Preserve `final.md`; add clarifying docs or implementation artifacts around it unless explicitly asked to edit it.
   - Name the invariant: authority boundary, audit evidence, resource bound, hot path, or macOS behavior being fixed.

3. Use subagents for sidecar work.
   - Delegate read-only issue/code investigation or disjoint implementation scopes.
   - Do not delegate the immediate critical-path edit if waiting would block progress.
   - Close completed agents after consuming their findings.

4. Implement conservatively.
   - Follow existing Rust/API patterns; prefer Rust when tradeoffs are close.
   - Keep unsafe/C/assembly out unless the boundary requires it.
   - Keep hot paths bounded: no unbounded queues, clones, syscalls, retries, spend, or allocations without a reason.
   - Security-sensitive changes fail closed and produce replayable evidence.

5. Verify before publishing.
   - Run the narrowest focused test first.
   - Then run relevant package tests, formatting, clippy, and whitespace checks.
   - For publishable branches, run:

```sh
TMPDIR=/private/tmp python3 scripts/local-e2e.py
```

6. Publish intentionally.
   - Stage only files in the slice.
   - Commit with a terse imperative message.
   - Push `codex/<description>` or the existing scoped branch.
   - Open a ready PR only when local verification is green; include issue links, root cause, impact, and validation.
   - Wait for GitHub checks and leave the PR non-draft only when checks pass.

## Governance

- Do not merge your own PR. Repo governance requires non-author review/merge.
- Keep PRs reviewable and scoped; avoid unrelated refactors.
- If unrelated dirty work exists, ignore it or explicitly isolate with path staging/stashing.
- If a PR overlaps another open PR, state the expected rebase/merge order.

## Completion Check

Before calling a slice done, prove:

- The issue's concrete acceptance criteria are covered by code, docs, tests, or explicit deferral.
- The repo builds and tests on macOS.
- The PR is open, linked to the issue when appropriate, and GitHub checks are green.
- Remaining work is tracked in an issue or clearly named in the PR body.
