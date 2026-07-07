# beater-os-audit

Independent audit surface for beaterOS. Reviewer-facing companion to
`beater-os-core`.

This crate exists so that the party who *reviews* a run is not forced to trust
the same code path that *produced* it. It re-derives the audit invariants from
`final.md` independently and presents a run in a form a human reviewer can read.

## Scope (this slice)

- **Independent verification** (`verify_snapshot`) — a second implementation of
  the journal audit invariants, in three families:
  - *Delegated content-hash integrity*: `cryptographic_chain` (the only signal
    that recomputes record hashes).
  - *Defense-in-depth second implementations* (intentionally overlap the core
    verifier): `sequence_contiguous`, `hash_linkage` (prev-hash linkage, not
    content), `receipt_causality`.
  - *Novel gap-fillers* (invariants the core journal verifier does **not**
    check): `referential_sessions`, `grant_references`, `grant_validity`
    (unrevoked + unexpired at use, `final.md` §26), `denial_explained` (§22.9).

  Fails closed. Maps to `final.md` §8.15, §13.11, §22.9, §26.
- **Trace rendering** (`render_trace`) — a legible, deterministic timeline of a
  session (`final.md` §25 step 9, §17.4).
- **Audit metrics** (`compute_metrics`) — exact coverage ratios for reviewers:
  decision coverage, receipt coverage, and denial-explanation coverage
  (`final.md` §23.3).
- **Audit bundle export** (`build_bundle`) — a redaction-safe, digest-anchored
  export for incident response and hand-off: it carries per-record hashes,
  kinds, verification results, and coverage, but not raw event payloads
  (`final.md` §6.6, §13.15).
- **Trace bundle verification** (`verify_trace_bundle`) — read-only verification
  for the full `beaterosctl trace export` artifact. The embedded journal is the
  authority payload; exported projection arrays are checked against a
  journal-derived projection and never imported into live daemon state.
- **`beateros-audit` binary** — `verify` / `show` / `metrics` / `bundle` a
  journal snapshot from a file or stdin, and `verify-trace` a full trace bundle.
- **Expected-root verification** — `verify --expected-root <hash>` compares the
  snapshot root against an external anchor, so a valid prefix or coherent
  re-hash can be detected relative to the trusted hand-off value.

## CLI

```sh
# exit non-zero if any independent audit check fails
beateros-audit verify snapshot.json
# require the snapshot to match an externally anchored root hash
beateros-audit verify --expected-root <root-hash> snapshot.json
# print a legible timeline
beateros-audit show snapshot.json
# coverage metrics as JSON
beateros-audit metrics snapshot.json
# redaction-safe audit bundle as JSON (also accepts - for stdin)
cat snapshot.json | beateros-audit bundle -
# verify the full trace/debug bundle emitted by beaterosctl
beaterosctl trace export --session demo | beateros-audit verify-trace -
# bind trace verification to an externally trusted journal root
beateros-audit verify-trace --expected-root <root-hash> trace-bundle.json
```

## Boundary vs. `observability-export` (backlog slice 7)

Slice 7 (`observability-export`) is about *live* emission: OpenTelemetry-style
spans wired into the session runtime and export plumbing. This crate is the
*offline, independent* counterpart: it consumes a journal snapshot after the
fact and re-verifies / renders / scores it, with no dependency on the session
runtime. The two are complementary; if their ownership starts to overlap, the
boundary should be settled on the PR before either lands.

## Non-goals

- It does not replace the core verifier; it corroborates it.
- It does not import, restore, resume, or apply full trace bundles.
- It does not emit spans or own live tracing (that is slice 7).
- It performs no network or filesystem I/O of its own beyond CLI arguments.
