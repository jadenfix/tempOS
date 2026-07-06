//! Independent verification of a beaterOS journal snapshot.
//!
//! `final.md` §8.15 argues for a small trusted computing base that can be
//! re-verified, and §13.11 requires tamper-evident logs. This module is a
//! deliberately *independent* second implementation of the audit invariants:
//! it recomputes each record's content hash itself (its own SHA-256 over the
//! canonical pre-image), never trusting the digest emitted by the code under
//! audit, then applies its own structural and cross-referential checks. It does
//! not call `beater-os-core`'s verifier for any pass/fail signal — a drift in
//! core's hashing would surface here as recomputed-hash mismatches on otherwise
//! valid records, i.e. loud audit failures, rather than being silently trusted.

use std::collections::{BTreeMap, BTreeSet};

use beater_os_core::{
    CapabilityGrant, DecisionResult, JournalEvent, JournalRecord, JournalSnapshot, SessionStatus,
};
use chrono::{DateTime, Utc};
use serde::Serialize;
use sha2::{Digest, Sha256};

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
        // Independent content-hash integrity — recomputes every record's hash
        // from the canonical pre-image using this crate's own SHA-256. This is
        // the load-bearing integrity signal; the structural checks below trust
        // `record.hash` and only add linkage.
        check_cryptographic_chain(snapshot),
        // Independent second implementations of structural invariants. The overlap
        // with `beater-os-core` is intentional (defense in depth): they catch a
        // core regression, not a gap in core.
        check_sequence_contiguous(snapshot),
        check_hash_linkage(snapshot),
        check_receipt_causality(snapshot),
        check_lifecycle_causality(snapshot),
        check_memory_provenance(snapshot),
        // Novel gap-fillers — invariants the core journal verifier does NOT check.
        check_referential_sessions(snapshot),
        check_grant_references(snapshot),
        check_grant_validity(snapshot),
        check_denial_explained(snapshot),
    ];
    let ok = checks.iter().all(|c| c.outcome == CheckOutcome::Pass);
    AuditReport {
        records: snapshot.records.len(),
        ok,
        checks,
    }
}

fn check_lifecycle_causality(snapshot: &JournalSnapshot) -> CheckResult {
    let mut statuses: BTreeMap<&str, &SessionStatus> = BTreeMap::new();
    for record in &snapshot.records {
        match &record.event {
            JournalEvent::SessionCreated { session } => {
                if statuses
                    .insert(session.session_id.as_str(), &session.status)
                    .is_some()
                {
                    return CheckResult::fail(
                        "lifecycle_causality",
                        format!("session {} was created more than once", session.session_id),
                    );
                }
            }
            JournalEvent::SessionStatusChanged {
                transition_id,
                session_id,
                from,
                to,
            } => {
                let Some(current) = statuses.get(session_id.as_str()) else {
                    return CheckResult::fail(
                        "lifecycle_causality",
                        format!(
                            "transition {transition_id} references unknown session {session_id}"
                        ),
                    );
                };
                if *current != from {
                    return CheckResult::fail(
                        "lifecycle_causality",
                        format!(
                            "transition {transition_id} from {from:?} does not match current status {current:?}"
                        ),
                    );
                }
                if !valid_session_transition(from, to) {
                    return CheckResult::fail(
                        "lifecycle_causality",
                        format!("illegal session transition {transition_id}: {from:?} -> {to:?}"),
                    );
                }
                statuses.insert(session_id.as_str(), to);
            }
            _ => {}
        }
    }
    CheckResult::pass(
        "lifecycle_causality",
        "session status transitions follow the legal state machine",
    )
}

fn check_memory_provenance(snapshot: &JournalSnapshot) -> CheckResult {
    let mut event_ids: BTreeSet<&str> = BTreeSet::new();
    for record in &snapshot.records {
        if let JournalEvent::MemoryWritten { memory } = &record.event {
            if memory.source_event_id.trim().is_empty() {
                return CheckResult::fail(
                    "memory_provenance",
                    format!("memory {} has an empty source_event_id", memory.memory_id),
                );
            }
            if !event_ids.contains(memory.source_event_id.as_str()) {
                return CheckResult::fail(
                    "memory_provenance",
                    format!(
                        "memory {} references unknown source event {}",
                        memory.memory_id, memory.source_event_id
                    ),
                );
            }
        }
        if let Some(event_id) = primary_event_id(record)
            && !event_ids.insert(event_id)
        {
            return CheckResult::fail(
                "memory_provenance",
                format!("journal event id {event_id} appears more than once"),
            );
        }
    }
    CheckResult::pass(
        "memory_provenance",
        "memory records reference prior unambiguous journal events",
    )
}

/// Canonical hash pre-image for a journal record.
///
/// This is an independent re-declaration of the exact field set and order that
/// `beater-os-core` hashes (`seq`, `created_at`, `event`, `prev_hash`). It is
/// duplicated here on purpose: an independent auditor must serialize and hash
/// the record itself rather than importing the hasher under audit. If core ever
/// changes its pre-image, this struct must change with it and the cross-check in
/// [`check_cryptographic_chain`] will flag the divergence until it does.
#[derive(Serialize)]
struct JournalHashPreimage<'a> {
    seq: u64,
    created_at: &'a DateTime<Utc>,
    event: &'a JournalEvent,
    prev_hash: &'a str,
}

/// Recompute a record's content hash from scratch: SHA-256 over the canonical
/// JSON pre-image, hex-encoded. No dependency on `beater-os-core`'s hasher.
fn recompute_record_hash(record: &JournalRecord) -> Result<String, serde_json::Error> {
    let preimage = JournalHashPreimage {
        seq: record.seq,
        created_at: &record.created_at,
        event: &record.event,
        prev_hash: record.prev_hash.as_str(),
    };
    let bytes = serde_json::to_vec(&preimage)?;
    let mut hasher = Sha256::new();
    hasher.update(&bytes);
    Ok(hex::encode(hasher.finalize()))
}

/// Independently verify the cryptographic hash chain.
///
/// Unlike the structural checks below, this does not trust `record.hash`: it
/// recomputes each record's content hash locally (closing the terminal-record
/// blind spot in [`check_hash_linkage`], which has no successor to catch a
/// tampered last record). It calls nothing in `beater-os-core` for its verdict.
/// Fails closed on any re-serialization error.
fn check_cryptographic_chain(snapshot: &JournalSnapshot) -> CheckResult {
    let mut prev_hash = GENESIS_HASH;
    for record in &snapshot.records {
        if record.prev_hash != prev_hash {
            return CheckResult::fail(
                "cryptographic_chain",
                format!(
                    "record seq {} prev_hash {} does not link to expected {prev_hash}",
                    record.seq, record.prev_hash
                ),
            );
        }
        let recomputed = match recompute_record_hash(record) {
            Ok(hash) => hash,
            Err(err) => {
                return CheckResult::fail(
                    "cryptographic_chain",
                    format!(
                        "record seq {} could not be re-serialized for hashing: {err}",
                        record.seq
                    ),
                );
            }
        };
        if recomputed != record.hash {
            return CheckResult::fail(
                "cryptographic_chain",
                format!(
                    "record seq {} content hash mismatch: independently recomputed {recomputed}, stored {}",
                    record.seq, record.hash
                ),
            );
        }
        prev_hash = &record.hash;
    }

    CheckResult::pass(
        "cryptographic_chain",
        format!(
            "independently recomputed and linked {} record hash(es)",
            snapshot.records.len()
        ),
    )
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
///
/// This checks prev-hash *linkage*, not content-hash *integrity*: a chain that
/// was consistently re-hashed after tampering would pass here and be caught only
/// by [`check_cryptographic_chain`]. The overlap with `beater-os-core` is a
/// deliberate independent second implementation (defense in depth) — do not
/// "simplify" it away; if the two ever disagree, that is an auditable incident.
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
    let mut transition_ids: BTreeSet<&str> = BTreeSet::new();
    for record in &snapshot.records {
        // (referrer kind, referrer id, referenced session id) that this record
        // requires to already exist. `SessionCreated` introduces a session
        // instead of referencing one.
        let reference: Option<(&str, &str, &str)> = match &record.event {
            JournalEvent::SessionCreated { session } => {
                known_sessions.insert(session.session_id.as_str());
                None
            }
            JournalEvent::SessionStatusChanged {
                transition_id,
                session_id,
                ..
            } => {
                if transition_id.trim().is_empty() {
                    return CheckResult::fail(
                        "referential_sessions",
                        "session transition id is empty".to_string(),
                    );
                }
                if !transition_ids.insert(transition_id.as_str()) {
                    return CheckResult::fail(
                        "referential_sessions",
                        format!("session transition {transition_id} appears more than once"),
                    );
                }
                Some((
                    "session transition",
                    transition_id.as_str(),
                    session_id.as_str(),
                ))
            }
            JournalEvent::CapabilityGranted { grant } => {
                Some(("grant", grant.grant_id.as_str(), grant.session_id.as_str()))
            }
            JournalEvent::PaymentMandateIssued { mandate } => Some((
                "payment mandate",
                mandate.mandate_id.as_str(),
                mandate.session_id.as_str(),
            )),
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
        "all lifecycle events, grants, and actions reference known sessions",
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

/// A grant named by an action must be neither revoked nor expired at the moment
/// the action is proposed. `final.md` §26 lists revocation as a never-compromise
/// invariant: the core admission path enforces `is_active_at` live, but the
/// offline journal verifier does not re-check it, so an audit would otherwise
/// miss a use-after-revoke or use-after-expiry trace. This re-derives it.
///
/// The journal has no explicit revocation event today, so this reads the
/// `revoked` flag and `expires_at` recorded on the grant at issuance. If a
/// revocation event type is added later, this check should also honor it.
fn check_grant_validity(snapshot: &JournalSnapshot) -> CheckResult {
    let mut grants: BTreeMap<&str, &CapabilityGrant> = BTreeMap::new();
    for record in &snapshot.records {
        match &record.event {
            JournalEvent::CapabilityGranted { grant } => {
                grants.insert(grant.grant_id.as_str(), grant);
            }
            JournalEvent::ActionProposed { manifest } => {
                for required in &manifest.required_grants {
                    // Existence is `grant_references`' job; do not double-report.
                    let Some(grant) = grants.get(required.as_str()) else {
                        continue;
                    };
                    if grant.revoked {
                        return CheckResult::fail(
                            "grant_validity",
                            format!(
                                "action {} uses revoked grant {required}",
                                manifest.action_id
                            ),
                        );
                    }
                    if grant.expires_at <= record.created_at {
                        return CheckResult::fail(
                            "grant_validity",
                            format!(
                                "action {} at {} uses grant {required} that expired at {}",
                                manifest.action_id,
                                record.created_at.to_rfc3339(),
                                grant.expires_at.to_rfc3339()
                            ),
                        );
                    }
                }
            }
            _ => {}
        }
    }
    CheckResult::pass(
        "grant_validity",
        "every required grant is unrevoked and unexpired at use",
    )
}

/// A receipt may only exist for an action that was proposed and then allowed by
/// a policy decision earlier in the journal.
///
/// Unlike the gap-fillers, `beater-os-core`'s journal verifier already enforces
/// this invariant, so this is a redundant second implementation (defense in
/// depth), not a check that catches something core misses.
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

fn valid_session_transition(from: &SessionStatus, to: &SessionStatus) -> bool {
    matches!(
        (from, to),
        (SessionStatus::Running, SessionStatus::Paused)
            | (SessionStatus::Paused, SessionStatus::Running)
            | (SessionStatus::Running, SessionStatus::Canceled)
            | (SessionStatus::Paused, SessionStatus::Canceled)
    )
}

fn primary_event_id(record: &JournalRecord) -> Option<&str> {
    match &record.event {
        JournalEvent::SessionCreated { session } => Some(session.session_id.as_str()),
        JournalEvent::SessionStatusChanged { transition_id, .. } => Some(transition_id.as_str()),
        JournalEvent::CapabilityGranted { grant } => Some(grant.grant_id.as_str()),
        JournalEvent::PaymentMandateIssued { mandate } => Some(mandate.mandate_id.as_str()),
        JournalEvent::ActionProposed { manifest } => Some(manifest.action_id.as_str()),
        JournalEvent::PolicyDecided { decision } => Some(decision.decision_id.as_str()),
        JournalEvent::ApprovalRecorded { approval } => Some(approval.review_id.as_str()),
        JournalEvent::SimulationRecorded { simulation } => Some(simulation.simulation_id.as_str()),
        JournalEvent::ReceiptAppended { receipt } => Some(receipt.receipt_id.as_str()),
        // Memory ids are mutable projection keys: the memory projection allows
        // later writes to replace the same memory_id, so they cannot be used as
        // globally unique journal event ids.
        JournalEvent::MemoryWritten { .. } => None,
        JournalEvent::ScenarioEvaluated { scenario, .. } => Some(scenario.scenario_id.as_str()),
        JournalEvent::IncidentAnnotated { incident_id, .. } => Some(incident_id.as_str()),
    }
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
        assert_eq!(report.checks.len(), 10);
        assert_eq!(report.failures().count(), 0);
    }
}
