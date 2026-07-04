# beater-os-tool-registry

The trustworthy **tool registry** for the beaterOS agent kernel — backlog
slice 9, implementing `final.md` §10.14 (Package And Tool Registry), §6.9
(Trustworthy Package And Tool Supply), §13.6 (MCP And Remote Tool Security),
and §13.10 (Supply Chain).

`final.md`: *"an agent OS without a trustworthy tool registry will inherit every
supply-chain problem in the ecosystem"* (§10.14), and *"tools are treated as
untrusted unless registered, pinned, signed, and policy-approved"* (§3.1).

This crate builds on the `ToolManifest` contract in `beater-os-core` (it does
**not** modify the core) and adds the registry metadata and admission logic:

| Capability | `final.md` | How |
| --- | --- | --- |
| Signed manifests | §6.9, §13.10 | trusted-publisher set + signed-digest == content-digest |
| Version + schema pinning | §13.6, §13.10 | `pin()`; resolve requires pinned version **and** digest |
| Risk ceiling | §10.14, §13.3 | `RegistryPolicy::max_risk` |
| Sandbox floor | §13.8 | high-risk tools must declare `sandbox_required` |
| Test status gate | §10.14 | `require_passing_tests` |
| Per-workspace allowlists | §13.6 | `set_workspace_allowlist()` |
| Quarantine & revocation | §13.6, §13.10 | `quarantine()` / `revoke()` |
| Tamper detection | §13.11 | caller `expected_digest` must match |
| Audit trail | §4.5, §13.11 | append-only `RegistryEvent` log |

**Everything fails closed.** `resolve()` returns a tool only when every check
passes; an unregistered, unpinned-mismatch, untrusted, untested (when required),
over-risk, unsandboxed-high-risk, quarantined, or revoked tool is never
resolvable.

## Boundary

- **No cryptography is invented** (§22.7). This crate owns the *trust policy*
  over signer identity and content digest; verifying signature bytes against a
  real key belongs to a later crypto layer (§13.12).
- **Disjoint from the core.** No edits to `crates/beater-os-core`.
- **Not the gateway.** Slice 10 (`mcp-gateway`) will call `resolve()` and then
  drive `PolicyEngine` admission + receipts. This crate is the identity/trust
  layer that the gateway sits on top of.

## Checks

```sh
cargo fmt --all -- --check
cargo test -p beater-os-tool-registry --locked
cargo clippy -p beater-os-tool-registry --all-targets --locked -- -D warnings
```
