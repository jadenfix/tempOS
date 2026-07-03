# Design Spec: Budget Enforcement & Runaway / Self-DoS Control

Status: design spec. Closes the gap tracked in **issue #15**. Grounded in the
merged `crates/beater-os-core` code; specifies a contract for the kernel/
scheduler lane (codex) to implement. Does not edit `final.md`.

`final.md` treats budgets as resources (§4.1, §6.8, session `budget` §12.1) and
the scheduler as policy-aware (§4.4), but never says what *actually happens*
when a session hits a ceiling, or who stops a runaway reasoning loop. The most
common real-world agent failure is not a clever attacker — it is a loop that
burns tokens, money, and API quota. This spec makes budgets **hard, kernel-
enforced stops with a defined terminal state.**

## 1. Budgets are ceilings enforced outside the model

The merged `Budget` (`contracts/… contracts.rs`) already carries the axes:

| Field | Unit | Guards against |
| --- | --- | --- |
| `max_model_cents` | USD cents | model spend |
| `max_tool_calls` | count | tool-call storms |
| `max_wall_ms` | milliseconds | wall-clock runaway |
| `max_payment_minor_units` | minor units | economic blow-up |

Consistent with `final.md` §13.1 ("the model is never the root of trust"), the
ceiling check must run in the kernel/scheduler, **not** in the model and **not**
as advisory prompt text. A `None` axis means "unbounded on that axis" and must
be an explicit, auditable choice — never a silent default (this mirrors the
fail-closed fix already merged for `GrantConstraints`).

## 2. The SessionMeter

The kernel maintains a monotonic **SessionMeter** per `session_id`, accumulating
the same four axes plus loop counters (§4). It is checked at every admission
point — before each `ActionManifest` is admitted and before each model call —
*and* the projected cost of the pending step is added before the check, so a
single step cannot overshoot the ceiling:

```
if meter.spent(axis) + step.cost(axis) > session.budget.axis:   # any bounded axis
    → deny the step, transition to the terminal state (§3)
```

Because admission is already a deterministic function in the merged core, this
is an extra pre-condition on the same code path — no new trust surface.

## 3. Terminal state on breach (fail closed, not fail open)

On the first breached axis:

1. The pending step is **denied** (never partially executed).
2. The session transitions to a terminal status. Reuse the merged
   `SessionStatus`: `WaitingForApproval` if human top-up/escalation is offered,
   else `Failed`. Never silently continue.
3. A journal event records the breach. The merged `JournalEvent` enum has no
   budget variant; add one, matching the existing shape:

   ```
   JournalEvent::BudgetExhausted {
       session_id: String,
       axis: BudgetAxis,          // model_cents | tool_calls | wall_ms | payment_minor_units
       limit: u64,
       observed: u64,
       at: DateTime<Utc>,
   }
   ```

   Until that lands, the breach can be recorded via the existing
   `IncidentAnnotated` event so causality is never lost.
4. **In-flight actions:** an action whose side effect has already committed runs
   to completion and emits its receipt (receipts must stay truthful); no *new*
   action is admitted. Idempotency keys (`ActionManifest.idempotency_key`)
   prevent a retried-then-cancelled step from double-committing (critical for
   `payment_minor_units`).

## 4. Loop / step guards (the non-budget runaway)

Token/tool budgets do not catch a tight replanning loop that is individually
cheap. Add kernel-enforced structural guards with conservative defaults:

| Guard | Default | Rationale (`final.md`) |
| --- | --- | --- |
| `max_reasoning_steps` | 100 | bounds an unproductive loop |
| `max_tool_retries` (per action) | 3 | §14.4 tool-retry metric |
| `max_replans` (per session) | 10 | §14.4 replanning count |
| `max_subagents` (per session) | 8 | §8.10 small-N principals |

Exceeding a guard is treated identically to a budget breach (§3): deny, journal,
terminal state. Guards live in the scheduler, outside the model.

## 5. Eval scenario (make it testable)

Add a `ScenarioManifest` (validates against `spec/contracts/scenario-manifest.schema.json`):

- **goal:** an agent enters a loop that would exceed `max_tool_calls`.
- **oracle (trace-assertion):** the session stops at the ceiling; a
  `BudgetExhausted` (or `IncidentAnnotated`) event is journaled; **no** receipt
  exists for any action proposed after the ceiling; final status is terminal.
- **risk trap:** a naive implementation keeps going after the ceiling — must
  not. Success is measured on the trace, not on the model choosing to stop.

## 6. `final.md` touch points

Extends, without weakening: §4.1 (budgets as resources), §4.4 (policy-aware
scheduling), §6.8 (economic boundaries), §12.1 (`budget` field), §13.1 (model
not root of trust), §22 (adds the "agent runaway / budget exhaustion" failure
mode that is currently missing), §23.4 (cost metrics — "wasted retry cost").

## 7. Acceptance (from issue #15)

- [x] Budget ceilings specified as hard, kernel-enforced stops with a defined
      terminal state.
- [x] Runaway-loop guards (step / retry / replan / subagent caps) specified.
- [x] A budget-exhaustion eval scenario specified.
