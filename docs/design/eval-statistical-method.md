# Design Spec: Statistical Eval Gates

Status: design spec closing the measurement-method gap tracked in **issue #50**.
Extends `final.md` §14.9 and `docs/design/success-metrics-and-gates.md`.

Agent evals are experiments over probabilistic systems. A scenario result is not
just pass/fail truth; it is one draw from a distribution shaped by the model,
tools, policy, fixtures, and runtime. Release gates therefore need a statistical
method, not single-run point estimates.

## 1. Gate Model

Each release gate evaluates a scenario suite with a declared gate config:

- `risk_class`: `smoke`, `core`, `irreversible`, or stricter local profile
- `trials_per_scenario`: the maximum repeated trials before stopping
- `reliability_target`: minimum acceptable pass^k for the suite or subgroup
- `minimum_detectable_effect`: smallest regression worth blocking on
- `confidence`: confidence target for paired/regression decisions
- `stopping_rule`: sequential rule for early pass/fail/continue decisions
- `baseline`: previous release, approved model route, or pinned trace bundle

The `ScenarioManifest` remains the reusable task fixture. It describes the goal,
environment, oracle, risk traps, and expected trace properties. It does **not**
own trial count or reliability target because those depend on release risk,
model-upgrade context, and budget. The eval service combines scenario manifests
with gate configs at runtime.

## 2. pass^k Reliability

Report both:

- `pass@1`: single-run scenario success rate, useful for debugging and cost
  estimation
- `pass^k`: probability that all `k` independent trials succeed, the reliability
  number that gates agent workflows

Default k values:

| Gate class | k | Use |
| --- | ---: | --- |
| smoke | 1 | Fast checks for obvious breakage |
| core | 4 | Normal workflows that must be repeatable |
| irreversible | 8 | Spend, deploy, communicate externally, or affect durable state |

The release report must show the per-suite `k`, the observed trial counts, and
which scenarios stopped early.

## 3. Paired Regression Tests

Model, tool, runtime, and policy upgrades compare against a baseline using the
same scenario IDs, fixtures, and oracles. The report is paired by scenario:

- per-scenario success delta
- per-scenario cost delta
- per-scenario p95/p99 latency delta where sampled enough
- trace-property regressions
- clustered standard errors when scenarios share fixtures or scenario packs

A gate fails on a statistically supported regression exceeding the declared
minimum detectable effect, not on a raw negative point delta.

## 4. Sequential Stopping

The eval scheduler may stop before `trials_per_scenario` when evidence is
already decisive:

- pass early when the confidence interval clears the target
- fail early when the confidence interval cannot clear the target
- continue sampling when the result is near the threshold

Sequential stopping is a cost-control rule, not a permission to change the
threshold after seeing results. The gate config records the stopping rule before
the run starts.

## 5. Trace Distributions

Trace metrics are reported as distributions:

- latency: p50, p95, p99, timeout count
- cost: median, p95, maximum, spend per successful task
- tool/model calls: median, p95, maximum
- retries/replans: distribution and cap breaches

Means alone are insufficient for OS gates because tail latency, runaway retries,
and rare side effects define operational risk.

## 6. Evidence

The method is grounded in:

- tau-bench (`arXiv:2406.12045`) for pass^k as a repeated-trial reliability
  metric for agents
- Evan Miller's `arXiv:2411.00640` for error bars, paired comparisons, and
  experimental planning for language-model evals
- Kharitonov et al. SIGIR 2015 for sequential stopping as a cost-control pattern
  in online experiments

## 7. Acceptance

- [x] §14 names k values per risk class, pairing, and sequential stopping.
- [x] §23 reliability metrics include pass^k.
- [x] Scenario repetition and reliability targets are assigned to gate config,
      not duplicated into every `ScenarioManifest`.
- [x] Source matrix records the statistical-method sources and caveats.
