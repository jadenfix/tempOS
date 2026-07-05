# beaterOS — End-to-End Repo Audit

**This is a point-in-time audit taken on 2026-07-03, when the repo was
planning-only. It is a historical record, not a description of current `main`.
See the Status Update below for what has since landed.**

Original scope (2026-07-03): `README.md` plus the 3,242-line design doc
`final.md`. At that time there was no implementation code; `final.md` was
"research plan and system design plan only."

Because there was no code to exercise, "end-to-end / properly done" was assessed
as: **is the plan complete, internally consistent, source-backed, and structured
well enough to guide the implementation it describes?** The plan is strong as
strategy; the findings below were about closing the loop.

Each finding is tracked as a GitHub issue. This document is the in-repo record of
that audit pass.

## Status Update (2026-07-05) — most findings have landed

Since this audit, the fleet has advanced `main` well past planning-only. `main`
now carries a Rust workspace for core contracts, session lifecycle, durable
single-writer daemon storage, sandbox execution, operator CLI, offline audit,
tool registry, and memory projection (`crates/beater-os-core`,
`crates/beater-os-session`, `crates/beater-osd`, `crates/beater-os-sandbox`,
`crates/beaterosctl`, `crates/beater-os-audit`,
`crates/beater-os-tool-registry`, `crates/beater-os-memory`), contract JSON
Schemas (`contracts/`, `spec/`), a conformance suite + tests,
security/IoT/resilience scenarios, example traces, governance docs, a source
matrix, an open-questions doc, a threat model, and a LICENSE.

Finding-by-finding state on current `main`:

- **Resolved on `main`:** #2 (LICENSE), #4 (glossary), #7 (threat model), #11
  (schemas/`spec/VERSION`), #12 (example traces), #14 (success-metrics-and-gates),
  #15 (budget-enforcement doc), #16/#20 (source matrix + open questions). #8
  (risk floor) is **partly shipped** — see the reconciled
  `docs/design/risk-class.md`. #10 (revocation) is covered by the merged
  `docs/design/revocation-semantics.md` (this lane's duplicate `revocation.md`
  was dropped in favor of it).
- **Still open / lighter:** #3 (README depth), #5 (doc split), #6 (`final.md`
  naming consistency — `final.md` is integrity-locked, so tracked not edited),
  #9 (redaction mechanism — spec in `docs/design/journal-redaction.md`), #17
  (doc-health CI — governance lane). #10's remaining pieces (runtime abort +
  compensation trigger) are already scoped in `revocation-semantics.md` §5.

The original findings list below is preserved verbatim as the historical record.

## Findings

### Repository hygiene / structure
- **No LICENSE** — work is "all rights reserved" by default; blocks reuse/contribution. (#2)
- **README is two lines** — no navigation, status, or scope; all substance is buried in `final.md`. (#3)
- **Phase 0 deliverables missing** — the plan's own §19 Phase 0 promises a glossary, an open-questions list, and a source matrix; the glossary doesn't exist, open questions live only as prose (§20), the source matrix is a static list (§27). Phase 0 fails its own "done" criteria. (#4)
- **Monolithic doc** — 3,242 lines, no table of contents or anchors; hard to navigate, diff, and cross-reference. (#5)

### Internal consistency
- **The "six core contracts" are inconsistent** across §3.7, §12, and §19. `MemoryRecord` is excluded from the Phase 1 core set even though "memory provenance" is a §26 *never-compromise*. The receipt object is named three ways (Side-Effect Receipt / CapabilityReceipt / "receipt"). (#6)

### Design gaps / concerns
- **No explicit threat model** — a named Phase 1 deliverable. §13 is principles and controls, not a threat model (assets, trust boundaries, adversaries, attack→mitigation matrix). (#7)
- **`risk_class` undefined** — load-bearing for policy admission, human review, and simulation, but never enumerated or given an assignment rule. (#8)
- **Append-only journal vs. redaction** — tamper-evident hash-linked journal must also support redaction / right-to-be-forgotten; the reconciling mechanism is asserted but never specified. (#9)
- **Revocation semantics** — behavior for in-flight actions and delegated sub-grants is unspecified, which weakens incident containment (§13.15). (#10)
- **No schema versioning/evolution** — long-lived journals must replay under evolving contracts; no version field or compatibility policy. (#11)
- **No worked end-to-end example** — Phase 1 lists "example traces"; the contracts are never shown composing on one concrete run. (#12)
- **A2A / cross-org identity under-specified** — named as required (§18.4) but no component in §10 and no cross-org delegation/identity model. (#13)
- **Success metrics lack targets** — ~35 metrics (§23) with no baselines, thresholds, or measurement methods, so they can't gate releases as intended. (#14)
- **No runaway/cost-exhaustion enforcement** — budgets are described as resources but hard-stop/terminal-state mechanics for a self-DoS loop are unspecified. (#15)

### Evidence integrity & guardrails
- **Unverifiable citations** — several §27 arXiv IDs are structurally anomalous (future-dated; `2606.29537` has an implausibly high sequence number; "Qualixar OS"). Not confirmed wrong — arXiv is unreachable from the audit environment — flagged for human verification. (#16)
- **No doc-health CI** — add markdown lint + link/citation checker to keep `final.md` and §27 valid over time. (#17)

## Tracking
See issue #18 for the live checklist linking all findings.
