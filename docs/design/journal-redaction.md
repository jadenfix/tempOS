# Design Spec: Redaction Without Breaking Integrity

Status: design spec closing issue #9. Audit/plan-hardening lane (PR #21). Aligned
to `final.md` §8.16 / §13.11 (the receipt-granularity + Merkle-anchor model) and
the shipped `crates/beater-os-core/src/{journal.rs,hash.rs}`. Reconciles the
append-only integrity requirement with redaction / right-to-be-forgotten (§3.4,
§10.4, §12.5).

> Reconciliation note (2026-07-05): an earlier draft (a) treated per-`JournalEvent`
> hashing as the integrity model and (b) used a content-addressed `content_ref =
> H(payload)`. Both are corrected here: `final.md` §8.16 targets **receipt-
> granularity** hash-linking with **Merkle anchors**, not per-event hashing; and a
> raw `H(payload)` reference is a **guessable digest** of low-entropy payloads that
> would survive redaction, defeating erasure.

## 1. Integrity model to design against (final.md §8.16, Issue #53)

Not per-journal-event hashing. The target, per `final.md`:

- **Hash-link at receipt granularity**, per-actor receipt chains
  (`ReceiptLedger`), so concurrent subagents don't contend on one chain head.
- **Merkle-batch at anchor points** — the session journal periodically anchors all
  actor chain heads into one Merkle node (§8.16, §10.4). Verification = per-chain
  links + anchor inclusion.
- Pure observations are async/batched; nothing irreversible happens un-journaled.

Redaction must preserve *both* the per-receipt links and the Merkle anchors while
letting the underlying payload be erased. (Today `journal.rs` hashes each event
inline via `hash_json`; that is the current implementation, but this spec targets
the §8.16 model the code is moving toward, and the mechanism below works for both.)

## 2. Mechanism: commit to a salted digest, store payload behind an opaque handle

Split every redactable record (receipt or journal entry) into:

- **Envelope** (in the hash-linked chain / Merkle leaf; never erased): non-
  sensitive metadata + a `content_commitment` + an **opaque** `content_handle`.
- **Payload** (in a separate, erasable content store, keyed by `content_handle`):
  the sensitive object.

```
salt              = 32 random bytes                       // erased on redaction
content_commitment = H(salt || canonical_bytes(payload))  // hiding commitment
content_handle    = random 128-bit id                      // NOT H(payload)
leaf/link hashes over the ENVELOPE (commitment + handle + meta), never the payload
```

**Redaction = delete the payload and its salt from the content store.** The
envelope, its `content_commitment`, the receipt links, and the Merkle anchors are
untouched, so verification still passes; the payload is simply unrecoverable.

### 2.1 Why the handle must be opaque and the commitment salted

- A content-addressed `H(payload)` **leaks the payload**: an attacker who guesses a
  low-entropy value (an email, a short secret) and hashes it confirms it against
  the surviving digest. So the stored reference is a **random opaque handle**, not
  a hash of content.
- The integrity commitment is **salted** (`H(salt‖payload)`) and the salt is erased
  with the payload, so after redaction the commitment is also non-guessing —
  it proves "some payload was committed here" without revealing which. Standard
  hiding commitment; no novel crypto (§13.12).

## 3. Redaction is a first-class, audited event

Redaction must be authorized and recorded, not a silent delete. `JournalEvent`
currently has no redaction variant, so add:

```rust
RedactionApplied {
    target_handle: ContentHandle,   // which payload was erased
    target_ref: ReceiptId | Seq,    // the record it belonged to
    authorized_by: CapabilityId,    // capability permitting redaction (fail-closed)
    reason: String,                 // e.g. "gdpr-erasure", "secret-leak-cleanup"
    // no payload content
}
```

Appended and Merkle-anchored like any record, so the journal proves *that* a
redaction happened, *who* authorized it, and *when* — while proving nothing about
the erased content. Redaction authority is a capability; the model can never
self-authorize it (§13.2).

## 4. Verification truth table

For a redactable record, verification distinguishes:

| Payload present? | Matching `RedactionApplied`? | Commitment recomputes? | Verdict |
| --- | --- | --- | --- |
| yes | no | yes | **intact** |
| no | yes | (n/a — payload gone) | **redacted (authorized)** — OK |
| no | no | (n/a) | **missing/tampered** — FAIL |
| yes | (any) | no | **tampered** — FAIL |

Authorized redaction verifies as redacted; unauthorized deletion is
indistinguishable from tampering (fails). This is the §10.4 requirement.

## 5. Dependencies

- **Canonical hashing.** Commitments and links must use a canonical encoding (the
  coordination ledger tracks adopting JCS/RFC 8785 so hashes verify cross-language);
  `hash.rs` currently does `sha256(serde_json::to_vec(..))`. Redaction commitments
  must use the same canonical encoder.
- **Receipts already digest-based.** `CapabilityReceipt` carries
  `input_digest`/`output_digest`; fold those into the same handle+commitment model
  so one redaction path covers receipts and journal payloads. (Note: those digests
  today are plain content digests — same guessability caveat as §2.1 if they ever
  cover low-entropy inputs; salt them or keep them over already-high-entropy data.)
- **Transparency log / Merkle anchors** carry only envelopes (commitments), never
  payloads, so external anchoring never blocks local erasure.

## 6. Follow-up kernel slice (contracts/kernel lane)

1. Introduce `ContentHandle` + content store; move sensitive fields behind
   `content_handle` + salted `content_commitment` at **receipt granularity**.
2. Add `JournalEvent::RedactionApplied` + an authorized `redact(...)`.
3. Implement the §4 truth table in chain + anchor verification.
4. Use the canonical (JCS) encoder for all commitments/links.

## 7. Acceptance mapping (issue #9)

- [x] Redaction-preserving-integrity mechanism specified against the §8.16 model — §1–§4.
- [x] PII/secret erasure end-to-end — erase payload+salt; opaque handle + salted
      commitment leak nothing (§2, §2.1).
- [x] Verifier distinguishes authorized redaction from tampering — §4.
