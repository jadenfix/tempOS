use std::collections::BTreeSet;
use std::error::Error;
use std::fs;

use beater_os_core::{
    ActionKind, Budget, CapabilitySelector, DataClass, ResourceKind, RiskClass, SideEffectClass,
    TaintLabel,
};
use beater_os_runtime::{
    AgentRuntime, GrantRequest, RuntimeBundle, RuntimeObservation, RuntimeStep, SessionStart,
    default_root_grant_id,
};
use serde_json::json;
use uuid::Uuid;

fn main() -> Result<(), Box<dyn Error>> {
    let mut as_json = false;
    for arg in std::env::args().skip(1) {
        match arg.as_str() {
            "--json" => as_json = true,
            other => return Err(format!("unsupported argument: {other}").into()),
        }
    }

    let root = std::env::temp_dir().join(format!("beater-os-runtime-smoke-{}", Uuid::new_v4()));
    let runtime = AgentRuntime::open(&root)?;
    let session_id = "hosted-runtime-smoke-session".to_string();
    let grant_id = default_root_grant_id(&session_id);
    let target = CapabilitySelector {
        resource_kind: ResourceKind::FilePath,
        resource_id: "/tmp/beater-os-runtime-smoke-observation".to_string(),
    };
    let outcome = runtime.run_bundle(RuntimeBundle {
        session_id: Some(session_id.clone()),
        session: Some(SessionStart::new(
            "agent:hosted-runtime-smoke",
            "workspace:hosted-runtime-smoke",
            "prove hosted runtime bundle orchestration",
        )),
        grants: vec![GrantRequest::new(
            ResourceKind::FilePath,
            "*",
            [ActionKind::Read],
        )],
        steps: vec![RuntimeStep {
            session_id,
            action_id: Some("hosted-runtime-smoke-observe".to_string()),
            tool_id: Some("tool:beater-os-runtime".to_string()),
            action_kind: ActionKind::Read,
            target: target.clone(),
            resolved_target: Some(target),
            inputs_summary: "observe hosted runtime bundle state".to_string(),
            inputs_digest: None,
            expected_outputs: Vec::new(),
            expected_side_effects: BTreeSet::from([SideEffectClass::None]),
            required_grants: BTreeSet::from([grant_id]),
            requested_budget: Budget::default(),
            risk_class: RiskClass::Low,
            data_classes: BTreeSet::from([DataClass::Internal]),
            taint: BTreeSet::from([TaintLabel::TrustedUserInstruction]),
            idempotency_key: Some("hosted-runtime-smoke-observe".to_string()),
            compensation_plan: None,
            human_explanation: "read-only hosted runtime bundle observation".to_string(),
            external_revoked_handles: BTreeSet::new(),
            observation: Some(RuntimeObservation::ok("hosted runtime bundle observed")),
        }],
    })?;

    let report = json!({
        "status": "ok",
        "session_id": outcome.session_id,
        "created_session": outcome.created_session,
        "issued_grants": outcome.issued_grants.len(),
        "steps": outcome.steps.len(),
        "decisions": outcome.projection.decisions,
        "receipts": outcome.projection.receipts,
        "final_journal_root_hash": outcome.steps.last().map(|step| step.evidence.final_journal_root_hash.clone()),
        "receipt_root_hash": outcome.steps.last().map(|step| step.evidence.receipt_root_hash.clone()),
    });

    let _ = fs::remove_dir_all(root);
    if as_json {
        println!("{}", serde_json::to_string_pretty(&report)?);
    } else {
        println!("hosted runtime bundle smoke OK");
        println!("  session: {}", report["session_id"]);
        println!("  steps: {}", report["steps"]);
        println!("  receipts: {}", report["receipts"]);
    }
    Ok(())
}
