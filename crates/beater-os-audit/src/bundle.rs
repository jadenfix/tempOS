//! Redaction-safe audit bundle export.
//!
//! `final.md` §6.6 asks for verifiable side effects hash-linked into a journal,
//! and §13.15 lists "export trace" and "generate incident timeline" as incident
//! response capabilities. This module produces a hand-off artifact that anchors
//! a run to its hashes and coverage without re-exporting raw event payloads,
//! which may contain sensitive summaries. It carries *what to trust and how
//! complete the record is*, not the private contents.

use beater_os_core::JournalSnapshot;
use serde::Serialize;

use crate::AUDIT_FORMAT_VERSION;
use crate::events::event_kind;
use crate::metrics::{AuditMetrics, compute_metrics};
use crate::verify::{AuditReport, GENESIS_HASH, verify_snapshot};

/// A per-record digest: enough to prove membership and linkage, no payload.
#[derive(Clone, Debug, PartialEq, Eq, Serialize)]
pub struct RecordDigest {
    pub seq: u64,
    pub kind: String,
    pub hash: String,
}

/// A redaction-safe audit bundle for hand-off or incident response.
#[derive(Clone, Debug, PartialEq, Eq, Serialize)]
pub struct AuditBundle {
    pub format_version: String,
    pub records: usize,
    /// Hash of the last record, or the genesis hash for an empty journal.
    pub root_hash: String,
    pub report: AuditReport,
    pub metrics: AuditMetrics,
    pub record_digests: Vec<RecordDigest>,
}

/// Build a [`AuditBundle`] from a snapshot. Read-only and deterministic. It
/// deliberately omits event payloads; only sequence, kind, and content hash are
/// exported per record.
pub fn build_bundle(snapshot: &JournalSnapshot) -> AuditBundle {
    let record_digests = snapshot
        .records
        .iter()
        .map(|record| RecordDigest {
            seq: record.seq,
            kind: event_kind(&record.event).to_string(),
            hash: record.hash.clone(),
        })
        .collect();

    let root_hash = snapshot
        .records
        .last()
        .map(|record| record.hash.clone())
        .unwrap_or_else(|| GENESIS_HASH.to_string());

    AuditBundle {
        format_version: AUDIT_FORMAT_VERSION.to_string(),
        records: snapshot.records.len(),
        root_hash,
        report: verify_snapshot(snapshot),
        metrics: compute_metrics(snapshot),
        record_digests,
    }
}

/// Serialize a bundle to pretty JSON. Errors only if serialization fails, which
/// cannot happen for these plain data types but is surfaced rather than hidden.
pub fn bundle_to_json(bundle: &AuditBundle) -> Result<String, serde_json::Error> {
    serde_json::to_string_pretty(bundle)
}

#[cfg(test)]
mod tests {
    use super::*;
    use beater_os_core::JournalSnapshot;

    #[test]
    fn empty_bundle_is_well_formed() {
        let bundle = build_bundle(&JournalSnapshot::default());
        assert_eq!(bundle.records, 0);
        assert_eq!(bundle.root_hash, GENESIS_HASH);
        assert!(bundle.report.ok);
        assert!(bundle.record_digests.is_empty());
        let json = bundle_to_json(&bundle);
        assert!(json.is_ok());
    }
}
