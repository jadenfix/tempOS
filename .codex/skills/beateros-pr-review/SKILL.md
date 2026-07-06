---
name: beateros-pr-review
description: Run beaterOS review gates for PRs that change runtime contracts, infra gates, docs, and migration evidence.
---

# beaterOS PR Review

## Overview

Use this skill when a PR claims changes in:

- runtime contracts or policy gates,
- migration frontier or bare-metal readiness configuration,
- repetitive infra/docs/governance artifacts,
- or claims that affect execution safety.

The goal is consistent, non-author review quality and a hard requirement that
runtime-first contracts stay intact.

## Review workflow

1. Read the PR scope and the claimed `final.md` sections.
2. Run the concrete reviewer checklist:
   - `docs/governance/review-checklist.md`
3. Confirm migration frontier status:
   - `docs/architecture-runtime-to-metal-path.md`
   - `docs/engineering/bare-metal-readiness-manifest.json`
   - `docs/engineering/bare-metal-e2e-matrix.json`
4. Validate runtime contracts are still present:
   - `python3 scripts/check-bare-metal-readiness.py --check-host --require-control-plane-lane --require-workload-class policy-admission --require-workload-route policy-admission=portable-control-plane`
   - If a hardware/migration lane changed, require the corresponding phase assertion in matrix cases.
5. Require review evidence for claims:
   - PR type, affected layer, test/eval evidence, and migration impact.
6. Record review outcome in `docs/governance/coordination-ledger.md` and ensure
   reviewer/author are distinct.

## Repetitive items for each infra/docs PR

- Update repository map links if a slice or boundary changed.
- Update architecture/runbook docs when execution order or migration meaning changes.
- Keep PR body aligned with `final.md` section references.
- Ensure no runtime-critical claims are added to architecture without
  corresponding manifest/matrix evidence.

