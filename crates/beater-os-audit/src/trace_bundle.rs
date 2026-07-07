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
    JournalRecord, PaymentMandate, PolicyDecision, SimulationEvidence,
};
use serde::Serialize;

/// A self-contained trace for one session run plus its hash-linked journal and
/// receipt chains.
#[derive(Clone, Debug, PartialEq, Eq, Serialize)]
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
