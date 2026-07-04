# Source Matrix Audit

Status: living verification artifact for `final.md` section 27.

Last checked: 2026-07-03 from the macOS repository checkout.

## Purpose

`final.md` uses the Source Matrix as its evidentiary backbone. This document
tracks whether those sources resolve, what each source should be used for, and
where reviewers should be careful not to over-claim. It supplements `final.md`
without replacing or editing it.

Source-quality order for beaterOS design decisions:

1. Primary papers, official specifications, and official vendor docs.
2. Mature OS/security references and reproducible benchmarks.
3. Company blogs for direction and product constraints.
4. Speculation only when clearly marked as speculation.

## 2026-07-03 Reachability Pass

Command shape:

```sh
sed -n '3137,3208p' final.md | rg -o 'https?://[^ )]+' |
  while read url; do
    curl -L --max-time 20 --connect-timeout 10 \
      -A 'beaterOS-source-audit/0.1' -o /dev/null -s -w '%{http_code}' "$url"
  done
```

Result:

- 51 URLs were extracted from `final.md` section 27.
- 44 returned HTTP 200 from the command-line reachability pass.
- 7 returned HTTP 403 to `curl` but resolved in a browser-style fetch:
  - `https://wiki.osdev.org/Main_Page`
  - `https://wiki.osdev.org/Beginner_Mistakes`
  - `https://wiki.osdev.org/Required_Knowledge`
  - `https://openai.com/index/introducing-operator/`
  - `https://openai.com/index/computer-using-agent/`
  - `https://openai.com/index/chatgpt-agent-system-card/`
  - `https://www.intel.com/content/www/us/en/developer/tools/trust-domain-extensions/overview.html`

Interpretation: no `final.md` section 27 URL was found dead in this pass, but
HTTP reachability is not the same as scholarly endorsement, benchmark quality,
or implementation correctness.

## Issue #16 arXiv Audit

Issue #16 flagged five structurally unusual 2026 arXiv IDs. Each resolved to the
named paper on arXiv as of this audit.

| final.md entry | Verified source metadata | beaterOS use | Caveat |
| --- | --- | --- | --- |
| Agent Operating Systems (AOS): Integrating Agentic Control Planes into, and Beyond, Traditional Operating Systems, `2606.01508` | arXiv page exists; submitted 2026-06-01; title matches; subjects include cs.CR and cs.AI | Agent-control-plane framing, AOS responsibilities, OS abstraction gaps | Treat as a recent preprint, not settled architecture |
| Qualixar OS: A Universal Operating System for AI Agent Orchestration, `2604.06392` | arXiv page exists; submitted 2026-04-07; title matches; subjects include cs.AI, cs.MA, cs.SE | Multi-agent orchestration, routing, compatibility, dashboard/product surface | Application-layer orchestration, not a kernel trust model |
| Toward Securing AI Agents Like Operating Systems, `2605.14932` | arXiv page exists; submitted 2026-05-14; title matches; subject cs.CR | Security analogy between agents and OSs; privilege separation; resource mediation | Under-submission preprint; verify claims against beaterOS threat model |
| CaMeLs Can Use Computers Too: System-level Security for Computer Use Agents, `2601.09923` | arXiv page exists; submitted 2026-01-14; revised 2026-06-04; title matches | CUA isolation, planning/execution separation, prompt-injection resistance | Security/utility tradeoffs need scenario tests before adoption |
| OSWorld2.0: Benchmarking Computer Use Agents on Long-Horizon Real-World Tasks, `2606.29537` | arXiv page exists; submitted 2026-06-28; title matches | Long-horizon computer-use benchmark pressure: hidden state, many tool calls, verification debt | Benchmark results must not become product requirements without local replication |

Conclusion for issue #16: the flagged IDs are valid and should stay in the
Source Matrix. They should be marked as recent research inputs, not canonical
requirements.

## Design Implications

- Agent OS papers justify an agent control plane, but beaterOS should avoid
  replacing kernel mechanisms before it has a safe local runtime and sandbox lane.
- CUA and browser-use sources reinforce the need for manifest-before-execution,
  action-bound approvals, trusted sandbox resolution, and screenshots/DOM/a11y
  receipts.
- OSWorld-style benchmarks show that long-horizon agents fail on hidden state,
  verification, cross-source reasoning, and repeated tool calls; release gates
  need trace-property checks rather than final-answer-only scores.
- OS architecture sources reinforce minimal TCB, explicit interfaces,
  capability-style authority, and measured close-to-metal performance.
- Protocol and vendor sources are useful for compatibility direction, but
  authority must still be enforced outside model output and outside tool claims.
- Crypto, TEE, stablecoin, and post-quantum sources are input material for
  threat-model and schema agility; they are not reasons to invent cryptography.

## Maintenance Rules

When adding a source:

- Record title, URL, source type, date checked, and why it matters.
- Prefer official links over secondary commentary.
- For arXiv papers, verify that the ID resolves to the stated title and record
  submission or revision date.
- Mark preprints, marketing posts, and speculative sources explicitly.
- Do not cite a source as evidence for claims it does not make.
- Add or update a doc-health check when citations become part of a release gate.
