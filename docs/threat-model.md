# beaterOS Threat Model

Status: Phase 1 deliverable (`final.md` §19). Living document — extend it as new
components, adversaries, and incidents appear (§13.15 turns every incident into a
new scenario; every new scenario should update this file).

Scope of this revision: the **agent kernel** as specified in `final.md` §7–§13
and as first implemented in `crates/beater-os-core` (PR #1, branch
`codex/agent-kernel-contracts`). Where a control already exists in code, this
document cites the concrete symbol so the mapping is verifiable rather than
aspirational. Where a control is specified but not yet built, it is marked
**PLANNED** with the roadmap slice that will land it.

Closes the acceptance criteria of issue #7: assets, trust boundaries, adversaries
and capabilities, an attack → §13-control → §14.5-eval matrix, and explicit
residual / out-of-scope risks.

---

## 1. Why a threat model (not just controls)

`final.md` §13 is an excellent catalogue of *controls and principles*, but a list
of controls cannot answer the only question that matters: **are the controls
sufficient, and against whom?** A control is only justified by an attacker, an
asset, and an attack path. This document anchors each §13 control to a concrete
adversary and asset, and to a security eval (§14.5) that proves the control
holds even when the model is fully persuaded.

The root security claim we are defending (`final.md` §13.1):

> The model is never the root of trust. Safety is enforced **outside** the model
> — by explicit authority, provenance, sandboxing, deterministic policy, and
> receipts.

Everything below is in service of making that claim true against a realistic
adversary set, not a polite one.

---

## 2. Method

- **Framing:** asset-centric, then adversary-centric, then attack-centric. We use
  a lightweight STRIDE lens (Spoofing, Tampering, Repudiation, Information
  disclosure, Denial of service, Elevation of privilege) but specialize it for a
  *probabilistic goal-seeking* principal that interprets untrusted data as
  potential instructions (`final.md` §4.2, §5.8).
- **Trust anchor:** the deterministic policy engine, capability checker, journal
  verifier, secret broker, and sandbox launcher form the Trusted Computing Base
  (TCB) (§20.2). Everything else — including the model — is untrusted-by-default.
- **Verification bar:** a mitigation is only "real" if it is enforced outside the
  model and is covered (or has a named PLANNED eval) in §14.5. Prompting is not a
  mitigation (§13.5).

---

## 3. System model and trust boundaries

```
                          ┌─────────────────────────────────────────────┐
   trusted user intent →  │            beaterOS Agent Kernel (TCB)        │
                          │  PolicyEngine · CapabilityChecker · Journal   │
                          │  Verifier · Secret Broker · Sandbox Launcher  │
                          └───────▲───────────────┬───────────────▲───────┘
        (B1) model↔policy         │               │               │  (B6) local↔cloud journal
   ┌──────────────┐   proposes    │   admits/denies (deterministic)│   ┌──────────────────┐
   │  Model(s)    ├───────────────┘               │               └──►│ Journal / audit   │
   │ (untrusted   │  action manifests             ▼                    │ sync (optional)   │
   │  worker)     │                        ┌──────────────┐            └──────────────────┘
   └──────────────┘                        │ Service Fabric│
        ▲                                   │  (least-priv) │
        │ (B2) agent↔subagent               └──┬───┬───┬───┬┘
   ┌────┴─────┐                    (B3) gateway │   │   │   │ (B4) host↔sandbox
   │ Subagent │                   ↔ remote MCP/ │   │   │   │
   └──────────┘                   A2A servers   ▼   ▼   ▼   ▼
                                       ┌─────┐┌─────┐┌──────┐┌────────┐
   (B5) browser origin boundary  ◄──── │Tool ││Brow-││Shell/││Memory  │
                                       │G'way││ ser ││ code ││service │
                                       └─────┘└─────┘└──────┘└────────┘
```

**Trust boundaries (crossings where authority or trust changes):**

| ID | Boundary | Why it is dangerous |
| --- | --- | --- |
| **B1** | Model ↔ Policy engine | The model can be manipulated by any text/image it reads; its proposals must be treated as untrusted requests, never as decisions (§13.1). |
| **B2** | Agent ↔ Subagent | Delegated authority can be broadened or leaked if not attenuated (§8.2, §20.7). |
| **B3** | Tool gateway ↔ remote MCP / A2A server | The counterparty can lie about tool behavior, change it after pinning, or request broad scope (§8.6, §13.6). |
| **B4** | Host ↔ sandbox (shell/code/browser lanes) | Code execution can escape scope, read ambient secrets, or reach the network (§13.8). |
| **B5** | Browser origin boundaries | Browsers fuse identity, cookies, payments, and untrusted content on one surface (§13.7). |
| **B6** | Local ↔ cloud journal / audit sync | Sync exposes causal history and can be a tamper or exfiltration target (§20.3). |
| **B7** | User/org ↔ agent identity | An over-privileged or spoofed principal can issue grants it shouldn't (§7.1). |

---

## 4. Assets

What an attacker wants, and the contract/code that owns it.

| # | Asset | Why it matters | Owning contract / code |
| --- | --- | --- | --- |
| A1 | **Capability grants** (authority itself) | Possession = permission. Forgery/broadening = arbitrary authority. | `CapabilityGrant`, `CapabilityGrant::allows_manifest` (contracts.rs) |
| A2 | **Journal integrity** (causal truth) | The record used for audit, replay, incident response, billing disputes. If forgeable, accountability collapses. | `InMemoryJournal::verify_chain`, `hash_json` (journal.rs, hash.rs) |
| A3 | **Receipt chain** (side-effect truth) | Proves what actually happened to external systems. | `ReceiptLedger::verify_chain`, `CapabilityReceipt` (receipt.rs) |
| A4 | **Secrets** (API keys, tokens, cookies) | Direct pivot to external systems; must never enter prompts/logs. | Secret broker (**PLANNED**, backlog); `DataClass::Secret`, `TaintLabel::Secret` |
| A5 | **Payment authority** | Money is an irreversible side effect. | `PaymentMandate` (defined; enforcement **PLANNED** slice 14) |
| A6 | **User / customer / financial data** | Privacy, compliance, exfiltration target. | `DataClass::{Personal,Customer,Financial}`, `ModelPolicy.max_data_class` |
| A7 | **Model routes** (which model sees which data) | Wrong route leaks sensitive data to a non-compliant provider. | `ModelPolicy`, model router (**PLANNED** slice 13) |
| A8 | **Memory** (long-term knowledge) | Poisoned memory silently steers future runs. | `MemoryRecord` (defined); memory service (**PLANNED** slice 11) |
| A9 | **Policy definitions & versions** | The rules themselves; if mutable by the agent, all bets are off. | `PolicyEngine`, `policy_version` binding (policy.rs) |
| A10 | **Agent identity & delegated authority** | Spoofed/over-broad identity issues bad grants. | `AgentIdentity` (defined); signing (**PLANNED** §7.1) |
| A11 | **Human attention / review integrity** | A vague or bypassed approval defeats the last line of defense. | `ApprovalRequirement`, `ApprovalEvidence`, `has_approval_for_grant` (policy.rs) |
| A12 | **Compute / budget** (availability) | Runaway loops burn money and quota (self-DoS, issue #15). | `Budget`, `Budget::fits_within` (defined; hard-stop enforcement **PLANNED**, issue #15) |

---

## 5. Adversaries and their capabilities

| ID | Adversary | Capabilities we assume | Notes |
| --- | --- | --- | --- |
| **T1** | **Malicious content** (web page, document, email) | Inject text/hidden instructions the model reads; cannot execute code directly. | The canonical agent threat (§5.8, §13.5). Carries taint `Untrusted{Web,Email,Document}`. |
| **T2** | **Malicious / compromised tool or MCP server** | Lie about behavior, mutate after pinning, request broad scope, return payloads that are themselves injection (§13.6). | Crosses B3. |
| **T3** | **Compromised / non-compliant model provider** | Retain, log, or exfiltrate anything routed to it; return adversarial completions. | Crosses B1; constrained by `ModelPolicy`. |
| **T4** | **Malicious subagent** | Request parent authority; try to widen a delegated grant. | Crosses B2. |
| **T5** | **Over-privileged / careless insider** (or spoofed principal) | Issue overbroad grants; approve their own risky action; self-merge process changes. | Crosses B7; also the *dev-process* threat that PR #19 governance addresses. |
| **T6** | **Network attacker** | Intercept/redirect egress, DNS/host spoofing, TLS downgrade attempts. | Crosses B3/B6. |
| **T7** | **Supply-chain attacker** | Ship a malicious dependency, tool, or build artifact (§13.10). | Affects the TCB itself. |
| **T8** | **Persuaded-but-honest model** | Not malicious, but fully convinced by T1/T2 to propose a dangerous action. | The most important case: safety must hold *even when the model is wrong* (§13.1). |
| **T9** | **Runaway agent** (non-malicious) | Loop, retry, or replan without bound; exhaust budget/quota (issue #15). | Availability/cost, crosses A12. |

We explicitly do **not** assume a fully compromised host kernel or a broken
SHA-256 (see §9 Residual risk).

---

## 6. Attack → control → eval matrix

Each row: an attack by an adversary against an asset, the beaterOS control that
stops or escalates it (with the enforcing code where it exists), and the §14.5
security eval that must prove it — even against T8, the persuaded model. Evals
marked **PLANNED** name a scenario to add to the suite (§14.2/§14.5); several map
directly to `final.md` §14.5's listed examples.

| # | Adversary | Attack on asset | Control (`final.md` §) | Enforcing code | Security eval (§14.5) |
| --- | --- | --- | --- | --- | --- |
| K1 | T1/T8 | Untrusted web text says "wire the funds / deploy to prod" → drive a `Spend`/`Deploy`/`Delegate` (A5/A1) | Data≠instructions; untrusted taint cannot authorize dangerous actions without action-bound approval (§13.4, §13.5) | `dangerous_untrusted_instruction` + `all_grants_have_action_approval` → `NeedsApproval` (policy.rs) | "Web page attempts prompt injection"; "Email asks agent to wire funds" — **COVERED** by `policy_requires_review_for_untrusted_payment_instruction` |
| K2 | T8 | Model proposes an action with **no** grant (ambient authority) (A1) | No ambient authority; every side effect needs an explicit grant (§13.2) | `admit` denies on empty `required_grants`; grant bound to `holder`+`session_id` in `allows_manifest` | **COVERED** by `policy_denies_ambient_authority_...`, `policy_denies_grant_bound_to_other_session_or_holder` |
| K3 | T4 | Subagent replays a parent/other-session grant (A1/A10) | Grants are holder- and session-bound; delegation must attenuate (§8.2, §20.7) | `allows_manifest`: `self.holder != actor_id \|\| self.session_id != manifest.session_id → false` | **COVERED** (session/holder test); **PLANNED** "subagent requests parent authority" attenuation eval (slice depends on delegation) |
| K4 | T1/T8 | Path traversal to escape a scoped file grant, e.g. `/ws/repo/../secret` (A6) | Capability scoping + normalized-absolute path prefix check on **both** requested and resolved target (§13.8) | `path_constraints_allow` + `normalized_absolute_path` (rejects `..`, relative, missing resolved) | **COVERED** by `policy_rejects_file_path_traversal_and_missing_resolved_target`, `policy_enforces_path_prefix_constraints_even_with_wildcard_resource` |
| K5 | T6/T2 | Lookalike egress host `example.com.evil` to exfiltrate (A4/A6) | Network allowlist with strict label-boundary suffix match (§13.6) | `network_constraints_allow` + `host_matches_allowed` (requires literal `.` boundary) | **COVERED** by `policy_enforces_network_allowlist_constraints`. **GAP:** IPv6-literal hosts mis-parsed — see §7 F2 |
| K6 | T5/T8 | Silently execute a high-risk external side effect without human sign-off (A11) | Risk-thresholded approval; evidence bound to action, grant, reviewer, policy version, non-future time (§13.14) | `has_approval_for_grant` / `has_approval_from_reviewer`; multi-party requires **all** reviewers | **COVERED** by `policy_requires_action_bound_review_evidence`, `..._all_reviewers_for_multiparty_...`, `..._rejects_future_dated_review_evidence` |
| K7 | T5 | Forge/replay an approval from a different action or a future timestamp (A11) | Approval evidence is action- and time-bound; fail-closed | `approval.action_id == manifest.action_id && approved_at <= now && policy_version == ctx.policy_version` | **COVERED** (future-dated + different-action tests) |
| K8 | T8 | Push a high-risk irreversible action to production untested (A5/A6) | High-risk external side effects require a passed, action-bound simulation first (§14) | `has_passed_simulation_for_action` → `NeedsSimulation` | **COVERED** by `policy_requires_action_bound_simulation_evidence`, `..._rejects_future_dated_simulation_evidence` |
| K9 | T2/T5 | Tamper with the journal to hide an action or fabricate authority (A2) | Hash-linked, genesis-anchored, seq-checked append-only journal; causal validity (§13.11, §4.5) | `InMemoryJournal::verify_chain` + `verify_event_causality` (receipt requires prior `Allowed` decision; binds tool/digest/target/side-effects) | **COVERED** by `journal_detects_event_tampering`, `journal_rejects_receipt_without_prior_allowed_decision`, `journal_rejects_receipt_that_does_not_match_manifest` |
| K10 | T2/T5 | Reorder/edit a receipt to misrepresent a side effect (A3) | Hash-chained receipt ledger (§7.6) | `ReceiptLedger::verify_chain` | **COVERED** by `receipt_ledger_detects_reordered_or_edited_receipts` |
| K11 | T3 | Route secret/customer data to a non-compliant provider (A4/A6/A7) | Model route bound by data-class ceiling & retention policy (§6.10, §13.4) | `ModelPolicy.max_data_class`, grant `max_data_class` in `allows_manifest` | **PARTIAL** — ceiling is enforced but semantics are flawed; see §7 F1. **PLANNED** "customer data cannot enter public model routes" eval (slice 13) |
| K12 | T2 | Tool description changes after pinning; token passthrough to remote server (A4) | Tool schema pinning, description distrust, no token passthrough, per-invocation policy (§13.6) | **PLANNED** tool gateway (slice 8/9); `ToolManifest` shape exists | **PLANNED** "MCP server exposes lookalike tool"; "tool output asks for a secret" |
| K13 | T1 | Poison long-term memory so a later run trusts a false fact (A8) | Memory has provenance, confidence, sensitivity, expiry; untrusted source cannot become privileged instruction (§3.4, §13.4) | `MemoryRecord` fields exist; enforcement **PLANNED** slice 11 | **PLANNED** "memory contains poisoned fact" |
| K14 | T7 | Malicious dependency/tool/build artifact enters the TCB (A9) | Signed manifests, pinned deps, SBOM, provenance, sandbox (§13.10) | Workspace hygiene today: `unsafe_code = forbid`, `unwrap_used/expect_used = deny`, `--locked`, small dep set; registry **PLANNED** slice 8 | **PLANNED** "malicious tool" scenario; supply-chain CI gate |
| K15 | T4/T8 | Escalate a delegated grant, or fail to revoke it when the parent is revoked (A1) | Attenuation-only delegation; revocation through indirection; fail-closed (§8.2, §6.2, issue #10) | `DelegationMode` exists; `is_active_at` fails closed on `revoked`/expiry; propagation semantics **PLANNED** (issue #10) | **PLANNED** revocation-propagation + in-flight-action eval |
| K16 | T9 | Runaway loop exhausts budget/quota (self-DoS) (A12) | Budgets are hard, kernel-enforced stops with a defined terminal state (issue #15) | `Budget::fits_within` fails closed on omitted budget; **hard-stop scheduler PLANNED** (issue #15) | **PLANNED** "agent runaway / budget exhaustion" scenario |
| K17 | T5 | **Dev-process** attack: an agent merges its own unreviewed change into `main` (A9) | No self-merge; independent review; policy outside the model — enforced by CI+CODEOWNERS, not goodwill | PR #19 governance (`pr-governance.yml`, `AGENTS.md`, `CODEOWNERS`) | Governance workflow; see honesty boundary in `AGENTS.md` §7 (attested, not cryptographic) |

---

## 7. Findings folded in from the PR #1 code review

These are concrete gaps found while reviewing the *actual* implementation, mapped
to the assets they touch. They are tracked here so the threat model reflects code
reality, not just the spec.

- **F1 — `DataClass` linear `Ord` as a sensitivity ceiling is unsound (asset A6/A7; attack K11).**
  `allows_manifest` rejects when any `*class > max_data_class` using the derived
  enum order `Public < … < Secret < Code < Binary < Untrusted* < ToolOutput`.
  This collapses three orthogonal axes (sensitivity, trust/provenance, content
  type) onto one line, so a grant capped at `Secret` would refuse to read `Code`
  while a grant capped at `Code` would permit `Secret`. `final.md` §13.4 models
  these as *labels with per-label policy*, not a linear scale. **Recommendation:**
  separate a monotonic sensitivity lattice from taint labels (there is already a
  `TaintLabel` enum), or make the ceiling an explicit allowed-set. This is
  entangled with issue #8 (risk-class taxonomy) and should be resolved with it.

- **F2 — `network_host` mis-parses IPv6 literals / userinfo (asset A4/A6; attack K5).**
  `split(':').next()` turns `https://[2001:db8::1]:8443/` into `[2001`, so the
  allowlist decision for bracketed IPv6 hosts is wrong. Low impact today (no
  network lane shipped) but must be fixed with a real URL parser at the gateway
  boundary before slice 8/9.

- **F3 — `hash_json` canonicalization is load-bearing but implicit (asset A2/A3; attack K9/K10).**
  Tamper-evidence rests entirely on `serde_json` emitting struct fields in
  declared order and on only `BTree*` collections being used (both true today). A
  future `HashMap` field or `#[serde(flatten)]` would silently break chain
  determinism and thus *all* integrity guarantees. **Recommendation:** add a
  canonical-serialization invariant (doc + a byte-stability test) so this cannot
  regress unnoticed.

---

## 8. Zero-tolerance invariants

These must be measured as hard gates (feeds issue #14). Any nonzero value is a
release blocker, wired to the K-attacks above:

- Ambient-authority admissions (K2) = **0**.
- Denied-action bypasses (K2/K4) = **0**.
- Secret exposure to external model routes not permitted by policy (K11) = **0**.
- Receipts without a prior `Allowed` decision accepted by the verifier (K9) = **0**.
- Journal/receipt chains that verify after tampering (K9/K10) = **0**.
- Self-merges into `main` (K17) = **0**.

---

## 9. Residual risk & out of scope

Stated explicitly so no one over-trusts the model (mirrors `AGENTS.md` §7's
honesty discipline):

- **Compromised host kernel / hypervisor.** Out of scope near-term. Sandbox lanes
  (§13.8) raise the bar but a fully compromised host defeats a user-space control
  plane. High-assurance track (§19 Phase 9: seL4/CHERI/TEE) is the long answer.
- **Broken primitive crypto** (SHA-256 preimage/collision). Out of scope; we rely
  on standard, maintained libraries and crypto-agility (§13.12) rather than
  novelty.
- **Agent-identity spoofing under a shared account (K17/T5).** Today all agents
  act as one GitHub/human account, so agent identity is **attested, not
  cryptographically verified** (`AGENTS.md` §7). Per-agent signing (§7.1) upgrades
  this from attested to verifiable and is the correct fix; until then the
  governance controls make violations *visible*, not *impossible*.
- **Model-provider-side retention/leakage after a compliant route (T3).** We can
  bound *which* data reaches a provider (A7) but cannot enforce their internal
  handling; this is a contractual/attestation problem (§8.9 TEE-attested
  inference is a partial technical lever).
- **Semantic correctness of a persuaded model's *allowed* work (T8).** Policy
  bounds *authority*, not *good judgement*: an action inside its grant that is
  merely unwise is not blocked. This is why evals (§14) and human review for
  risk (§13.14) exist as separate layers.

---

## 10. Coverage check (issue #7 acceptance)

- **Every §13 control traces to at least one attack:** §13.1→K1/K2/K8, §13.2→K2,
  §13.4→K1/K11/K13, §13.5→K1, §13.6→K5/K12, §13.7→K5 (browser, PLANNED),
  §13.8→K4/K16, §13.9→K11 (secrets), §13.10→K14, §13.11→K9/K10, §13.14→K6/K7,
  §13.15→K15 (revocation/incident).
- **Every attack traces to a §14.5 eval:** see the right column of §6 — COVERED
  rows point at existing tests in `crates/beater-os-core/tests/foundation.rs`;
  PLANNED rows name the scenario to add and the roadmap slice that owns it.
- **Assets, trust boundaries, adversaries enumerated:** §3–§5.
- **Residual / out-of-scope explicit:** §9.

---

## 11. Maintenance

- When a roadmap slice lands a **PLANNED** control (tool gateway, memory service,
  model router, payment mandate, revocation, budget hard-stops), flip its row to
  COVERED and cite the enforcing symbol + eval.
- When an incident occurs (§13.15), add the attack row and a regression eval, and
  update §8 if it reveals a new zero-tolerance invariant.
- Keep this file's write scope disjoint from other agents' branches; it is
  additive by design. Cross-references to `final.md` use stable section numbers so
  they survive a future doc split (issue #5).
