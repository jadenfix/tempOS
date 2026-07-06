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
        JournalEvent::CapabilityGranted { grant } => format!(
            "grant={} holder={} scope={:?}:{} actions={:?} expires={}",
            grant.grant_id,
            grant.holder,
            grant.scope.selector.resource_kind,
            grant.scope.selector.resource_id,
            grant.scope.actions,
            grant.expires_at.to_rfc3339(),
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
        JournalEvent::ActionProposed { manifest } => format!(
            "action={} tool={} kind={:?} target={:?}:{} risk={:?} grants={:?}",
            manifest.action_id,
            manifest.tool_id,
            manifest.action_kind,
            manifest.target.resource_kind,
            manifest.target.resource_id,
            manifest.risk_class,
            manifest.required_grants,
        ),
        JournalEvent::PolicyDecided { decision } => format!(
            "decision={} action={} result={:?} why={:?}",
            decision.decision_id, decision.action_id, decision.result, decision.explanation
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
