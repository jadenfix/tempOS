//! Full replay trace bundle export.
//!
//! This module is intentionally separate from [`crate::AuditBundle`]. The audit
//! bundle is redaction-safe and digest-only; this trace bundle is the complete
//! replay artifact shaped like `contracts/schema/trace-bundle.schema.json`.
//! Callers should only expose it across already-authorized audit/control-plane
//! boundaries because manifests, targets, summaries, and journal payloads may
//! contain sensitive data.

use beater_os_core::{
    ActionManifest, AgentSession, ApprovalEvidence, CapabilityGrant, CapabilityReceipt,
    InMemoryJournal, JournalEvent, JournalRecord, JournalSnapshot, PaymentMandate, PolicyDecision,
    ReceiptLedger, SessionStatus, SimulationEvidence,
};
use serde::{Deserialize, Serialize};

use crate::verify::{
    CheckOutcome, CheckResult, snapshot_root_hash, verify_expected_root, verify_snapshot,
};

/// A self-contained trace for one session run plus its hash-linked journal and
/// receipt chains.
#[derive(Clone, Debug, PartialEq, Eq, Serialize, Deserialize)]
pub struct TraceBundle {
    pub bundle_id: String,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub description: Option<String>,
    pub policy_version: String,
    pub sessions: Vec<AgentSession>,
    pub grants: Vec<CapabilityGrant>,
    pub payment_mandates: Vec<PaymentMandate>,
    pub approvals: Vec<ApprovalEvidence>,
    pub simulations: Vec<SimulationEvidence>,
    pub manifests: Vec<ActionManifest>,
    pub decisions: Vec<PolicyDecision>,
    pub receipts: Vec<CapabilityReceipt>,
    pub journal: Vec<JournalRecord>,
}

/// Serialize a full trace bundle to pretty JSON.
pub fn trace_bundle_to_json(bundle: &TraceBundle) -> Result<String, serde_json::Error> {
    serde_json::to_string_pretty(bundle)
}

/// Options for read-only trace bundle verification.
#[derive(Clone, Debug, Default, PartialEq, Eq)]
pub struct TraceBundleVerifyOptions<'a> {
    pub expected_journal_root: Option<&'a str>,
}

/// Result of verifying a full trace bundle as a read-only audit artifact.
#[derive(Clone, Debug, PartialEq, Eq, Serialize)]
pub struct TraceBundleVerificationReport {
    pub ok: bool,
    pub bundle_id: String,
    pub session_id: Option<String>,
    pub records: usize,
    pub journal_root_hash: String,
    pub receipt_root_hash: String,
    pub grants: usize,
    pub payment_mandates: usize,
    pub approvals: usize,
    pub simulations: usize,
    pub manifests: usize,
    pub decisions: usize,
    pub receipts: usize,
    pub checks: Vec<CheckResult>,
}

#[derive(Default)]
struct ProjectedTrace {
    sessions: Vec<AgentSession>,
    grants: Vec<CapabilityGrant>,
    payment_mandates: Vec<PaymentMandate>,
    approvals: Vec<ApprovalEvidence>,
    simulations: Vec<SimulationEvidence>,
    manifests: Vec<ActionManifest>,
    decisions: Vec<PolicyDecision>,
    receipts: Vec<CapabilityReceipt>,
}

/// Verify that a trace bundle is a faithful, read-only audit artifact.
///
/// The top-level arrays are treated as redundant evidence. The journal is the
/// only authority source: verification rebuilds projection state from
/// `bundle.journal`, verifies the embedded journal, rebuilds the receipt chain
/// from `ReceiptAppended` events, and rejects any mismatch between the
/// journal-derived projection and exported arrays. This never applies/imports a
/// bundle into daemon state and never re-runs policy admission or tools.
pub fn verify_trace_bundle(bundle: &TraceBundle) -> TraceBundleVerificationReport {
    verify_trace_bundle_with_options(bundle, TraceBundleVerifyOptions::default())
}

pub fn verify_trace_bundle_with_options(
    bundle: &TraceBundle,
    options: TraceBundleVerifyOptions<'_>,
) -> TraceBundleVerificationReport {
    let snapshot = trace_bundle_snapshot(bundle);
    let mut checks = verify_snapshot(&snapshot).checks;
    if bundle.bundle_id.trim().is_empty() {
        checks.push(check_fail(
            "trace_bundle_id",
            "trace bundle id must not be empty",
        ));
    } else {
        checks.push(check_pass(
            "trace_bundle_id",
            format!("trace bundle id {} is present", bundle.bundle_id),
        ));
    }
    let journal = InMemoryJournal::from_records(bundle.journal.clone());
    let core_verification = journal.verify_chain();
    let records = bundle.journal.len();
    let journal_root_hash = snapshot_root_hash(&snapshot);
    match core_verification {
        Ok(report) => checks.push(check_pass(
            "core_journal_causality",
            format!(
                "core journal verifier accepted {} record(s)",
                report.records
            ),
        )),
        Err(err) => checks.push(check_fail(
            "core_journal_causality",
            format!("core journal verifier rejected bundle: {err}"),
        )),
    }
    if let Some(expected_root) = options.expected_journal_root {
        checks.push(verify_expected_root(&snapshot, expected_root));
    }
    let projected = match project_trace_from_journal(&bundle.journal) {
        Ok(projected) => {
            checks.push(check_pass(
                "journal_projection",
                "journal-derived projection completed",
            ));
            projected
        }
        Err(err) => {
            checks.push(check_fail("journal_projection", err));
            ProjectedTrace::default()
        }
    };
    if projected.sessions.len() == 1 {
        checks.push(check_pass(
            "single_session_trace",
            "trace bundle projects exactly one session",
        ));
    } else {
        checks.push(check_fail(
            "single_session_trace",
            format!(
                "trace bundle verifier currently requires exactly one session, found {}",
                projected.sessions.len()
            ),
        ));
    }
    push_section_check(
        &mut checks,
        "sessions",
        &bundle.sessions,
        &projected.sessions,
    );
    push_section_check(&mut checks, "grants", &bundle.grants, &projected.grants);
    push_section_check(
        &mut checks,
        "payment_mandates",
        &bundle.payment_mandates,
        &projected.payment_mandates,
    );
    push_section_check(
        &mut checks,
        "approvals",
        &bundle.approvals,
        &projected.approvals,
    );
    push_section_check(
        &mut checks,
        "simulations",
        &bundle.simulations,
        &projected.simulations,
    );
    push_section_check(
        &mut checks,
        "manifests",
        &bundle.manifests,
        &projected.manifests,
    );
    push_section_check(
        &mut checks,
        "decisions",
        &bundle.decisions,
        &projected.decisions,
    );
    push_section_check(
        &mut checks,
        "receipts",
        &bundle.receipts,
        &projected.receipts,
    );
    let receipt_ledger = ReceiptLedger::from_receipts(projected.receipts.clone());
    match receipt_ledger.verify_chain() {
        Ok(()) => checks.push(check_pass(
            "receipt_chain",
            "journal-derived receipt chain verified",
        )),
        Err(err) => checks.push(check_fail(
            "receipt_chain",
            format!("journal-derived receipt chain failed verification: {err}"),
        )),
    }
    let ok = checks
        .iter()
        .all(|check| check.outcome == CheckOutcome::Pass);
    TraceBundleVerificationReport {
        ok,
        bundle_id: bundle.bundle_id.clone(),
        session_id: projected
            .sessions
            .first()
            .map(|session| session.session_id.clone()),
        records,
        journal_root_hash,
        receipt_root_hash: receipt_ledger.root_hash(),
        grants: projected.grants.len(),
        payment_mandates: projected.payment_mandates.len(),
        approvals: projected.approvals.len(),
        simulations: projected.simulations.len(),
        manifests: projected.manifests.len(),
        decisions: projected.decisions.len(),
        receipts: projected.receipts.len(),
        checks,
    }
}

pub fn trace_bundle_snapshot(bundle: &TraceBundle) -> JournalSnapshot {
    JournalSnapshot {
        records: bundle.journal.clone(),
    }
}

fn project_trace_from_journal(records: &[JournalRecord]) -> Result<ProjectedTrace, String> {
    let mut projected = ProjectedTrace::default();
    for record in records {
        match &record.event {
            JournalEvent::SessionCreated { session } => projected.sessions.push(session.clone()),
            JournalEvent::SessionStatusChanged { session_id, to, .. } => {
                let Some(session) = projected
                    .sessions
                    .iter_mut()
                    .find(|session| session.session_id == *session_id)
                else {
                    return Err(format!(
                        "session status transition references missing session {session_id}",
                    ));
                };
                session.status = to.clone();
            }
            JournalEvent::CapabilityGranted { grant } => projected.grants.push(grant.clone()),
            JournalEvent::CapabilityRevoked {
                grant_id,
                revocation_handle,
                ..
            } => {
                let Some(_grant) = projected.grants.iter().find(|grant| {
                    grant.grant_id == *grant_id && grant.revocation_handle == *revocation_handle
                }) else {
                    return Err(format!(
                        "grant revocation references missing grant {grant_id}",
                    ));
                };
                // Trace export serializes the live projection's `grants` array
                // without folding `revoked_handles` back into each grant. Keep
                // this verifier byte-contract aligned with that exported shape;
                // the revocation itself remains authoritative evidence in the
                // journal that is independently verified above.
            }
            JournalEvent::PaymentMandateIssued { mandate } => {
                projected.payment_mandates.push(mandate.clone());
            }
            JournalEvent::ActionProposed { manifest } => {
                projected.manifests.push((**manifest).clone());
            }
            JournalEvent::PolicyDecided { decision } => projected.decisions.push(decision.clone()),
            JournalEvent::ExecutionLeaseIssued { .. }
            | JournalEvent::ExecutionLeaseReconciled { .. } => {}
            JournalEvent::ApprovalRecorded { approval } => {
                projected.approvals.push(approval.clone());
            }
            JournalEvent::SimulationRecorded { simulation } => {
                projected.simulations.push(simulation.clone());
            }
            JournalEvent::ReceiptAppended { receipt } => projected.receipts.push(receipt.clone()),
            JournalEvent::MemoryWritten { .. }
            | JournalEvent::ScenarioEvaluated { .. }
            | JournalEvent::IncidentAnnotated { .. } => {}
        }
    }
    if projected
        .sessions
        .iter()
        .any(|session| session.status != SessionStatus::Running)
    {
        projected
            .sessions
            .sort_by(|a, b| a.session_id.cmp(&b.session_id));
    }
    Ok(projected)
}

fn push_section_check<T: PartialEq>(
    checks: &mut Vec<CheckResult>,
    name: &str,
    exported: &[T],
    projected: &[T],
) {
    if exported == projected {
        checks.push(check_pass(
            &format!("trace_bundle_{name}"),
            format!("{name} section matches journal-derived projection"),
        ));
    } else {
        checks.push(check_fail(
            &format!("trace_bundle_{name}"),
            format!("{name} section does not match journal-derived projection"),
        ));
    }
}

fn check_pass(check: &str, detail: impl Into<String>) -> CheckResult {
    CheckResult {
        check: check.to_string(),
        outcome: CheckOutcome::Pass,
        detail: detail.into(),
    }
}

fn check_fail(check: &str, detail: impl Into<String>) -> CheckResult {
    CheckResult {
        check: check.to_string(),
        outcome: CheckOutcome::Fail,
        detail: detail.into(),
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn empty_payload_sections_remain_present() {
        let bundle = TraceBundle {
            bundle_id: "trace-empty".to_string(),
            description: None,
            policy_version: "policy-test".to_string(),
            sessions: Vec::new(),
            grants: Vec::new(),
            payment_mandates: Vec::new(),
            approvals: Vec::new(),
            simulations: Vec::new(),
            manifests: Vec::new(),
            decisions: Vec::new(),
            receipts: Vec::new(),
            journal: Vec::new(),
        };
        let json = trace_bundle_to_json(&bundle).unwrap_or_else(|err| err.to_string());
        assert!(json.contains("\"bundle_id\""));
        assert!(json.contains("\"sessions\""));
        assert!(json.contains("\"journal\""));
        assert!(!json.contains("\"description\""));
    }
}
