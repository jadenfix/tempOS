# beaterOS Ownership Registry

Single source of truth for **who is building what, on which branch, writing
which paths, and in what status**. Every agent reads this first (see
`docs/multi-agent-coordination.md` §4) and appends/updates only its own rows.

Do not rewrite another agent's rows. Append your rows; edit only your own
status. Conflicting claims resolve by earliest entry (§7 of the protocol).

Legend for **Status**: `planned` → `in-progress` → `in-review` → `merged`
→ `abandoned`.

## Write-Scope Map (collision avoidance)

Each active branch declares the paths it is allowed to write. Two active
branches must have **disjoint** write scopes unless they file a coordination
note. Shared files are listed in the protocol §5.

| Namespace | Owns paths (write scope) |
| --- | --- |
| `codex/*` | `Cargo.toml`, `Cargo.lock`, `crates/**`, `.github/workflows/ci.yml`, `.github/PULL_REQUEST_TEMPLATE.md`, `docs/implementation-backlog.md`, `.gitignore` |
| `claude/*` | `docs/multi-agent-coordination.md`, `docs/ownership-registry.md`, `docs/reviewer-guide.md`, `.github/workflows/pr-governance.yml`, `scripts/**`, `contracts/**`, `scenarios/**` |
| shared (coordinate first) | `README.md`, `final.md` (read-only), the two `docs/*` coordination files (append-only) |

## Slice Registry

### codex agent (Rust implementation roadmap — from `docs/implementation-backlog.md`)

| Slice | Branch | Scope | Status | PR |
| --- | --- | --- | --- | --- |
| Bootstrap agent kernel contracts | `codex/agent-kernel-contracts` | `crates/beater-os-core` core contracts, journal, receipts, policy admission | in-review | #1 |
| Session runtime | `codex/session-runtime` | beater-osd session lifecycle | planned | — |
| beaterosctl CLI | `codex/beaterosctl-contracts` | CLI for sessions/grants/manifests/journal | planned | — |
| Sandbox shell lane | `codex/sandbox-shell-lane` | safe local execution lane, fs diff receipts | planned | — |
| MVP coding workflow | `codex/mvp-coding-workflow` | end-to-end granted repo edit/test | planned | — |
| Scenario runner | `codex/scenario-runner` | deterministic fixtures, oracle ladder | planned | — |
| Observability export | `codex/observability-export` | OTel spans, audit export | planned | — |
| Tool registry | `codex/tool-registry` | signed manifests, pinning, risk metadata | planned | — |
| MCP gateway | `codex/mcp-gateway` | MCP adapter, no token passthrough | planned | — |
| Browser service | `codex/browser-service` | isolated contexts, action receipts | planned | — |
| Memory provenance | `codex/memory-provenance` | journal-projected memory | planned | — |
| Human review | `codex/human-review` | review queue, approval receipts | planned | — |
| Model router | `codex/model-router` | policy-aware routing metadata | planned | — |
| Payment mandates | `codex/payment-mandates` | bounded mandates, fake rail | planned | — |
| Release gates | `codex/release-gates` | mandatory eval gates | planned | — |
| Distribution hardening | `codex/distribution-hardening` | installable local runtime | planned | — |
| High-assurance track | `codex/high-assurance-track` | formal invariants, crypto agility | planned | — |

### claude agent (coordination + interop + eval-data layer — deliberately disjoint from codex's Rust source tree)

| Slice | Branch | Scope | Status | PR |
| --- | --- | --- | --- | --- |
| Coordination protocol | `claude/multi-agent-coordination-bgnft1` | `docs/multi-agent-coordination.md`, `docs/ownership-registry.md`, `docs/reviewer-guide.md` | in-progress | — |
| PR-governance automation | `claude/pr-governance-ci-bgnft1` | `.github/workflows/pr-governance.yml`, `scripts/**` | planned | — |
| Contract schemas (interop) | `claude/contract-schemas-bgnft1` | `contracts/schemas/**`, `contracts/examples/**`, `contracts/validate.py`, `contracts/README.md` | planned | — |
| Scenario & security-eval fixtures | `claude/scenario-fixtures-bgnft1` | `scenarios/**` | planned | — |

## Coordination Notes

- **2026-07-03 (claude → codex):** claude is intentionally building the
  language-neutral / process / eval-data layer to stay disjoint from
  codex's Rust source tree. The `contracts/schemas/*` files mirror the
  exact serde field names in `crates/beater-os-core/src/contracts.rs` so
  the two layers validate the same wire format. If codex renames a contract
  field, please update `contracts/schemas/` (or ping claude) so the schemas
  stay in sync. See protocol §6 option 2 (build against the contract).
- **Rationale for disjoint scopes:** everything Rust depends on codex's
  unmerged workspace root (`Cargo.toml`/`crates/`). To avoid destroying
  in-flight work, claude adds no Rust and no root-level build files.
