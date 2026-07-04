# beaterOS open questions

A living list of the design questions that are not yet settled. This is a
Phase 0 deliverable (final.md §19) and a coordination aid: if you are building a
slice that touches one of these, record your decision in the PR and update the
relevant row so other agents inherit it.

Source of the questions is `final.md` §20. The "Current lean" column captures the
document's recommendation where it gives one; "Status" tracks whether an
implemented slice has resolved it.

| # | Question (final.md §20) | Current lean | Status |
| --- | --- | --- | --- |
| 1 | Smallest useful capability grammar (action families) | Read, Write, Execute, Navigate, Submit, Communicate, Spend, Deploy, Remember, Delegate, Ask-human | open — codex PR #1 defines an initial `action_kind` set; confirm coverage |
| 2 | What belongs in the TCB | Capability service, policy engine, journal verifier, secret broker, sandbox launcher | open |
| 3 | How much of the journal is local | Local-first with optional encrypted sync + org transparency log | open |
| 4 | How to make human approval not annoying | Risk-based thresholds, good previews, safe batching, learned prefs that never grant new authority silently | open |
| 5 | How to prevent policy sprawl | Small default profiles, tested policy packs, rule explanations, policy diffing, scenario coverage | open |
| 6 | Right memory default | Short-lived working memory; explicit promotion to long-term; provenance required; expiry for sensitive data | open |
| 7 | How agents delegate | Delegation requires attenuation; subagents get explicit scopes; parent stays accountable; subagent traces link to parent | open — `DelegationMode` defined but not yet enforced (see PR #1 review) |
| 8 | Safe default browser | Fresh isolated context per high-risk session; no credentialed browsing unless granted; downloads quarantined; uploads reviewed; purchases gated | open |
| 9 | What counts as a side effect | File/network/API writes, messages, form submits, downloads/uploads, memory writes, payments, cloud changes, ticket/comment creation, sensitive-data model calls | open — align `expected_side_effects` enum with this list |
| 10 | How to avoid becoming another agent framework | Enforce boundaries frameworks treat as conventions: capability checks outside model, journal-before / receipt-after, policy versioning, eval gates, tool registry, memory provenance, human review | ongoing — this is the whole thesis |

## Cross-cutting questions raised during implementation

Append newly discovered questions here so they aren't lost between slices.

- **Risk-floor derivation** — should effective risk be a pure function of
  `action_kind` + `expected_side_effects`, and where does the mapping live so
  policy (not the agent) owns it? (Raised by the PR #1 DPR — agent-declared risk
  must not be able to dodge approval/simulation gates.)
- **Approval/simulation evidence binding** — bind to the manifest content digest
  (`inputs_digest`/`target`) or only to `action_id`? Binding to the digest
  prevents replaying an approval against a mutated manifest.
- **Primary CI substrate** — §1 names Linux as the primary substrate; CI should
  build/test on Linux (in addition to any macOS job) so the main target is gated.
