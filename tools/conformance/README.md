# beaterOS Conformance Gate

A dependency-free (Python 3.9+ stdlib only) **executable eval gate** for the
language-neutral contract corpus in this repo. It is the release-gate layer that
`final.md` §5.9 and §14.6 call for: every contract and behaviour gets a machine
check that any implementation must pass.

It validates the corpus against:

1. **JSON Schemas** (`contracts/schema/`) — structural conformance of every
   session, grant, manifest, decision, receipt, memory record, payment mandate,
   scenario, and journal record.
2. **Semantic invariants** (`admission.py`, `journalcheck.py`) — an independent
   Python port of the deterministic rules in `crates/beater-os-core` (policy
   admission, journal causality, hash-linked receipt/journal chains). Because it
   is a *second implementation in a second language*, the corpus doubles as a
   **cross-language differential oracle** for the Rust core (`final.md` §15.6:
   verifier separable from executor).
3. **Adversarial scenarios** (`scenarios/`) — proof that the policy layer blocks
   or escalates attacks even when the model is persuaded (`final.md` §14.5).

## Usage

```
python3 tools/conformance/validate.py                 # validate the whole corpus
python3 tools/conformance/build_fixtures.py --check   # golden traces are reproducible
```

Both run in `.github/workflows/contracts.yml`.

## Scope & deconfliction (multi-agent build)

This slice (branch `claude/multi-agent-pr-review-a3bwl1`, PR #22) owns **only**
the executable conformance/eval-data layer and edits no file owned by another
open PR:

| This slice adds | Deliberately NOT in scope (owned elsewhere) |
| --- | --- |
| `contracts/schema/**` (JSON Schemas) | Rust core `crates/beater-os-core/**` — PR #1 |
| `examples/traces/**` (machine-readable golden fixtures) | Governance protocol / `AGENTS.md` — PR #19/#20 |
| `scenarios/**` (adversarial eval corpus) | `docs/threat-model.md`, `docs/glossary.md`, prose example — PR #21 |
| `tools/conformance/**` (this gate) | Slice plan `docs/implementation-backlog.md` — PR #1 |
| `.github/workflows/contracts.yml` | Rust CI `ci.yml`, governance CI — PR #1/#19 |

The JSON Schemas are pinned **field-for-field to PR #1's Rust types** (exact
field names, snake_case enum values as of commit `3e5625a`) so they are a
canonical cross-language mirror, not a competing vocabulary.

## Open cross-implementation item: canonical hashing

The Rust core hashes values with serde's *struct-declaration* field order
(`crates/beater-os-core/src/hash.rs`), which a non-Rust implementation cannot
reproduce, so digests will not match across languages. This harness uses a JSON
Canonicalization Scheme (JCS, RFC 8785) layout — sorted keys, compact
separators (`canonical.py`). Recommendation: every implementation, including the
Rust core, adopts JCS so receipt/journal digests are byte-identical everywhere.
Raised with the core author on PR #1; needs a small joint follow-up to `hash.rs`.
Until then, this gate verifies chain *linkage and causality* (which are
language-neutral) and recomputes digests under its own documented canonical form.
