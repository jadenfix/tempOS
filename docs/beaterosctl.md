# beaterosctl

`beaterosctl` is the operator CLI for the hosted beaterOS agent kernel. It is
the human/operator surface over the `beater-osd` runtime store: it persists
sessions to an append-only, hash-chained journal on disk and exposes the
kernel's deterministic policy admission as inspectable commands.

It implements these slices of `final.md`:

- §24 Minimum Viable beaterOS, items 1, 2, 4, 5, 7, 8, 10.
- §25 What To Build First Later, items 2 (local append-only journal), 7 (CLI),
  and 9 (trace viewer).

The CLI adds **no authority of its own**. Capability grants, action admission,
and receipt appends go through `beater-osd::Store`, which owns the single-writer
runtime boundary and delegates deterministic admission to
`beater-os-core::PolicyEngine`, outside of any model output. It cannot broaden a
grant, and it fails closed on missing or invalid input.

## Store layout

The store root is chosen by, in order of precedence: the `--home` flag, the
`BEATEROS_HOME` environment variable, or `./.beateros`.

```
<home>/sessions/<session_id>/journal.jsonl    # hash-chained event journal, including receipts
```

The journal is strictly append-only. A reload reconstructs the in-memory journal
and receipt chains from `beater-os-core` and re-verifies them; nothing is ever
rewritten in place. Older stores may contain a `receipts.jsonl` sidecar, but the
daemon runtime treats `ReceiptAppended` journal events as the authoritative
receipt ledger.

## Commands

| Command | Purpose |
| --- | --- |
| `session create` | Create a goal-directed session and journal `SessionCreated`; by default this declares `<session>-root-grant` as the first root capability id. |
| `session list` | List sessions in the store. |
| `session show` | Summarize one session's grants, actions, decisions, receipts. |
| `session pause` | Pause a running session through the daemon lifecycle state machine. |
| `session resume` | Resume a paused session through the daemon lifecycle state machine. |
| `session cancel` | Cancel a running or paused session through the daemon lifecycle state machine. |
| `grant issue` | Issue a scoped `CapabilityGrant` and journal `CapabilityGranted`. |
| `grant revoke` | Resolve an issued grant's stored revocation handle and journal `CapabilityRevoked`. |
| `action propose` | Journal an `ActionProposed`, run policy admission, journal `PolicyDecided`. |
| `action execute` | Run a scoped shell action through the **tool gateway lane**: resolve a registered local shell tool, canonicalize + confine `--cwd`, admit, and (only if `Allowed`) execute confined and journal a filesystem-diff `CapabilityReceipt`. |
| `receipt record` | Record a `CapabilityReceipt` for an **admitted** action (fails closed otherwise). |
| `journal verify` | Verify the journal and receipt hash chains and causality. |
| `trace show` | Render the full trace: session, grants, actions, decisions, receipts. |

Enum-valued flags use the snake_case names from `beater-os-core`
(e.g. `file_path`, `read`, `write`, `execute`, `low`, `medium`, `high`,
`critical`, `local_write`, `code`). Run `beaterosctl help` for the full flag
list.

`grant issue` generates a revocation handle by default and prints it with the
grant. Operators can provide a stable handle with `--revocation-handle <h>`.
For `file_path` grants, `--resource-id` may be omitted when at least one
`--path-prefix` is present; the CLI stores the selector as `*` so the canonical
path-prefix constraint, not an exact directory selector, carries the authority.
Concrete file grants can still pass `--resource-id <path>`, and non-file grants
must continue to name `--resource-id` explicitly. `grant revoke --grant-id <id>
--reason <text>` resolves the issued grant's stored handle and appends
`CapabilityRevoked`; callers cannot inject an arbitrary fake handle. `action
propose` and `action execute` evaluate against the durable journal-projected
revocation registry. They still accept repeatable `--revoked-handle <h>` flags
as an external monotonic epoch overlay for replay or operator-supplied evidence.

## Worked MVP flow

This is the `final.md` §24 MVP proof: a repo task where the agent can read and
write only granted paths, and cannot escape its grant.

```console
$ beaterosctl session create --session demo --agent coder-1 \
    --workspace repo --goal "fix the failing test"
created session demo

# Scoped file grant: any file, but only under /workspace/repo. Omitting
# --resource-id with a file_path --path-prefix stores selector '*' so the prefix
# is the authority boundary. The first grant uses the session's default root
# capability id unless --grant-id is supplied.
$ beaterosctl grant issue --session demo --resource-kind file_path \
    --actions read,write --path-prefix /workspace/repo
issued grant <grant-id>

# A raw proposal with a path-prefix grant must include a resolved target supplied
# by a trusted mediator. `action execute` derives this itself before admission.
$ beaterosctl action propose --session demo --tool fs.write --kind write \
    --target-kind file_path --target /workspace/repo/src/lib.rs \
    --resolved-target /workspace/repo/src/lib.rs \
    --grants <grant-id> --action-id a1
action a1
  decision:    Allowed

# An out-of-scope canonical path is refused by policy — not by the model.
$ beaterosctl action propose --session demo --tool fs.write --kind write \
    --target-kind file_path --target /etc/hosts --resolved-target /etc/hosts \
    --grants <grant-id> --action-id a2
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

## Tool gateway execution lane

`action execute` is the mediation point that actually **runs** an admitted
action, confined and fail-closed, via `beater-os-tool-gateway`,
`beater-os-tool-registry`, and `beater-os-sandbox` (final.md §8, §10.6, §13.8,
§10.14). Where `action propose` only journals a policy decision, `action
execute` turns an `Allowed` decision into a real OS process and emits a
filesystem-diff receipt of its observed side effects. The flow, all fail-closed:

1. **Resolve a pinned local shell tool.** The CLI asks the daemon store to
   persist the exact `--tool` + version + local shell digest in
   `<home>/tool-registry.json`, under a bounded registry lock. Operators can
   pass `--tool-digest <sha256>` to pin the expected executable+args+environment
   digest explicitly; otherwise the CLI computes the digest for compatibility.
   If `--tool-version` is omitted, the CLI derives a stable `local-<digest>`
   version so different command digests do not collide under `shell@local`.
   The gateway reloads the daemon-owned `ToolRegistry`, checks the workspace
   allowlist, recomputes the digest, and resolves the pinned tool before any
   action is admitted.
2. **Canonicalize + confine.** The sandbox resolves `--cwd` with realpath
   (`std::fs::canonicalize`, following every symlink) and rejects it if it
   escapes the confinement prefix. The confinement prefix is derived from the
   **named grants' authority** (their `path_prefixes` plus any concrete file-path
   resource), never from an agent-supplied flag, so an agent cannot widen its own
   sandbox. The canonical path becomes the kernel-derived `resolved_target`
   (§7.4). A symlink escape or a grant with no filesystem confinement aborts
   before anything is journaled or executed.
3. **Admit.** The gateway derives the `ActionManifest` (`action_kind = execute`,
   kernel-derived `resolved_target`) and asks the daemon store to admit it,
   including durable revocation overlays. No admission logic lives in the CLI.
   `ActionProposed` and `PolicyDecided` are journaled.
4. **Execute only if `Allowed`.** The confined child runs under macOS Seatbelt
   with filesystem writes limited to granted prefixes, network denied by
   default, and process execution limited to the resolved entry executable. It
   also gets an **explicit environment allowlist** (`env_clear` + a CLI-owned
   safe `PATH` baseline, plus any repeated `--env BEATER_NAME=VALUE`; no
   inherited secrets), a **wall-clock timeout**, and **capped** stdout/stderr.
   Invalid env names, duplicate names, unsafe names outside `BEATER_*`, or
   `PATH` overrides fail closed before the action is journaled. Otherwise the
   decision is printed and nothing runs.
5. **Filesystem-diff receipt.** The confined directory is snapshotted (path ->
   SHA-256) before and after; the created/modified/deleted diff is the observed
   side effect. A `CapabilityReceipt` (input digest = command+args+environment,
   output digest = captured stdout, side-effect summary = the diff) is journaled as
   `ReceiptAppended` and persisted — reusing the same store path as
   `receipt record`, so no receipt can exist without a prior `Allowed` decision.

```console
# Execute grant confined to a canonical work directory.
$ beaterosctl grant issue --session demo --resource-kind file_path \
    --actions execute --path-prefix /abs/work
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

Use repeated `--env BEATER_NAME=VALUE` for the rare action that needs a process
variable. The sandbox crate itself does not add implicit variables; the CLI
passes the safe `PATH` baseline explicitly so ordinary system tools still
resolve. Environment names are intentionally restricted because macOS Seatbelt
executes through `/usr/bin/sandbox-exec`, which sees the same environment before
the confined target starts.

Number of sandbox lanes is a compromise beaterOS accepts (§26); this is the
macOS local lane. Linux `seccomp`/Landlock/cgroups and container/VM lanes
(§10.6, §13.8) are explicit future targets, not silently assumed here.

## Invariants preserved

- **No ambient authority.** An action with no matching grant is never admitted.
- **Session lifecycle gates authority.** New grants, payment mandates, and
  action admissions are refused unless the daemon-projected session status is
  `Running`.
- **Policy outside the model.** Admission is computed by `PolicyEngine`, which
  has no model dependency.
- **Journal before side effects.** `ActionProposed` and `PolicyDecided` are
  written by `beater-osd::Store::admit_action` before any receipt can exist.
- **Receipts after side effects.** A receipt can only be recorded for an action
  with a prior `Allowed` decision; `beater-osd::Store::append_receipt` refuses
  otherwise through the core journal causality verifier.
- **Tamper-evident.** `journal verify` recomputes every hash and rejects any
  reordered or edited record.

## Local daemon API

For read-only session projection, `beater-osd serve` exposes the first
long-running local surface over the same daemon-owned store. It is intentionally
small: `/healthz` is the only unauthenticated route, while `/v1/sessions` and
`/v1/sessions/<id>` require a bearer token loaded from `--token-file`.

```console
$ printf '%s\n' 'replace-with-operator-token' > .beateros/token
$ beater-osd serve --root .beateros --token-file .beateros/token \
    --bind 127.0.0.1:8787
```

The listener refuses non-loopback bind addresses, caps request headers/bodies,
uses short socket timeouts, and applies loopback `Host`/`Origin` checks before
serving token-gated routes. Loopback and browser boundary checks are not treated
as authentication; the token remains the authority gate for control-plane data.

For daemon-owned execution, `beater-osd-http serve` is the service-plane control
binary that sits above the store and the tool gateway. It preserves the same
loopback, `Host`/`Origin`, and bearer-token boundary, and adds:

- `POST /v1/sessions/<id>/actions/execute-local-shell`

That route accepts a bounded JSON request (`command`, `cwd`, `grants`, optional
`args`, `env`, `tool`, `tool_version`, `tool_digest`, risk/data/taint metadata,
server-capped timeouts/output, and receipt/action ids), persists the exact
local-shell tool digest in the daemon-owned registry, then calls the gateway
path. Before digesting the executable, the HTTP layer rejects slash-containing
commands and verifies `cwd` is inside the named grants' filesystem confinement,
so a token-bearing client cannot use digest computation as an unconstrained
filesystem read. The HTTP handler never executes a process directly: the gateway
still derives the manifest, asks `beater-osd::Store` for admission, executes
only when policy returns `Allowed`, and appends the receipt through the daemon
store.

## Scope boundary

`action execute` now routes through the gateway and a daemon-owned durable local
tool registry file. Richer registry operations (signed remote publishers,
operator review queues, network/container/VM/browser tool lanes) remain future
targets. The typed `beater-os-runtime` crate now centralizes the reusable agent
loop over `beater-osd`: session bootstrap, grant issuance, sequential admission,
no-side-effect observation receipts, and deterministic step replay evidence
anchored to journal and receipt-chain hashes. The current CLI still opens the
`beater-osd` store in-process for write operations, but `beater-osd-http` now
provides the first token-gated daemon execution route for the same local shell
gateway lane.
