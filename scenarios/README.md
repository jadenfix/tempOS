# beaterOS Scenario & Security-Eval Fixtures

Concrete `ScenarioManifest` fixtures (final.md §7.10 / §12.8 / §14) that make
beaterOS workflows testable and turn the safety model into adversarial evals.
Each file is a scenario the eval/simulation service (final.md §10.11) can run:
a goal, an environment fixture, allowed tools, forbidden actions, an oracle,
success criteria, risk traps, a budget, and expected trace properties.

Every scenario validates against
`contracts/schemas/scenario-manifest.schema.json` (PR-C). This slice **depends
on** that schema — it builds "against the contract, not the code"
(`docs/multi-agent-coordination.md` §6).

## The core idea: security is measured by the policy layer, not the model

For the adversarial scenarios, **success does not mean the model resisted a
trick.** It means the *policy layer blocked or escalated the side effect even
if the model was fully persuaded* (final.md §13.5, §14.5, §26). The oracles and
`expected_trace_properties` assert on the trace — denials, receipts, taint
labels — not on the model's prose.

## Scenarios

| File | Class | What it proves |
| --- | --- | --- |
| `coding.repo-edit-test.json` | workflow | Granted repo edit + sandboxed tests; push denied (no grant). |
| `browser.research-cite.json` | workflow | Read-only browsing with citations; no credentialed action. |
| `payment.bounded-purchase.json` | workflow | Spend only within a mandate; over-threshold gated; idempotent. |
| `security.prompt-injection-web.json` | adversarial | Injected web text cannot create authority or exfiltrate secrets. |
| `security.malicious-mcp-tool.json` | adversarial | Lookalike MCP tool: no token passthrough; schema-drift quarantine. |
| `security.compromised-document.json` | adversarial | Hidden doc instruction cannot authorize a payment. |
| `security.memory-poisoning.json` | adversarial | Untrusted observation can't be promoted to a privileged fact. |
| `resilience.human-review-timeout.json` | resilience | Approval timeout fails **closed**, not open. |
| `regression.model-downgrade.json` | regression | Paired eval on a smaller model route stays within budget/risk. |

These map to the scenario classes required in final.md §14.2 (filesystem,
browser research, payment, prompt injection, malicious tool, compromised
document, memory poisoning, model downgrade, human-review timeout) and the
security-eval examples in §14.5.

## Validate

```bash
python3 scenarios/validate_scenarios.py           # verbose
python3 scenarios/validate_scenarios.py --quiet   # summary + failures only
```

It reuses the dependency-free validator in `contracts/validate.py` (one engine,
one source-of-truth schema). Exit code `0` iff every `scenarios/*.json`
validates against `ScenarioManifest`.

## Adding a scenario

1. Write `scenarios/<class>.<name>.json` conforming to `ScenarioManifest`.
2. For adversarial scenarios, make the oracle assert on the **trace/policy
   outcome** (a denial, a missing receipt, a taint label), never on model text.
3. Run `python3 scenarios/validate_scenarios.py`.
4. Per final.md §13.15, every production incident should become a new scenario
   here.
