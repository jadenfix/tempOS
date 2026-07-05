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
| `action execute` | Run a scoped shell action through the **sandbox execution lane**: canonicalize + confine `--cwd`, admit, and (only if `Allowed`) execute confined and journal a filesystem-diff `CapabilityReceipt`. |
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

## Sandbox execution lane

`action execute` is the mediation point that actually **runs** an admitted
action, confined and fail-closed, via the `beater-os-sandbox` crate (final.md §8,
§10.6, §13.8). Where `action propose` only journals a policy decision,
`action execute` turns an `Allowed` decision into a real OS process and emits a
filesystem-diff receipt of its observed side effects. The flow, all fail-closed:

1. **Canonicalize + confine.** The sandbox resolves `--cwd` with realpath
   (`std::fs::canonicalize`, following every symlink) and rejects it if it
   escapes the confinement prefix. The confinement prefix is derived from the
   **named grants' authority** (their `path_prefixes` plus any concrete file-path
   resource), never from an agent-supplied flag, so an agent cannot widen its own
   sandbox. The canonical path becomes the kernel-derived `resolved_target`
   (§7.4). A symlink escape or a grant with no filesystem confinement aborts
   before anything is journaled or executed.
2. **Admit.** An `ActionManifest` (`action_kind = execute`, kernel-derived
   `resolved_target`) is admitted by `PolicyEngine` — no admission logic in the
   CLI. `ActionProposed` and `PolicyDecided` are journaled.
3. **Execute only if `Allowed`.** The confined child runs with a **scrubbed
   environment** (`env_clear` + a minimal `PATH` — no inherited secrets), a
   **wall-clock timeout**, and **capped** stdout/stderr. Otherwise the decision
   is printed and nothing runs.
4. **Filesystem-diff receipt.** The confined directory is snapshotted (path ->
   SHA-256) before and after; the created/modified/deleted diff is the observed
   side effect. A `CapabilityReceipt` (input digest = command+args, output digest
   = captured stdout, side-effect summary = the diff) is journaled as
   `ReceiptAppended` and persisted — reusing the same store path as
   `receipt record`, so no receipt can exist without a prior `Allowed` decision.

```console
# Execute grant confined to a canonical work directory.
$ beaterosctl grant issue --session demo --resource-kind file_path \
    --resource-id '*' --actions execute --path-prefix /abs/work
issued grant <grant-id>

$ beaterosctl action execute --session demo --tool shell \
    --command sh --arg -c --arg 'printf hi > out.txt' \
    --cwd /abs/work --grants <grant-id> --side-effects local_write
action <id>
  decision:    Allowed
  resolved:    /abs/work
  execution:   ok
  fs-diff:     created=["out.txt"] modified=[] deleted=[]
  receipt:     <receipt-id> hash=<...>
```

Number of sandbox lanes is a compromise beaterOS accepts (§26); this is the
single portable local lane. Network isolation, seccomp/AppArmor/cgroups, and
container/VM lanes (§10.6, §13.8) are explicit future targets, not silently
assumed here.

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
(pause/resume/cancel) or tool registration. Those are separate backlog slices
(`session-runtime`, `tool-registry`). The scoped shell **sandbox lane** is now
implemented (`action execute`, via `beater-os-sandbox`); richer lanes (network,
container/VM, browser) remain future targets. When the `beater-osd` runtime
lands, `beaterosctl` should delegate session mutation to it rather than
journaling transitions directly.
