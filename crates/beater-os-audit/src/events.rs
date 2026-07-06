//! Small shared helpers over `beater-os-core` journal events.

use beater_os_core::JournalEvent;

/// Stable, lowercase kind name for a journal event. Used by the trace renderer
/// and the redaction-safe bundle so both agree on a single vocabulary.
pub(crate) fn event_kind(event: &JournalEvent) -> &'static str {
    match event {
        JournalEvent::SessionCreated { .. } => "session_created",
        JournalEvent::CapabilityGranted { .. } => "capability_granted",
        JournalEvent::PaymentMandateIssued { .. } => "payment_mandate_issued",
        JournalEvent::ActionProposed { .. } => "action_proposed",
        JournalEvent::PolicyDecided { .. } => "policy_decided",
        JournalEvent::ApprovalRecorded { .. } => "approval_recorded",
        JournalEvent::SimulationRecorded { .. } => "simulation_recorded",
        JournalEvent::ReceiptAppended { .. } => "receipt_appended",
        JournalEvent::MemoryWritten { .. } => "memory_written",
        JournalEvent::ScenarioEvaluated { .. } => "scenario_evaluated",
        JournalEvent::IncidentAnnotated { .. } => "incident_annotated",
    }
}
