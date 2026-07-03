# Example Traces

Machine-readable, end-to-end trace bundles that make `final.md`'s reproducibility
and MVP claims concrete (§2.2, §24). Each `*.trace.json` conforms to
`contracts/schema/trace-bundle.schema.json` and is checked by the conformance
gate for schema conformance, hash-linked receipt/journal chains, journal
causality, and independent policy admission of every recorded decision.

## Bundles

- `coding-workflow.trace.json` — the §24 MVP proof: an agent reads a file
  (allowed), is **blocked** trying to write outside its granted path
  (`needs_narrowed_grant`), writes the fix in scope (allowed), and runs the test
  runner (allowed). Every side effect carries a receipt; every step is journaled.
- `payment-workflow.trace.json` — the human-review *approval-satisfied* path
  (§7.9, §13.14, §16.1): a bounded vendor `spend` over the grant's approval
  threshold is admitted **only because** valid, action-bound human approval
  evidence exists. Carries a `PaymentMandate` (§12.7) and a payment receipt.
  Complements the adversarial scenarios, which show approval being *required* but
  never *granted*.

## Regenerating

This bundle is generated (not hand-authored) so its 14 linked hashes stay
consistent:

```
python3 tools/conformance/build_fixtures.py           # regenerate
python3 tools/conformance/build_fixtures.py --check    # assert no drift (CI)
```

These are *prose-free machine fixtures* for the gate. A separate worked,
narrated example lives in the docs lane (PR #21, `docs/examples/`); the two are
complementary — this one is executed, that one is read.
