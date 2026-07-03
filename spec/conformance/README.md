# Conformance runner

`validate.py` is a small, dependency-free JSON Schema (draft 2020-12 **subset**)
validator plus a runner over the fixtures in `spec/examples`.

## Why not use an off-the-shelf validator?

The conformance suite is the shared source of truth for every beaterOS
implementation and must run anywhere with zero setup: a stock Python 3, no
`pip install`, no network. A full JSON Schema library would be a dependency and
a supply-chain surface for a repo whose entire thesis is trustworthy tooling.
The validator here covers exactly the keywords the contract schemas use and is
short enough for any reviewer to audit in one sitting.

## Supported keywords

`$ref` (local `#/…` and cross-file `file.schema.json#/…`), `$defs`, `type`
(incl. arrays and `null`), `enum`, `const`, `properties`, `required`,
`additionalProperties` (bool or schema), `patternProperties`, `items`,
`oneOf` / `anyOf` / `allOf`, `minLength` / `maxLength`, `minimum` / `maximum`,
`minItems` / `maxItems`, `uniqueItems`, `pattern`, and `format: date-time`.

Notable correctness details:

- JSON booleans are never treated as `integer`/`number` (Python's `bool`-is-`int`
  trap is handled explicitly).
- `uniqueItems` compares items by canonical JSON, so object/array duplicates are
  caught, not just scalars.
- `format: date-time` is validated against an RFC 3339 pattern (the shape
  `serde`/`chrono` emits).

If a future schema needs a keyword not listed above, add it to `validate.py`
in the same PR and cover it with an example.

## How the runner maps fixtures to schemas

For each `spec/contracts/<name>.schema.json`, the runner validates every file in
`spec/examples/valid/<name>/` (which must all pass) and every file in
`spec/examples/invalid/<name>/` (which must all be rejected). It also fails if
any contract is missing a valid or an invalid example, so coverage cannot
silently regress.

Each invalid fixture isolates a **single** defect so the rejection reason is
unambiguous (see the `(rejected: …)` line the runner prints).
