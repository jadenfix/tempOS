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

## Loopback HTTP control plane

`beater-osd-http serve` exposes a token-gated loopback control plane for local
agent runners. It enforces loopback `Host`/`Origin` checks and an unguessable
bearer token from `--token-file`; routes other than `GET /healthz` require that
token.

The worker-loop route is:

```text
POST /v1/sessions/<session-id>/actions/execute-local-shell-loop
```

The body names a local shell command recipe and an explicit `max_actions` bound.
The route does not accept lease ids, receipt ids, or capability grants. It
delegates to `AgentRuntime::run_local_shell_worker_loop`, which repeatedly
selects daemon-claimable admitted actions, claims fresh execution leases,
executes through the gateway/sandbox, and appends exact receipts. The HTTP
surface caps `max_actions` at 16 and `timeout_secs` at 30 seconds per action so
one protocol request cannot monopolize the synchronous control-plane server.

Example body:

```json
{
  "tool": "shell",
  "tool_digest": "<sha256>",
  "command": "sh",
  "args": ["-c", "printf ok > out.txt"],
  "cwd": "/workspace/project",
  "side_effects": ["local_write"],
  "timeout_secs": 30,
  "max_actions": 8
}
```

The response is the serialized runtime worker-loop outcome, including
`stop_reason`, executed action reports, and the final projection summary.
Projection summaries keep the compatibility fields `open_execution_leases` and
`recovery_blocked`, and also include `open_execution_lease_statuses`,
`live_open_execution_leases`, `live_open_execution_lease_ids`,
`expired_recoverable_execution_leases`, and
`expired_recoverable_execution_lease_ids`. A scheduler should wait or inspect
the owning worker while live leases remain, and should use explicit
`outcome_unknown` recovery only for expired-recoverable leases.

Callers may explicitly opt into supervised recovery before the loop runs:

```json
{
  "tool": "shell",
  "tool_digest": "<sha256>",
  "command": "sh",
  "args": ["-c", "printf ok > out.txt"],
  "cwd": "/workspace/project",
  "side_effects": ["local_write"],
  "timeout_secs": 30,
  "max_actions": 8,
  "recover_expired_leases": true,
  "max_recoveries": 1,
  "recovery_reason": "runner observed expired worker lease",
  "reconciled_by": "agent:local-runner",
  "recovery_evidence_refs": ["runner://pid/1234/dead"]
}
```

Supervised recovery is opt-in only. When `recover_expired_leases` is absent or
false, the route returns the plain worker-loop outcome and refuses recovery
fields. When true, `max_recoveries` must be between 1 and 16. The route returns
the serialized supervised-cycle outcome: `recoveries`, `worker_loop`, and final
`projection`. Live leases are not recovered; they return a successful supervised
outcome with zero recoveries and `worker_loop.stop_reason = "recovery_blocked"`.
Recovered leases are reconciled only as `outcome_unknown`; the reconciled action
is closed and not retried.

Schedulers can preflight the same state through:

```text
GET /v1/sessions/<session-id>
```

The response includes `pending_allowed_action_ids`,
`runnable_pending_action_ids`, `open_execution_lease_statuses`,
`live_open_execution_leases`, `live_open_execution_lease_ids`,
`expired_recoverable_execution_leases`, and
`expired_recoverable_execution_lease_ids`. Each open lease status includes the
`action_id`, `lease_id`, `expires_at`, and `status` (`live_open` or
`expired_recoverable`).

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
| `session resume` | Resume a paused session through the daemon lifecycle state machine, unless the journal still has an unresolved open execution lease. |
| `session cancel` | Cancel a running or paused session through the daemon lifecycle state machine. |
| `grant issue` | Issue a scoped `CapabilityGrant` and journal `CapabilityGranted`. |
| `grant revoke` | Resolve an issued grant's stored revocation handle and journal `CapabilityRevoked`. |
| `payment-mandate issue` | Issue bounded economic authority through `Store::issue_payment_mandate`; receipt requirement is always `required`. |
| `payment-spend propose` | Derive a typed payment `ActionManifest` from an issued mandate and normalized rail envelope evidence, then run daemon admission. |
| `action propose` | Journal an `ActionProposed`, run policy admission, journal `PolicyDecided`. |
| `action execute` | Run a scoped shell action through the **tool gateway lane**: resolve a registered local shell tool, canonicalize + confine `--cwd`, admit, and (only if `Allowed`) execute confined and journal a filesystem-diff `CapabilityReceipt`. |
| `execution-lease reconcile` | Reconcile an expired unresolved execution lease as `outcome_unknown`, closing the runtime recovery blocker without creating a receipt or proving success/no-side-effect. |
| `simulation record` | Record passed, action-bound simulation evidence for the latest `NeedsSimulation` decision. |
| `receipt record` | Record a `CapabilityReceipt` for an **admitted** action (fails closed otherwise). |
| `journal verify` | Verify the journal and receipt hash chains and causality. |
| `trace show` | Render the full trace: session, grants, actions, decisions, receipts. |
| `trace export` | Export a full core-wire trace/action bundle for one live session. |

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

Pre-admitted worker actions may also supply an explicit `--inputs-digest` and
`--max-wall-ms` so a later scheduler claim can bind a finite execution lease to
the admitted input digest. The claim request separately compares pinned tool
version/digest against the daemon registry.

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

## Payment operator flow

Payment actions require two independent authorities:

- a spend `CapabilityGrant`, which authorizes the agent to perform a spend verb
  on a payment rail; and
- a `PaymentMandate`, which authorizes the economics: rail, asset, amount
  ceiling, counterparty policy, purpose, idempotency key, adapter/envelope
  allowlists, approval threshold, and receipt requirement.

The CLI never asks the operator to retype mandate-owned authority fields during
spend proposal. `payment-spend propose` derives rail, asset, purpose, and
payment idempotency key from the stored mandate and only accepts the concrete
rail attempt fields: amount, adapter id/version, counterparty reference and
binding hash, envelope format/hash, and optional envelope expiry.

```console
$ beaterosctl session create --session pay-demo --agent agent:runtime \
    --created-by human:owner --workspace ws-payments \
    --goal "pay approved vendor" --initial-capability-id grant-spend
created session pay-demo

$ beaterosctl grant issue --session pay-demo --grant-id grant-spend \
    --resource-kind payment_rail --resource-id stablecoin:x402 \
    --actions spend --max-risk critical --max-data-class financial
issued grant grant-spend

$ beaterosctl payment-mandate issue --session pay-demo \
    --mandate mandate-spend --rail stablecoin:x402 --asset USDC \
    --max-minor-units 100 --counterparty-policy prefix:vendor: \
    --purpose "vendor payment" --expires-at 2030-01-01T00:00:00Z \
    --approval-threshold-minor-units 100 \
    --payment-idempotency-key pay-once \
    --adapter x402 --envelope-format x402-payment-v1
issued payment mandate mandate-spend

$ beaterosctl payment-spend propose --session pay-demo --action-id act-pay \
    --mandate mandate-spend --grants grant-spend --amount-minor-units 100 \
    --adapter-id x402 --adapter-version v1 --counterparty-ref vendor:runtime \
    --counterparty-binding-hash 2222222222222222222222222222222222222222222222222222222222222222 \
    --envelope-format x402-payment-v1 \
    --envelope-hash 3333333333333333333333333333333333333333333333333333333333333333
payment action act-pay
  decision:   NeedsSimulation

$ beaterosctl simulation record --session pay-demo --action act-pay
recorded simulation sim-act-pay for action act-pay

$ beaterosctl payment-spend propose --session pay-demo --action-id act-pay ...
payment action act-pay
  decision:   Allowed

$ beaterosctl receipt record --session pay-demo --action act-pay \
    --status submitted --rail-receipt-hash 6666666666666666666666666666666666666666666666666666666666666666 \
    --settlement-status submitted --external-id rail:receipt:runtime
recorded receipt <receipt-id> for action act-pay

$ beaterosctl journal verify --session pay-demo
journal OK
```

Fail-closed payment rules:

- `payment-mandate issue` writes only through `Store::issue_payment_mandate`;
  raw `PaymentMandateIssued` events are refused by the daemon public append API.
- Mandates require non-empty rail, asset, counterparty policy, purpose,
  idempotency key, positive amount ceiling, future expiry, and explicit
  adapter/envelope-format allowlists. CLI-issued mandates always set
  `receipt_requirement = required`, and approval thresholds must not exceed the
  mandate ceiling.
- `payment-spend propose` refuses missing mandate, expired mandate, zero amount,
  disallowed adapter/envelope, malformed lowercase 32-byte hashes, disallowed
  counterparty, and expired envelope before admission. Core policy rechecks the
  same normalized intent against the mandate.
- `simulation record` derives the manifest hash from the stored action and
  defaults the scenario id from the latest `NeedsSimulation` decision.
- `receipt record` derives typed payment receipt evidence from the stored
  manifest and mandate. Generic `--external-id` values are supplemental and can
  never satisfy a required payment receipt without `--rail-receipt-hash` and
  `--settlement-status`. `settled` receipts require `--settled-at`; non-settled
  receipts reject `--settled-at`.

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
   including durable revocation overlays and replay-derived session budget
   usage. Local-shell manifests debit one requested tool call and the sandbox
   timeout as requested wall-clock budget; finite `AgentSession.budget`
   `max_tool_calls` / `max_wall_ms` limits deny before execution when the
   journaled receipts already exhaust the session envelope. No admission logic
   lives in the CLI. `ActionProposed` and `PolicyDecided` are journaled.
4. **Acquire a durable execution lease, then execute only if `Allowed`.**
   `Allowed` is not executable authority by itself. Before spawning the child,
   the gateway asks `beater-osd` to append an `ExecutionLeaseIssued` event bound
   to the exact decision id, manifest hash, tool ref, resolved target, required
   grants, and requested runtime budget. The lease expiry covers the sandbox
   timeout plus a bounded two-second gateway overhead grace for durable append,
   preflight, and receipt bookkeeping; it does not widen the sandbox's own
   timeout. The daemon stamps the lease issuance time after acquiring the
   session lock, so lock wait cannot backdate executable authority. Journal
   appends are flushed and synced
   before the daemon returns from the append path, so the lease is durable before
   the gateway starts the side-effecting process. The same daemon session lock is
   held through lease append, sandbox execution, and receipt append, so lifecycle
   transitions cannot interleave. If a process fails after the lease is written
   but before a receipt exists, replay sees an open lease and refuses to run the
   action again until `execution-lease reconcile` handles it explicitly. The
   daemon also refuses new action admission and paused-session resume while any
   unresolved open execution lease remains, because that state means the side
   effect outcome is unknown and must not be hidden behind model memory or a
   synthetic success receipt. Reconciliation requires the lease to be expired,
   records `outcome_unknown`, and does not make the action executable again. The
   confined child runs under macOS Seatbelt with
   filesystem writes limited to granted prefixes, network denied by default, and
   process execution limited to the resolved entry executable. It also gets an
   **explicit environment allowlist** (`env_clear` + a CLI-owned safe `PATH`
   baseline, plus any repeated `--env BEATER_NAME=VALUE`; no inherited
   secrets), a **wall-clock timeout**, and **capped** stdout/stderr. Invalid env
   names, duplicate names, unsafe names outside `BEATER_*`, or `PATH` overrides
   fail closed before the action is journaled. Otherwise the decision is printed
   and nothing runs.
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

If the daemon crashes or a callback fails after `ExecutionLeaseIssued` is
durable but before `ReceiptAppended`, operators must not record a synthetic
receipt. After the lease expires and the side-effect outcome has been reviewed
as unknown, reconcile the lease explicitly:

```console
$ beaterosctl execution-lease reconcile --session demo \
    --action <action-id> --lease-id <lease-id> \
    --resolution outcome_unknown \
    --reconciliation-id reconcile-<lease-id> \
    --reason "operator inspected workspace and external systems; outcome remains unknown" \
    --evidence incident:ticket-123
reconciled execution lease <lease-id>
  reconciliation: reconcile-<lease-id>
  resolution:     outcome_unknown
```

This closes the runtime recovery blocker for future admission or session resume,
but the trace still shows an unresolved action outcome rather than a receipt.

Use repeated `--env BEATER_NAME=VALUE` for the rare action that needs a process
variable. The sandbox crate itself does not add implicit variables; the CLI
passes the safe `PATH` baseline explicitly so ordinary system tools still
resolve. Environment names are intentionally restricted because macOS Seatbelt
executes through `/usr/bin/sandbox-exec`, which sees the same environment before
the confined target starts.

Number of sandbox lanes is a compromise beaterOS accepts (§26); this is the
macOS local lane. Linux `seccomp`/Landlock/cgroups and container/VM lanes
(§10.6, §13.8) are explicit future targets, not silently assumed here.

## Live trace bundle export

`trace export --session <id>` emits the full core-wire replay artifact for one
session: session, grants, payment mandates, approvals, simulations, manifests,
policy decisions, receipts, and journal records. `--bundle-id <id>` overrides
the default `<session>:<journal-root-hash>`; `--description <text>` adds an
optional human note.

The export is read-only and holds the daemon session lock once, so projection
arrays and journal records come from the same verified journal snapshot. It
does not read `receipts.jsonl`; receipt state remains projected from
`ReceiptAppended` journal events. It preserves daemon-native journal and receipt
hashes and the core serde wire shape, including `null` where `null` carries
contract meaning such as an explicit unbounded grant ceiling. This is a full
replay/debug artifact, not a redaction-safe incident handoff: goals, paths,
summaries, external IDs, and payment metadata may be present. Use
`beateros-audit bundle` when a digest-only redaction-safe bundle is required.

For offline full-trace verification, pipe or save the export and run
`beateros-audit verify-trace`:

```console
$ beaterosctl trace export --session demo | beateros-audit verify-trace -
$ beateros-audit verify-trace --expected-root <journal-root-hash> trace-bundle.json
```

`verify-trace` treats the embedded `journal` section as authoritative, derives
projection arrays from that journal, compares them to the exported arrays, and
verifies the receipt chain from `ReceiptAppended` events. It is not an import,
resume, restore, or live replay path.

## Invariants preserved

- **No ambient authority.** An action with no matching grant is never admitted.
- **Session lifecycle gates authority.** New grants, payment mandates, and
  action admissions are refused unless the daemon-projected session status is
  `Running`.
- **No payment without a mandate.** Spend actions proposed through the payment
  CLI carry a typed `PaymentIntent` and must be covered by an issued mandate.
- **Typed payment receipts.** Required payment receipt evidence is recorded as
  structured `PaymentReceiptEvidence`; external IDs alone do not satisfy the
  receipt requirement.
- **Policy outside the model.** Admission is computed by `PolicyEngine`, which
  has no model dependency.
- **Journal before side effects.** `ActionProposed`, `PolicyDecided`, and for
  real gateway execution `ExecutionLeaseIssued` are written by the daemon before
  any side-effecting process can spawn or any receipt can exist.
- **Receipts after side effects.** A receipt can only be recorded for an action
  with a prior `Allowed` decision; gateway-executed receipts additionally
  consume the open execution lease. `beater-osd::Store::append_receipt` refuses
  otherwise through the core journal causality verifier.
- **Tamper-evident.** `journal verify` recomputes every hash and rejects any
  reordered or edited record.

## Local daemon API

For read-only session projection, `beater-osd serve` exposes the first
long-running local surface over the same daemon-owned store. It is intentionally
small: `/healthz` is the only unauthenticated route, while `/v1/sessions` and
`/v1/sessions/<id>` require a bearer token loaded from `--token-file`.
Session projection responses include recovery fields:
`pending_allowed_actions`, `pending_allowed_action_ids`,
`runnable_pending_actions`, `runnable_pending_action_ids`, `execution_leases`,
`open_execution_leases`, `open_execution_lease_ids`,
`execution_reconciliations`, `recovery_blocked`, `admission_blocked`, and
`admission_blockers`, so operators and schedulers can see when runtime work is
ready to dispatch, paused by session state, or blocked by an unresolved
execution lease without requesting a full trace export.

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
- `POST /v1/sessions/<id>/actions/<action_id>/claims`
- `POST /v1/sessions/<id>/actions/<action_id>/claims/<lease_id>/complete`
- `POST /v1/runtime/bundles`

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
store. Successful execution responses include an `evidence` object that binds
the admitted manifest hash, proposal and decision journal records, exact
`tool_id@version#digest`, receipt journal record, receipt root hash, and final
journal root hash. Successful responses also include the durable execution
lease id, lease journal sequence, and lease journal hash between the decision
and receipt records. Denied actions remain receipt-free and do not include
side-effect execution evidence.

The hosted `beater-os-runtime` crate now exposes a typed runtime bundle for
future agent workers and service adapters. A bundle can create a session, issue
bounded grants, admit ordered runtime steps, and return replay evidence without
giving callers a direct store mutation API. The bundle path is smoke-gated by
`scripts/run-beater-os-runtime-smoke.py --json`.

`POST /v1/runtime/bundles` is the daemon HTTP service boundary for the same
contract. The route accepts a bounded JSON `RuntimeBundle`, runs it through
`AgentRuntime::run_bundle` over the daemon store, and returns the full
`RuntimeBundleOutcome` including step replay evidence and the same recovery
and scheduler summary counts (`pending_allowed_actions`,
`runnable_pending_actions`, `open_execution_leases`,
`open_execution_lease_ids`, `execution_reconciliations`, `recovery_blocked`,
`admission_blocked`, `admission_blockers`) in its projection summary. It does
not import trace exports or replay historical journals into live state: each
submitted bundle is new runtime work that must pass the current daemon
admission path. Observation receipts remain limited to no-side-effect steps and
are bound to the dedicated `tool:beater-os-runtime` observation tool; bundle
submissions cannot mint receipts for gateway tools such as `shell`. Real
process/tool side effects still belong behind `execute-local-shell`, the
gateway, sandbox confinement, and receipt path. A token-authorized bundle may
bootstrap a new session and its declared root capability, so this route is an
authenticated authority-minting surface for new runtime work, not a read-only
replay or import API.

`POST /v1/sessions/<id>/actions/execute-local-shell` also acts as the first
dispatch bridge for scheduler-visible runnable work. When the request supplies
an `action_id` that is already present in the session journal, the route refuses
unless the latest decision for that action is `Allowed` and the action has no
receipt, execution lease, or outcome-unknown reconciliation. If eligible, the
same request body must reconstruct the original action manifest; the daemon
rejects mismatches before issuing the execution lease. Successful responses
include `dispatch: "runnable_pending_action"` for this path and
`dispatch: "new_action"` for fresh action submission. The durable execution
lease remains the atomic worker claim, so competing workers cannot both execute
the same pending action.

Schedulers that split claim from execution use
`POST /v1/sessions/<id>/actions/<action_id>/claims`. The request carries only
compare-and-set fields (`expected_manifest_hash`, `expected_decision_id`, and
`expected_tool_version`, `expected_tool_digest`, and an optional `lease_id`);
it does not carry target, grant, or budget authority. The daemon rebuilds the
latest admitted manifest and policy decision from the journal, resolves the
pinned tool through the daemon-owned registry, derives the execution lease from
that state, fsyncs `ExecutionLeaseIssued`, and returns the lease id, manifest
hash, decision id, pinned `tool_id@version#digest`, target, required grants,
budget, lease journal sequence/hash, expiration, and journal root hash with
`201 Created`.

Workers complete claimed work with
`POST /v1/sessions/<id>/actions/<action_id>/claims/<lease_id>/complete` and a
receipt-shaped body. The receipt `action_id` must match the route action id,
and the store accepts it only if `<lease_id>` is the exact currently open lease
for that action. Generic receipt append paths refuse to complete open execution
leases, so scheduler workers cannot bypass the claim token by posting a receipt
that only matches the action id.

## Scope boundary

`action execute` now routes through the gateway and a daemon-owned durable local
tool registry file. Richer registry operations (signed remote publishers,
operator review queues, network/container/VM/browser tool lanes) remain future
targets. The typed `beater-os-runtime` crate now centralizes the reusable agent
loop over `beater-osd`: session bootstrap, grant issuance, sequential admission,
no-side-effect observation receipts, runtime bundles, and deterministic step
replay evidence anchored to journal and receipt-chain hashes. The current CLI
still opens the `beater-osd` store in-process for write operations, but
`beater-osd-http` now provides the first token-gated daemon execution route for
the same local shell gateway lane.
