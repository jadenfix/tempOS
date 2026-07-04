# beaterosctl

`beaterosctl` is the operator CLI and durable local store for the beaterOS
agent kernel. It is the human/operator surface over `beater-os-core`: it
persists sessions to an append-only, hash-chained journal on disk and exposes
the kernel's deterministic policy admission as inspectable commands.

It implements these slices of `final.md`:

- §24 Minimum Viable beaterOS, items 1, 2, 4, 5, 7, 8, 10.
- §25 What To Build First Later, items 2 (local append-only journal), 7 (CLI),
  and 9 (trace viewer).

The CLI adds **no authority of its own**. Every capability check is delegated to
`beater-os-core::PolicyEngine`, outside of any model output. It cannot broaden a
grant, and it fails closed on missing or invalid input.

## Store layout

The store root is chosen by, in order of precedence: the `--home` flag, the
`BEATEROS_HOME` environment variable, or `./.beateros`.

```
<home>/sessions/<session_id>/journal.jsonl    # hash-chained event journal
<home>/sessions/<session_id>/receipts.jsonl   # hash-chained side-effect ledger
```

Both files are strictly append-only. A reload reconstructs the in-memory chains
from `beater-os-core` and re-verifies them; nothing is ever rewritten in place.

## Commands

| Command | Purpose |
| --- | --- |
| `session create` | Create a goal-directed session and journal `SessionCreated`. |
| `session list` | List sessions in the store. |
| `session show` | Summarize one session's grants, actions, decisions, receipts. |
| `grant issue` | Issue a scoped `CapabilityGrant` and journal `CapabilityGranted`. |
| `action propose` | Journal an `ActionProposed`, run policy admission, journal `PolicyDecided`. |
| `receipt record` | Record a `CapabilityReceipt` for an **admitted** action (fails closed otherwise). |
| `journal verify` | Verify the journal and receipt hash chains and causality. |
| `trace show` | Render the full trace: session, grants, actions, decisions, receipts. |

Enum-valued flags use the snake_case names from `beater-os-core`
(e.g. `file_path`, `read`, `write`, `execute`, `low`, `medium`, `high`,
`critical`, `local_write`, `code`). Run `beaterosctl help` for the full flag
list.

## Worked MVP flow

This is the `final.md` §24 MVP proof: a repo task where the agent can read and
write only granted paths, and cannot escape its grant.

```console
$ beaterosctl session create --session demo --agent coder-1 \
    --workspace repo --goal "fix the failing test"
created session demo

# Scoped file grant: any file, but only under /workspace/repo.
$ beaterosctl grant issue --session demo --resource-kind file_path \
    --resource-id '*' --actions read,write --path-prefix /workspace/repo
issued grant <grant-id>

# An in-scope write is admitted by an explicit, active capability grant.
$ beaterosctl action propose --session demo --tool fs.write --kind write \
    --target-kind file_path --target /workspace/repo/src/lib.rs \
    --grants <grant-id> --action-id a1
action a1
  decision:    Allowed

# An out-of-scope write is refused by policy — not by the model.
$ beaterosctl action propose --session demo --tool fs.write --kind write \
    --target-kind file_path --target /etc/hosts --grants <grant-id> --action-id a2
action a2
  decision:    NeedsNarrowedGrant

# Receipts may only be recorded for admitted actions.
$ beaterosctl receipt record --session demo --action a1 --status ok

# The hash chains verify, and the trace explains every side effect.
$ beaterosctl journal verify --session demo
journal OK
$ beaterosctl trace show --session demo
=== beaterOS trace: demo ===
...
```

## Invariants preserved

- **No ambient authority.** An action with no matching grant is never admitted.
- **Policy outside the model.** Admission is computed by `PolicyEngine`, which
  has no model dependency.
- **Journal before side effects.** `ActionProposed` and `PolicyDecided` are
  written before any receipt can exist.
- **Receipts after side effects.** A receipt can only be recorded for an action
  with a prior `Allowed` decision; the store refuses otherwise, mirroring the
  core journal causality verifier.
- **Tamper-evident.** `journal verify` recomputes every hash and rejects any
  reordered or edited record.

## Scope boundary

This crate deliberately does **not** implement session lifecycle transitions
(pause/resume/cancel), sandboxed execution, or tool registration. Those are
separate backlog slices (`session-runtime`, `sandbox-shell-lane`,
`tool-registry`). When the `beater-osd` runtime lands, `beaterosctl` should
delegate session mutation to it rather than journaling transitions directly.
