//! Human-legible rendering of a journal snapshot.
//!
//! `final.md` §25 (step 9) calls for a trace viewer, and §17.4 lists the
//! questions a reviewer needs answered: "what did it already do", "what
//! changed", "why did policy allow or deny this". This module turns an
//! append-only journal into a compact timeline that answers them without
//! requiring the reader to parse JSON or understand the model.

use std::fmt::Write as _;

use beater_os_core::{JournalEvent, JournalSnapshot};

use crate::events::event_kind;

/// Render `snapshot` as a legible, line-per-event timeline.
///
/// The output is deterministic and side-effect free. Enum values are shown via
/// their `Debug` form, which is stable and readable for a reviewer.
pub fn render_trace(snapshot: &JournalSnapshot) -> String {
    let mut out = String::new();
    if snapshot.records.is_empty() {
        out.push_str("(empty journal — no events)\n");
        return out;
    }
    for record in &snapshot.records {
        // Header: sequence, timestamp, kind. `write!` to a String cannot fail.
        let _ = write!(
            out,
            "#{:<3} {}  {:<18}",
            record.seq,
            record.created_at.to_rfc3339(),
            event_kind(&record.event),
        );
        out.push(' ');
        out.push_str(&summarize_event(&record.event));
        out.push('\n');
    }
    out
}

fn summarize_event(event: &JournalEvent) -> String {
    match event {
        JournalEvent::SessionCreated { session } => format!(
            "session={} agent={} by={} goal={:?}",
            session.session_id, session.agent_id, session.created_by, session.goal
        ),
        JournalEvent::SessionStatusChanged {
            transition_id,
            session_id,
            from,
            to,
        } => format!(
            "transition={} session={} {:?}->{:?}",
            transition_id, session_id, from, to
        ),
        JournalEvent::CapabilityGranted { grant } => format!(
            "grant={} holder={} scope={:?}:{} actions={:?} expires={}",
            grant.grant_id,
            grant.holder,
            grant.scope.selector.resource_kind,
            grant.scope.selector.resource_id,
            grant.scope.actions,
            grant.expires_at.to_rfc3339(),
        ),
        JournalEvent::CapabilityRevoked {
            grant_id,
            revocation_handle,
            revoked_by,
            reason,
        } => format!(
            "grant={} handle={} revoked_by={} reason={:?}",
            grant_id, revocation_handle, revoked_by, reason
        ),
        JournalEvent::PaymentMandateIssued { mandate } => format!(
            "mandate={} holder={} rail={} asset={} max={} adapters={:?} formats={:?} expires={}",
            mandate.mandate_id,
            mandate.holder,
            mandate.rail,
            mandate.asset,
            mandate.max_minor_units,
            mandate.allowed_adapter_ids,
            mandate.allowed_envelope_formats,
            mandate.expires_at.to_rfc3339(),
        ),
        JournalEvent::ActionProposed { manifest } => {
            let resolved = manifest
                .resolved_target
                .as_ref()
                .map(|target| {
                    format!(
                        " resolved={:?}:{}",
                        target.resource_kind, target.resource_id
                    )
                })
                .unwrap_or_default();
            format!(
                "action={} tool={} kind={:?} target={:?}:{}{} risk={:?} grants={:?}",
                manifest.action_id,
                manifest.tool_id,
                manifest.action_kind,
                manifest.target.resource_kind,
                manifest.target.resource_id,
                resolved,
                manifest.risk_class,
                manifest.required_grants,
            )
        }
        JournalEvent::PolicyDecided { decision } => format!(
            "decision={} action={} result={:?} why={:?}",
            decision.decision_id, decision.action_id, decision.result, decision.explanation
        ),
        JournalEvent::ExecutionLeaseIssued { lease } => format!(
            "lease={} action={} tool_ref={} target={:?}:{} expires={}",
            lease.lease_id,
            lease.action_id,
            lease.tool_ref,
            lease.target.resource_kind,
            lease.target.resource_id,
            lease.expires_at.to_rfc3339()
        ),
        JournalEvent::ExecutionLeaseHeartbeated { heartbeat } => format!(
            "heartbeat={} lease={} action={} previous_expires={} renewed_until={} by={}",
            heartbeat.heartbeat_id,
            heartbeat.lease_id,
            heartbeat.action_id,
            heartbeat.previous_expires_at.to_rfc3339(),
            heartbeat.extended_expires_at.to_rfc3339(),
            heartbeat.observed_by
        ),
        JournalEvent::ExecutionLeaseReconciled { reconciliation } => format!(
            "reconciliation={} lease={} action={} resolution={:?} by={} reason={:?}",
            reconciliation.reconciliation_id,
            reconciliation.lease_id,
            reconciliation.action_id,
            reconciliation.resolution,
            reconciliation.reconciled_by,
            reconciliation.reason
        ),
        JournalEvent::ApprovalRecorded { approval } => format!(
            "approval={} action={} grant={} reviewer={}",
            approval.review_id, approval.action_id, approval.grant_id, approval.reviewer_id
        ),
        JournalEvent::SimulationRecorded { simulation } => format!(
            "simulation={} action={} scenario={}",
            simulation.simulation_id, simulation.action_id, simulation.scenario_id
        ),
        JournalEvent::ReceiptAppended { receipt } => format!(
            "receipt={} action={} status={} effects={:?}",
            receipt.receipt_id, receipt.action_id, receipt.status, receipt.side_effects
        ),
        JournalEvent::MemoryWritten { memory } => format!(
            "memory={} kind={} sensitivity={:?} writer={}",
            memory.memory_id, memory.kind, memory.sensitivity, memory.writer
        ),
        JournalEvent::ScenarioEvaluated { scenario, passed } => {
            format!("scenario={} passed={}", scenario.scenario_id, passed)
        }
        JournalEvent::IncidentAnnotated { incident_id, note } => {
            format!("incident={incident_id} note={note:?}")
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use beater_os_core::JournalSnapshot;

    #[test]
    fn empty_journal_renders_placeholder() {
        let snapshot = JournalSnapshot::default();
        let rendered = render_trace(&snapshot);
        assert!(rendered.contains("empty journal"));
    }
}
