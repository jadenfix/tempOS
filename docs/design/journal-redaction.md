# Design Spec: Journal Redaction Without Breaking Integrity

Status: design spec closing issue #9. Audit/plan-hardening lane (PR #21).
Grounded in the shipped journal at `crates/beater-os-core/src/journal.rs` and
`hash.rs`. Reconciles two `final.md` requirements that today are in direct
tension in the code: an **append-only, hash-linked journal** (§8.3, §13.11) that
must also support **redaction / right-to-be-forgotten** (§3.4, §10.4 "redaction
without destroying integrity", §12.5).

## 1. The problem is concrete, not hypothetical

The merged journal embeds **full contract objects inline** in each event
(`journal.rs`):

```rust
pub enum JournalEvent {
    SessionCreated  { session: AgentSession },
    CapabilityGranted { grant: CapabilityGrant },
    ActionProposed  { manifest: ActionManifest },   // full inputs_summary, target, ...
    PolicyDecided   { decision: PolicyDecision },
    ReceiptAppended { receipt: CapabilityReceipt },
    MemoryWritten   { memory: MemoryRecord },        // full content
    ...
}
```

and the per-record hash covers the **entire event**:

```rust
record.hash = hash_json(&JournalHashView { seq, created_at, event, prev_hash });
// verify_chain() recomputes exactly this and checks prev_hash linkage.
```

Consequences:

1. Any secret/PII that reaches an `inputs_summary`, a `MemoryRecord.content`, or a
   grant `reason` is stored **in full** in the journal.
2. Deleting or masking any byte of that content changes `hash_json(event)`, which
   breaks the record's `hash` and every subsequent `prev_hash` link →
   `verify_chain()` fails. So today redaction and integrity are mutually
   exclusive. This is exactly what §12.5 waves at ("redacted through references
   without breaking the receipt chain") but the code does not yet implement.

Note: `CapabilityReceipt` already does the right thing partially — it carries
`input_digest`/`output_digest`, not raw payloads. The fix generalizes that
pattern to every event that can hold sensitive content.

## 2. Mechanism: commit to a digest, store the payload separately

Split every sensitive event into two parts:

- **Envelope** (in the journal, hash-covered, never erased): non-sensitive
  metadata + a `content_commitment` (a salted digest of the payload) + a
  `content_ref` (content-addressed pointer).
- **Payload** (in a separate, erasable content store keyed by `content_ref`):
  the actual sensitive object.

The chain hashes the **envelope**, i.e. the commitment — not the raw payload:

```
content_commitment = H(salt || canonical_bytes(payload))
envelope           = { seq, created_at, kind, non_sensitive_meta,
                       content_ref, content_commitment, prev_hash }
record.hash        = H(canonical_bytes(envelope))
```

**Redaction = delete the payload (and its salt) from the content store.** The
envelope, its `content_commitment`, and the whole hash chain are untouched, so
`verify_chain()` still passes. What changes is only that the payload is no longer
retrievable.

### 2.1 Why the salt

Without a salt, a low-entropy payload (e.g. an email address, a short secret) can
be **confirmed** after redaction by an attacker who guesses the value and hashes
it against the surviving commitment. A per-payload random `salt`, stored with the
payload and erased on redaction, makes the commitment hiding: after redaction the
commitment reveals nothing about the payload. (This is a standard commitment
scheme; no novel crypto — §13.12 "do not invent primitives.")

## 3. Redaction as a first-class, audited event

Redaction must itself be authorized, recorded, and tamper-evident — not a silent
delete. Add a journal event:

```rust
RedactionApplied {
    target_ref: ContentRef,      // which payload was erased
    target_seq: u64,             // record whose payload this was
    authorized_by: CapabilityId, // capability that permits redaction (fail-closed)
    reason: String,              // e.g. "gdpr-erasure", "secret-leak-cleanup"
    // no payload content here
}
```

Because `RedactionApplied` is appended and hash-linked like any event, the
journal proves *that* a redaction happened, *who* authorized it, and *when* —
while proving nothing about the erased content. Redaction authority is a
capability (fail-closed, §13.2); the model can never self-authorize it.

## 4. Verification semantics (the important part)

`verify_chain()` must distinguish three states for a redactable record:

| Payload present? | Matching `RedactionApplied`? | Commitment recomputes? | Verdict |
| --- | --- | --- | --- |
| yes | no | yes | **intact** |
| no | yes | (n/a — payload gone) | **redacted (authorized)** — OK |
| no | no | (n/a) | **missing/tampered** — FAIL |
| yes | (any) | no | **tampered** — FAIL |

So an authorized redaction is verifiable-as-redacted, and an unauthorized
deletion is indistinguishable from tampering (i.e. it fails). This is the
property §10.4 asks for.

## 5. Dependencies and interactions

- **Canonical hashing (blocker):** `hash.rs` currently does
  `sha256(serde_json::to_vec(..))`, which is **not** a canonical encoding. The
  coordination ledger already raises adopting **JCS (RFC 8785)** so hashes verify
  cross-language. Redaction commitments *must* use the canonical encoder too;
  otherwise a re-serialization changes the commitment and looks like tampering.
  This spec depends on that JCS decision landing.
- **Receipts:** already digest-based; fold their `input_digest`/`output_digest`
  into the same content-store + commitment model so one redaction path covers
  receipts, manifests, and memory.
- **Transparency log (§20.3, #20.3):** if envelopes are mirrored to an external
  transparency log, only envelopes (commitments) go out — never payloads — so
  erasure remains possible locally without contradicting an append-only external
  log.
- **Memory service (#9 ↔ memory):** `MemoryRecord` redaction/expiry (§10.8) uses
  the same mechanism; "forget this everywhere you are allowed to" becomes
  "erase the payload + append a `RedactionApplied`."

## 6. Follow-up kernel slice (for the contracts lane)

1. Introduce a `ContentRef` + content store; move sensitive fields of
   `ActionManifest`/`MemoryRecord`/grant `reason` behind `content_ref` +
   `content_commitment` in `JournalEvent`.
2. Add `JournalEvent::RedactionApplied` and an `InMemoryJournal::redact(...)` that
   requires a redaction capability and appends the record.
3. Update `verify_chain()` to implement the §4 truth table.
4. Switch `hash_json` to the canonical (JCS) encoder first.

This spec is the contract; implementation belongs to the kernel lane (codex/#1),
consistent with `risk-class.md`.

## 7. Acceptance mapping (issue #9)

- [x] Redaction-preserving-integrity mechanism specified, not just asserted — §2–§4.
- [x] PII/secret erasure path described end-to-end — payload+salt erase, envelope
      survives, `RedactionApplied` audits it (§2–§3).
- [x] Verifier distinguishes authorized redaction from tampering — §4 truth table.
