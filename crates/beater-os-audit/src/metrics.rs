//! Audit coverage metrics over a journal snapshot.
//!
//! `final.md` §23.3 lists observability metrics a reviewer cares about: trace
//! completeness, receipt completeness, and policy-explanation coverage. This
//! module derives them from the journal. Coverage is expressed as an exact
//! `covered / total` ratio rather than a lossy float, so results are stable and
//! easy to assert on.

use std::collections::BTreeSet;

use beater_os_core::{DecisionResult, JournalEvent, JournalSnapshot};
use serde::Serialize;

/// An exact coverage ratio. A zero total is treated as vacuously complete.
#[derive(Clone, Copy, Debug, PartialEq, Eq, Serialize)]
pub struct Coverage {
    pub covered: usize,
    pub total: usize,
}

impl Coverage {
    fn new(covered: usize, total: usize) -> Self {
        Self { covered, total }
    }

    /// True when everything that could be covered is covered (or nothing was
    /// expected). Fails closed for negative gaps by construction (`covered`
    /// never exceeds `total` in this module).
    pub fn is_complete(&self) -> bool {
        self.covered >= self.total
    }

    /// Fraction in `[0.0, 1.0]`; `1.0` when there was nothing to cover.
    pub fn as_fraction(&self) -> f64 {
        if self.total == 0 {
            1.0
        } else {
            self.covered as f64 / self.total as f64
        }
    }
}

/// Aggregate audit metrics for a snapshot.
#[derive(Clone, Debug, PartialEq, Eq, Serialize)]
pub struct AuditMetrics {
    pub records: usize,
    pub sessions: usize,
    pub grants: usize,
    pub actions_proposed: usize,
    pub decisions: usize,
    pub allowed_actions: usize,
    pub gated_or_denied_decisions: usize,
    pub receipts: usize,
    /// Fraction of proposed actions that received at least one policy decision.
    pub decision_coverage: Coverage,
    /// Fraction of allowed actions that produced at least one receipt.
    pub receipt_coverage: Coverage,
    /// Fraction of non-allowed decisions that carry a non-empty explanation.
    pub denial_explanation_coverage: Coverage,
}

/// Compute [`AuditMetrics`] for `snapshot`. Read-only and deterministic.
pub fn compute_metrics(snapshot: &JournalSnapshot) -> AuditMetrics {
    let mut sessions = 0usize;
    let mut grants = 0usize;
    let mut receipts = 0usize;

    let mut proposed_actions: BTreeSet<&str> = BTreeSet::new();
    let mut decided_actions: BTreeSet<&str> = BTreeSet::new();
    let mut allowed_actions: BTreeSet<&str> = BTreeSet::new();
    let mut receipted_actions: BTreeSet<&str> = BTreeSet::new();

    let mut decisions = 0usize;
    let mut gated_or_denied = 0usize;
    let mut gated_or_denied_explained = 0usize;

    for record in &snapshot.records {
        match &record.event {
            JournalEvent::SessionCreated { .. } => sessions += 1,
            JournalEvent::CapabilityGranted { .. } => grants += 1,
            JournalEvent::ActionProposed { manifest } => {
                proposed_actions.insert(manifest.action_id.as_str());
            }
            JournalEvent::PolicyDecided { decision } => {
                decisions += 1;
                decided_actions.insert(decision.action_id.as_str());
                if decision.result == DecisionResult::Allowed {
                    allowed_actions.insert(decision.action_id.as_str());
                } else {
                    allowed_actions.remove(decision.action_id.as_str());
                    gated_or_denied += 1;
                    if !decision.explanation.trim().is_empty() {
                        gated_or_denied_explained += 1;
                    }
                }
            }
            JournalEvent::ReceiptAppended { receipt } => {
                receipts += 1;
                receipted_actions.insert(receipt.action_id.as_str());
            }
            _ => {}
        }
    }

    let decisions_for_proposed = proposed_actions
        .iter()
        .filter(|action| decided_actions.contains(*action))
        .count();
    let receipts_for_allowed = allowed_actions
        .iter()
        .filter(|action| receipted_actions.contains(*action))
        .count();

    AuditMetrics {
        records: snapshot.records.len(),
        sessions,
        grants,
        actions_proposed: proposed_actions.len(),
        decisions,
        allowed_actions: allowed_actions.len(),
        gated_or_denied_decisions: gated_or_denied,
        receipts,
        decision_coverage: Coverage::new(decisions_for_proposed, proposed_actions.len()),
        receipt_coverage: Coverage::new(receipts_for_allowed, allowed_actions.len()),
        denial_explanation_coverage: Coverage::new(gated_or_denied_explained, gated_or_denied),
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use beater_os_core::JournalSnapshot;

    #[test]
    fn empty_snapshot_is_vacuously_complete() {
        let metrics = compute_metrics(&JournalSnapshot::default());
        assert_eq!(metrics.records, 0);
        assert!(metrics.decision_coverage.is_complete());
        assert!(metrics.receipt_coverage.is_complete());
        assert!(metrics.denial_explanation_coverage.is_complete());
        assert!((metrics.decision_coverage.as_fraction() - 1.0).abs() < f64::EPSILON);
    }
}
