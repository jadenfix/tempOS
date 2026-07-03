# beaterOS Conformance Gate

A dependency-free (Python 3.9+ stdlib only) gate that validates the
language-neutral contract corpus in this repo against:

1. **JSON Schemas** (`contracts/schema/`) — structural conformance of every
   session, grant, manifest, decision, receipt, memory record, payment mandate,
   scenario, and journal record.
2. **Semantic invariants** (`admission.py`, `journalcheck.py`) — an independent
   port of the deterministic rules in `crates/beater-os-core` (policy admission,
   journal causality, hash-linked receipt/journal chains).
3. **Adversarial scenarios** (`scenarios/`) — proof that the policy layer blocks
   or escalates attacks even when the model is persuaded (`final.md` §14.5).

## Usage

```
python3 tools/conformance/validate.py          # validate the whole corpus
python3 tools/conformance/build_fixtures.py --check   # golden traces are reproducible
```

Both are wired into `.github/workflows/contracts.yml`. See `AGENTS.md` for how
this slice fits the multi-agent build and why it does not collide with the Rust
core crate.
