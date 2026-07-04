//! Accountable memory projection for beaterOS.
//!
//! This crate realizes `final.md` §25 build-order step 10 ("Memory projection
//! from journal") and the §10.8 Memory Service responsibilities of *building
//! projections*, *serving context with provenance*, *enforcing retention*, and
//! *supporting redaction*. It exists to keep the §26 NEVER-compromise invariant
//! **"Memory provenance"** true by construction: memory is a deterministic
//! *fold* over the append-only, hash-chained journal, never a separate mutable
//! store that could drift from the audit trail.
//!
//! Every projected memory originates from a [`JournalEvent::MemoryWritten`]
//! record, so it is always traceable back to a journaled source event, its
//! writer, and the exact journal record (seq + hash) that wrote it. Replaying
//! the same journal always yields the same projection.
//!
//! # Before Coding (per `CLAUDE.md`)
//!
//! - **Critical path.** [`project`] / [`project_with_redactions`] is a single
//!   linear pass over the journal records; provenance and expiry are decided
//!   inline. Provenance/active lookups after projection are `O(log n)`
//!   `BTreeMap` reads. There is no non-critical background path — the projection
//!   is a pure value computed on demand.
//! - **Allocation / copy / syscall.** No syscalls, no I/O, no async. Bounded by
//!   journal size: one [`ProjectedMemory`] per *distinct* `memory_id` (a later
//!   `MemoryWritten` for the same id replaces the earlier one — last-writer-wins),
//!   so memory never grows past the number of live memory ids. Each surviving
//!   record is cloned once into the projection; non-memory records are skipped
//!   without allocation.
//! - **Queue / retry bounds.** None. The projection is synchronous and total; it
//!   cannot block, retry, or fail partway. Determinism comes from folding in
//!   journal (seq) order into an ordered [`std::collections::BTreeMap`].
//! - **Failure mode under overload.** Fails *closed* on the safety-relevant
//!   axes: an expired memory (`expires_at <= now`) is excluded from
//!   [`MemoryProjection::active`] and confers nothing; it survives only in the
//!   audit view for accountability. A redacted memory keeps its provenance but
//!   surrenders its `content_ref`/`summary`. There is no code path that serves a
//!   memory without provenance.
//! - **Security boundary & evidence.** The evidence is the journal itself. This
//!   crate reads a journal/snapshot and *never mutates it* — redaction is a
//!   projection-layer concern (see [`RedactionDirective`]) so the hash chain and
//!   [`beater_os_core::InMemoryJournal::verify_chain`] remain intact. The
//!   accountability chain for any memory is exposed via
//!   [`MemoryProjection::provenance`].
//! - **Language / Rust tie-breaker.** Pure deterministic in-memory fold over
//!   core contracts: idiomatic safe Rust, no FFI, no unsafe (workspace forbids
//!   it). No tie-breaker in play.
//! - **macOS impact.** None. No platform-specific code, I/O, or dependencies
//!   beyond `beater-os-core` and `chrono`.
//! - **Local verification.**
//!   `cargo test -p beater-os-memory && cargo clippy -p beater-os-memory --all-targets -- -D warnings`
//!
//! # Redaction seam (issue #9)
//!
//! The append-only, hash-chained journal must NEVER be mutated; doing so would
//! break §26 integrity. So redaction is modeled here as a *projection input*: a
//! [`RedactionDirective`] omits/replaces a memory's `content_ref` and `summary`
//! in the projected view while keeping its provenance metadata, and the
//! underlying journal record (and its hash) is left untouched. This crate
//! provides the *mechanism/seam* only. The deep policy question of append-only
//! integrity vs. a right-to-be-forgotten is tracked in **issue #9** and is
//! intentionally NOT decided here.
//!
//! ```
//! use beater_os_memory::project;
//! use beater_os_core::InMemoryJournal;
//! use chrono::Utc;
//!
//! let journal = InMemoryJournal::new();
//! let projection = project(&journal, Utc::now());
//! assert!(projection.is_empty());
//! ```

use std::collections::BTreeMap;

use beater_os_core::{DataClass, MemoryRecord};
use beater_os_core::{HashValue, InMemoryJournal, JournalEvent, JournalRecord, JournalSnapshot};
use chrono::{DateTime, Utc};

/// Default replacement text substituted for a redacted memory's `content_ref`
/// and `summary` when a [`RedactionDirective`] supplies no explicit replacement.
pub const REDACTION_PLACEHOLDER: &str = "[redacted]";

/// Anything that exposes a slice of journal records in append order.
///
/// Implemented for [`InMemoryJournal`], [`JournalSnapshot`], and a raw
/// `[JournalRecord]` slice so [`project`] accepts a journal or a snapshot
/// interchangeably (the §12.6 "derived memory can be rebuilt" invariant: a
/// snapshot replays to the same projection as the live journal it came from).
pub trait JournalRecords {
    /// The journal records, in append (seq) order.
    fn journal_records(&self) -> &[JournalRecord];
}

impl JournalRecords for InMemoryJournal {
    fn journal_records(&self) -> &[JournalRecord] {
        self.records()
    }
}

impl JournalRecords for JournalSnapshot {
    fn journal_records(&self) -> &[JournalRecord] {
        &self.records
    }
}

impl JournalRecords for Vec<JournalRecord> {
    fn journal_records(&self) -> &[JournalRecord] {
        self
    }
}

/// A projection-layer directive to redact a memory's content by `memory_id`.
///
/// This does NOT touch the journal (see the crate-level "Redaction seam" note
/// and issue #9). It only tells [`project_with_redactions`] to replace the
/// projected memory's `content_ref`/`summary` while preserving its provenance.
#[derive(Clone, Debug, PartialEq, Eq)]
pub struct RedactionDirective {
    /// The memory whose content is to be omitted from the projected view.
    pub memory_id: String,
    /// Replacement text for `content_ref`/`summary`. `None` uses
    /// [`REDACTION_PLACEHOLDER`].
    pub replacement: Option<String>,
}

impl RedactionDirective {
    /// Redact `memory_id` with the default [`REDACTION_PLACEHOLDER`].
    pub fn new(memory_id: impl Into<String>) -> Self {
        Self {
            memory_id: memory_id.into(),
            replacement: None,
        }
    }

    fn replacement_text(&self) -> &str {
        self.replacement.as_deref().unwrap_or(REDACTION_PLACEHOLDER)
    }
}

/// The accountability chain for a single projected memory (§26 "Memory
/// provenance"): where it came from, who wrote it, and the exact journal record
/// that recorded it. Every [`ProjectedMemory`] has one; a memory without a
/// journaled source is impossible by construction.
#[derive(Clone, Debug, PartialEq, Eq)]
pub struct MemoryProvenance {
    /// The memory this provenance describes.
    pub memory_id: String,
    /// The source event that gave rise to the memory (`MemoryRecord::source_event_id`).
    pub source_event_id: String,
    /// Digest of the source material (`MemoryRecord::source_digest`).
    pub source_digest: String,
    /// The principal that wrote the memory (`MemoryRecord::writer`).
    pub writer: String,
    /// `seq` of the journal record whose `MemoryWritten` event wrote this memory.
    pub journal_seq: u64,
    /// Hash of that journal record — anchors the memory into the hash chain.
    pub journal_record_hash: HashValue,
    /// When the memory was written (`MemoryRecord::created_at`).
    pub created_at: DateTime<Utc>,
}

/// One memory in the projection: the (possibly redacted) record, its
/// provenance, and its expiry/redaction status at the projection's `now`.
#[derive(Clone, Debug, PartialEq, Eq)]
pub struct ProjectedMemory {
    record: MemoryRecord,
    provenance: MemoryProvenance,
    expired: bool,
    redacted: bool,
}

impl ProjectedMemory {
    /// The projected memory record. If [`Self::is_redacted`], `content_ref` and
    /// `summary` are the replacement text; all other fields are as journaled.
    pub fn record(&self) -> &MemoryRecord {
        &self.record
    }

    /// The accountability chain for this memory. Always present.
    pub fn provenance(&self) -> &MemoryProvenance {
        &self.provenance
    }

    /// `true` if `expires_at <= now`. Expired memory is excluded from
    /// [`MemoryProjection::active`] and confers nothing (fail-closed).
    pub fn is_expired(&self) -> bool {
        self.expired
    }

    /// `true` if a [`RedactionDirective`] omitted this memory's content.
    pub fn is_redacted(&self) -> bool {
        self.redacted
    }

    /// `true` if the memory may be served in the active context view (not expired).
    pub fn is_active(&self) -> bool {
        !self.expired
    }
}

/// The current memory state derived from a journal at a fixed `now`.
///
/// Deterministic and idempotent: [`project`]-ing the same records with the same
/// `now` (and redactions) always yields an equal `MemoryProjection`. Ordered by
/// `memory_id` for stable iteration.
#[derive(Clone, Debug, PartialEq, Eq)]
pub struct MemoryProjection {
    projected_at: DateTime<Utc>,
    memories: BTreeMap<String, ProjectedMemory>,
}

impl MemoryProjection {
    /// The `now` this projection was computed at (the expiry cut-off).
    pub fn projected_at(&self) -> DateTime<Utc> {
        self.projected_at
    }

    /// Total number of distinct memories (active + expired).
    pub fn len(&self) -> usize {
        self.memories.len()
    }

    /// `true` if no memory has been written to the projected journal.
    pub fn is_empty(&self) -> bool {
        self.memories.is_empty()
    }

    /// Look up a memory by id, regardless of expiry.
    pub fn get(&self, memory_id: &str) -> Option<&ProjectedMemory> {
        self.memories.get(memory_id)
    }

    /// The accountability chain for a memory id, if it exists (§26 provenance
    /// query).
    pub fn provenance(&self, memory_id: &str) -> Option<&MemoryProvenance> {
        self.memories
            .get(memory_id)
            .map(ProjectedMemory::provenance)
    }

    /// The active context view: memories that are not expired. Redacted memories
    /// are included but carry no content. Ordered by `memory_id`.
    pub fn active(&self) -> impl Iterator<Item = &ProjectedMemory> {
        self.memories.values().filter(|m| m.is_active())
    }

    /// The expired memories, retained for audit only (never served). Ordered by
    /// `memory_id`.
    pub fn expired(&self) -> impl Iterator<Item = &ProjectedMemory> {
        self.memories.values().filter(|m| m.is_expired())
    }

    /// Every projected memory, active and expired, for accountability/audit.
    /// Ordered by `memory_id`.
    pub fn audit_view(&self) -> impl Iterator<Item = &ProjectedMemory> {
        self.memories.values()
    }

    /// Count of memories servable in the active view.
    pub fn active_count(&self) -> usize {
        self.active().count()
    }

    /// Count of expired (audit-only) memories.
    pub fn expired_count(&self) -> usize {
        self.expired().count()
    }

    /// The §26 invariant, checkable: every projected memory is traceable to a
    /// journaled source event. True by construction (projections only ingest
    /// [`JournalEvent::MemoryWritten`] records, each carrying a
    /// `source_event_id`); exposed so callers and tests can assert it.
    pub fn all_traceable(&self) -> bool {
        self.memories
            .values()
            .all(|m| !m.provenance.source_event_id.is_empty())
    }

    /// Count of memories at a given sensitivity class (§10.8 "separate data
    /// classes"). Redaction does not change a memory's sensitivity.
    pub fn count_by_sensitivity(&self, sensitivity: DataClass) -> usize {
        self.memories
            .values()
            .filter(|m| m.record.sensitivity == sensitivity)
            .count()
    }
}

/// Project the current memory state from a journal or snapshot at `now`.
///
/// A deterministic fold over every [`JournalEvent::MemoryWritten`] record.
/// Last-writer-wins: if the same `memory_id` is written more than once, the
/// record with the higher `seq` prevails (memory *can be invalidated* / updated;
/// §12.6). Expiry is fail-closed at `now`. Equivalent to
/// [`project_with_redactions`] with no directives.
pub fn project(source: &impl JournalRecords, now: DateTime<Utc>) -> MemoryProjection {
    project_with_redactions(source, now, &[])
}

/// Like [`project`], but applies projection-layer [`RedactionDirective`]s: a
/// redacted memory keeps its provenance and metadata but its
/// `content_ref`/`summary` are replaced. The journal is never mutated.
pub fn project_with_redactions(
    source: &impl JournalRecords,
    now: DateTime<Utc>,
    redactions: &[RedactionDirective],
) -> MemoryProjection {
    let redaction_by_id: BTreeMap<&str, &RedactionDirective> = redactions
        .iter()
        .map(|directive| (directive.memory_id.as_str(), directive))
        .collect();

    let mut memories: BTreeMap<String, ProjectedMemory> = BTreeMap::new();

    for record in source.journal_records() {
        let JournalEvent::MemoryWritten { memory } = &record.event else {
            continue;
        };

        let provenance = MemoryProvenance {
            memory_id: memory.memory_id.clone(),
            source_event_id: memory.source_event_id.clone(),
            source_digest: memory.source_digest.clone(),
            writer: memory.writer.clone(),
            journal_seq: record.seq,
            journal_record_hash: record.hash.clone(),
            created_at: memory.created_at,
        };

        let expired = memory
            .expires_at
            .is_some_and(|expires_at| expires_at <= now);

        let mut projected_record = memory.clone();
        let redacted = match redaction_by_id.get(memory.memory_id.as_str()) {
            Some(directive) => {
                let replacement = directive.replacement_text().to_string();
                projected_record.content_ref = replacement.clone();
                projected_record.summary = replacement;
                true
            }
            None => false,
        };

        // Last-writer-wins: a later `MemoryWritten` for the same id replaces the
        // earlier projection (keyed insert overwrites), so `memory_id` stays
        // unique and provenance points at the winning record.
        memories.insert(
            memory.memory_id.clone(),
            ProjectedMemory {
                record: projected_record,
                provenance,
                expired,
                redacted,
            },
        );
    }

    MemoryProjection {
        projected_at: now,
        memories,
    }
}
