//! Independent verification of a beaterOS journal snapshot.
//!
//! `final.md` §8.15 argues for a small trusted computing base that can be
//! re-verified, and §13.11 requires tamper-evident logs. This module is a
//! deliberately *independent* second implementation of the audit invariants:
//! it delegates the cryptographic chain check back to `beater-os-core` as one
//! signal, then applies its own structural and cross-referential checks that
//! do not share state with the core verifier. If the two disagree, that
//! disagreement is itself an auditable incident.

use std::collections::BTreeSet;

use beater_os_core::{DecisionResult, InMemoryJournal, JournalEvent, JournalSnapshot};
use serde::Serialize;

/// Expected genesis linkage hash.
///
/// Hardcoded here on purpose: an independent auditor must not import the
/// constant it is checking against from the code under audit.
pub const GENESIS_HASH: &str = "0000000000000000000000000000000000000000000000000000000000000000";

/// Outcome of a single audit check.
#[derive(Clone, Copy, Debug, PartialEq, Eq, Serialize)]
#[serde(rename_all = "snake_case")]
pub enum CheckOutcome {
    Pass,
    Fail,
}

/// Result of one named audit check over a snapshot.
#[derive(Clone, Debug, PartialEq, Eq, Serialize)]
pub struct CheckResult {
    pub check: String,
    pub outcome: CheckOutcome,
    pub detail: String,
}

impl CheckResult {
    fn pass(check: &str, detail: impl Into<String>) -> Self {
        Self {
            check: check.to_string(),
            outcome: CheckOutcome::Pass,
            detail: detail.into(),
        }
    }

    fn fail(check: &str, detail: impl Into<String>) -> Self {
        Self {
            check: check.to_string(),
            outcome: CheckOutcome::Fail,
            detail: detail.into(),
        }
    }
}

/// Aggregate report over all independent checks for a snapshot.
#[derive(Clone, Debug, PartialEq, Eq, Serialize)]
pub struct AuditReport {
    pub records: usize,
    pub ok: bool,
    pub checks: Vec<CheckResult>,
}

impl AuditReport {
    /// Iterate over the checks that failed.
    pub fn failures(&self) -> impl Iterator<Item = &CheckResult> {
        self.checks
            .iter()
            .filter(|check| check.outcome == CheckOutcome::Fail)
    }
}

/// Run every independent audit check over `snapshot` and aggregate the result.
///
/// This never panics and never mutates the input. It fails closed: any check
/// that cannot positively confirm an invariant reports `Fail`.
pub fn verify_snapshot(snapshot: &JournalSnapshot) -> AuditReport {
    let checks = vec![
        check_cryptographic_chain(snapshot),
        check_sequence_contiguous(snapshot),
        check_hash_linkage(snapshot),
        check_referential_sessions(snapshot),
        check_grant_references(snapshot),
        check_receipt_causality(snapshot),
        check_denial_explained(snapshot),
    ];
    let ok = checks.iter().all(|c| c.outcome == CheckOutcome::Pass);
    AuditReport {
        records: snapshot.records.len(),
        ok,
        checks,
    }
}

/// Delegate the cryptographic hash-chain check to the core verifier. This is
/// one of two independent signals: the core recomputes content hashes; the
/// structural checks below re-derive linkage without the core's private view.
fn check_cryptographic_chain(snapshot: &JournalSnapshot) -> CheckResult {
    let journal = InMemoryJournal::from_records(snapshot.records.clone());
    match journal.verify_chain() {
        Ok(report) => CheckResult::pass(
            "cryptographic_chain",
            format!("core verifier accepted {} record(s)", report.records),
        ),
        Err(err) => CheckResult::fail("cryptographic_chain", err.to_string()),
    }
}

/// Sequence numbers must start at zero and be contiguous.
fn check_sequence_contiguous(snapshot: &JournalSnapshot) -> CheckResult {
    for (idx, record) in snapshot.records.iter().enumerate() {
        let expected = idx as u64;
        if record.seq != expected {
            return CheckResult::fail(
                "sequence_contiguous",
                format!(
                    "record at index {idx} has seq {}, expected {expected}",
                    record.seq
                ),
            );
        }
    }
    CheckResult::pass(
        "sequence_contiguous",
        "sequence numbers are contiguous from 0",
    )
}

/// Every record must link to its predecessor (genesis for the first) and
/// carry a non-empty content hash.
fn check_hash_linkage(snapshot: &JournalSnapshot) -> CheckResult {
    let mut prev_hash = GENESIS_HASH.to_string();
    for record in &snapshot.records {
        if record.hash.is_empty() {
            return CheckResult::fail(
                "hash_linkage",
                format!("record seq {} has an empty content hash", record.seq),
            );
        }
        if record.prev_hash != prev_hash {
            return CheckResult::fail(
                "hash_linkage",
                format!(
                    "record seq {} prev_hash does not link to the previous record",
                    record.seq
                ),
            );
        }
        prev_hash = record.hash.clone();
    }
    CheckResult::pass("hash_linkage", "every record links to its predecessor")
}

/// Grants and action manifests may only reference sessions that were already
/// introduced by a `SessionCreated` event earlier in the journal.
fn check_referential_sessions(snapshot: &JournalSnapshot) -> CheckResult {
    let mut known_sessions: BTreeSet<&str> = BTreeSet::new();
    for record in &snapshot.records {
        // (referrer kind, referrer id, referenced session id) that this record
        // requires to already exist. `SessionCreated` introduces a session
        // instead of referencing one.
        let reference: Option<(&str, &str, &str)> = match &record.event {
            JournalEvent::SessionCreated { session } => {
                known_sessions.insert(session.session_id.as_str());
                None
            }
            JournalEvent::CapabilityGranted { grant } => {
                Some(("grant", grant.grant_id.as_str(), grant.session_id.as_str()))
            }
            JournalEvent::ActionProposed { manifest } => Some((
                "action",
                manifest.action_id.as_str(),
                manifest.session_id.as_str(),
            )),
            _ => None,
        };
        if let Some((kind, id, session_id)) = reference
            && !known_sessions.contains(session_id)
        {
            return CheckResult::fail(
                "referential_sessions",
                format!("{kind} {id} references unknown session {session_id}"),
            );
        }
    }
    CheckResult::pass(
        "referential_sessions",
        "all grants and actions reference known sessions",
    )
}

/// Every grant named in a manifest's `required_grants` must have been issued by
/// a prior `CapabilityGranted` event. No authority may be conjured mid-trace.
fn check_grant_references(snapshot: &JournalSnapshot) -> CheckResult {
    let mut granted: BTreeSet<&str> = BTreeSet::new();
    for record in &snapshot.records {
        match &record.event {
            JournalEvent::CapabilityGranted { grant } => {
                granted.insert(grant.grant_id.as_str());
            }
            JournalEvent::ActionProposed { manifest } => {
                for required in &manifest.required_grants {
                    if !granted.contains(required.as_str()) {
                        return CheckResult::fail(
                            "grant_references",
                            format!(
                                "action {} requires grant {} that was never issued",
                                manifest.action_id, required
                            ),
                        );
                    }
                }
            }
            _ => {}
        }
    }
    CheckResult::pass(
        "grant_references",
        "every required grant was issued before use",
    )
}

/// A receipt may only exist for an action that was proposed and then allowed by
/// a policy decision earlier in the journal (independent of the core check).
fn check_receipt_causality(snapshot: &JournalSnapshot) -> CheckResult {
    let mut proposed: BTreeSet<&str> = BTreeSet::new();
    let mut allowed: BTreeSet<&str> = BTreeSet::new();
    for record in &snapshot.records {
        match &record.event {
            JournalEvent::ActionProposed { manifest } => {
                proposed.insert(manifest.action_id.as_str());
            }
            JournalEvent::PolicyDecided { decision } => {
                if decision.result == DecisionResult::Allowed {
                    allowed.insert(decision.action_id.as_str());
                } else {
                    allowed.remove(decision.action_id.as_str());
                }
            }
            JournalEvent::ReceiptAppended { receipt } => {
                if !proposed.contains(receipt.action_id.as_str()) {
                    return CheckResult::fail(
                        "receipt_causality",
                        format!(
                            "receipt {} references action {} that was never proposed",
                            receipt.receipt_id, receipt.action_id
                        ),
                    );
                }
                if !allowed.contains(receipt.action_id.as_str()) {
                    return CheckResult::fail(
                        "receipt_causality",
                        format!(
                            "receipt {} references action {} without a prior allowed decision",
                            receipt.receipt_id, receipt.action_id
                        ),
                    );
                }
            }
            _ => {}
        }
    }
    CheckResult::pass(
        "receipt_causality",
        "every receipt follows a proposed and allowed action",
    )
}

/// Any decision that is not `Allowed` must carry a human-readable explanation.
/// `final.md` §22.9 names "it cannot explain denials" as a failure mode.
fn check_denial_explained(snapshot: &JournalSnapshot) -> CheckResult {
    for record in &snapshot.records {
        if let JournalEvent::PolicyDecided { decision } = &record.event
            && decision.result != DecisionResult::Allowed
            && decision.explanation.trim().is_empty()
        {
            return CheckResult::fail(
                "denial_explained",
                format!(
                    "decision {} on action {} is not allowed but has no explanation",
                    decision.decision_id, decision.action_id
                ),
            );
        }
    }
    CheckResult::pass(
        "denial_explained",
        "every non-allowed decision carries an explanation",
    )
}

#[cfg(test)]
mod tests {
    use super::*;
    use beater_os_core::JournalSnapshot;

    #[test]
    fn empty_snapshot_passes_all_checks() {
        let snapshot = JournalSnapshot::default();
        let report = verify_snapshot(&snapshot);
        assert_eq!(report.records, 0);
        assert!(report.ok, "empty journal should pass: {:?}", report.checks);
        assert_eq!(report.checks.len(), 7);
        assert_eq!(report.failures().count(), 0);
    }
}
