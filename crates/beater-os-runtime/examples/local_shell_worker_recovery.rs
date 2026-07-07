use std::collections::BTreeSet;
use std::error::Error;
use std::fs;
use std::thread;
use std::time::Duration;

use beater_os_core::{
    ActionKind, Budget, CapabilitySelector, DataClass, ResourceKind, RiskClass, SideEffectClass,
    TaintLabel,
};
use beater_os_runtime::{
    AgentRuntime, GrantRequest, RuntimeBundle, RuntimeExecutionLeaseRecoveryRequest, RuntimeStep,
    SessionStart, default_root_grant_id,
};
use beater_os_sandbox::safe_path_environment;
use beater_os_tool_gateway::local_shell_tool_digest_with_environment;
use beater_osd::{ExecutionLeaseClaimRequest, LocalShellToolRegistration};
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

    let root = std::env::temp_dir().join(format!(
        "beater-os-runtime-recovery-smoke-{}",
        Uuid::new_v4()
    ));
    let workdir = root.join("work");
    fs::create_dir_all(&workdir)?;

    let runtime = AgentRuntime::open(&root)?;
    let session_id = "runtime-recovery-smoke-session".to_string();
    let action_id = "runtime-recovery-smoke-action".to_string();
    let lease_id = "runtime-recovery-smoke-lease".to_string();
    let grant_id = default_root_grant_id(&session_id);
    let command = "sh".to_string();
    let args = vec!["-c".to_string(), "printf should-not-run".to_string()];
    let cwd = workdir.display().to_string();
    let environment = safe_path_environment();
    let command_digest =
        local_shell_tool_digest_with_environment(&cwd, &command, &args, &environment)?;
    let target = CapabilitySelector {
        resource_kind: ResourceKind::FilePath,
        resource_id: cwd.clone(),
    };

    let bundle = runtime.run_bundle(RuntimeBundle {
        session_id: Some(session_id.clone()),
        session: Some(SessionStart::new(
            "agent:runtime-recovery-smoke",
            "workspace:runtime-recovery-smoke",
            "prove expired worker lease recovery",
        )),
        grants: vec![GrantRequest::new(
            ResourceKind::FilePath,
            cwd.clone(),
            [ActionKind::Execute],
        )],
        steps: vec![RuntimeStep {
            session_id: session_id.clone(),
            action_id: Some(action_id.clone()),
            tool_id: Some("shell".to_string()),
            action_kind: ActionKind::Execute,
            target: target.clone(),
            resolved_target: Some(target),
            inputs_summary: "claim a worker lease and simulate worker loss".to_string(),
            inputs_digest: Some(command_digest.clone()),
            expected_outputs: Vec::new(),
            expected_side_effects: BTreeSet::from([SideEffectClass::LocalWrite]),
            required_grants: BTreeSet::from([grant_id]),
            requested_budget: Budget {
                max_model_cents: None,
                max_tool_calls: Some(1),
                max_wall_ms: Some(1),
                max_payment_minor_units: None,
            },
            risk_class: RiskClass::Low,
            data_classes: BTreeSet::from([DataClass::Internal]),
            taint: BTreeSet::from([TaintLabel::TrustedUserInstruction]),
            idempotency_key: Some(action_id.clone()),
            compensation_plan: None,
            human_explanation: "runtime recovery smoke action".to_string(),
            external_revoked_handles: BTreeSet::new(),
            observation: None,
        }],
    })?;
    if bundle.projection.runnable_pending_actions != 1 {
        return Err(format!(
            "expected one runnable action before lease claim, found {}",
            bundle.projection.runnable_pending_actions
        )
        .into());
    }

    runtime
        .store()
        .register_local_shell_tool(LocalShellToolRegistration {
            workspace_id: "workspace:runtime-recovery-smoke".to_string(),
            tool_id: "shell".to_string(),
            version: format!("local-{}", &command_digest[..16]),
            content_digest: command_digest.clone(),
            side_effects: BTreeSet::from([SideEffectClass::LocalWrite]),
            risk_class: RiskClass::Low,
        })?;
    let action = runtime
        .store()
        .claimable_execution_actions(&session_id)?
        .into_iter()
        .find(|action| action.action_id == action_id)
        .ok_or("runtime recovery smoke action was not claimable")?;
    let lease = runtime.store().claim_execution_lease_from_admission(
        &session_id,
        ExecutionLeaseClaimRequest {
            lease_id: lease_id.clone(),
            action_id: action.action_id,
            expected_manifest_hash: action.manifest_hash,
            expected_decision_id: action.decision_id,
            expected_tool_version: action.expected_tool_version,
            expected_tool_digest: action.expected_tool_digest,
        },
        chrono::Utc::now(),
    )?;
    let blocked = runtime.store().project(&session_id)?;
    if RuntimeBundleProjectionSummaryProbe::from(&blocked).open_execution_leases != 1 {
        return Err("expected claimed lease to block recovery-sensitive admission".into());
    }
    while chrono::Utc::now() <= lease.lease.expires_at {
        thread::sleep(Duration::from_millis(100));
    }

    let recovery = runtime
        .recover_expired_execution_lease_once(RuntimeExecutionLeaseRecoveryRequest {
            session_id: session_id.clone(),
            action_id: Some(action_id.clone()),
            lease_id: Some(lease_id.clone()),
            reconciliation_id: Some("runtime-recovery-smoke-reconciliation".to_string()),
            reconciled_by: Some("agent:runtime-recovery-smoke".to_string()),
            reason: Some("worker process disappeared before receipt completion".to_string()),
            evidence_refs: vec!["smoke://runtime-worker-lost".to_string()],
        })?
        .ok_or("expired lease recovery returned no reconciliation")?;
    if recovery.resolution != beater_os_core::ExecutionLeaseResolution::OutcomeUnknown {
        return Err(format!("unexpected recovery resolution: {:?}", recovery.resolution).into());
    }
    if recovery.projection.open_execution_leases != 0
        || recovery.projection.recovery_blocked
        || recovery.projection.runnable_pending_actions != 0
        || recovery.projection.execution_reconciliations != 1
        || recovery.projection.receipts != 0
    {
        return Err(format!(
            "unexpected recovery projection: open={} blocked={} runnable={} reconciliations={} receipts={}",
            recovery.projection.open_execution_leases,
            recovery.projection.recovery_blocked,
            recovery.projection.runnable_pending_actions,
            recovery.projection.execution_reconciliations,
            recovery.projection.receipts
        )
        .into());
    }

    let report = json!({
        "status": "ok",
        "session_id": recovery.session_id,
        "action_id": recovery.action_id,
        "lease_id": recovery.lease_id,
        "reconciliation_id": recovery.reconciliation_id,
        "resolution": recovery.resolution,
        "reconciliation_hash": recovery.reconciliation_hash,
        "open_execution_leases": recovery.projection.open_execution_leases,
        "recovery_blocked": recovery.projection.recovery_blocked,
        "execution_reconciliations": recovery.projection.execution_reconciliations,
        "receipts": recovery.projection.receipts,
    });
    let _ = fs::remove_dir_all(root);
    if as_json {
        println!("{}", serde_json::to_string_pretty(&report)?);
    } else {
        println!("runtime worker recovery smoke OK");
        println!("  session: {}", report["session_id"]);
        println!("  action: {}", report["action_id"]);
        println!("  reconciliation: {}", report["reconciliation_id"]);
    }
    Ok(())
}

struct RuntimeBundleProjectionSummaryProbe {
    open_execution_leases: usize,
}

impl From<&beater_osd::SessionProjection> for RuntimeBundleProjectionSummaryProbe {
    fn from(projection: &beater_osd::SessionProjection) -> Self {
        let closed: BTreeSet<&str> = projection
            .receipts
            .iter()
            .map(|receipt| receipt.action_id.as_str())
            .chain(
                projection
                    .execution_reconciliations
                    .iter()
                    .map(|reconciliation| reconciliation.action_id.as_str()),
            )
            .collect();
        Self {
            open_execution_leases: projection
                .execution_leases
                .iter()
                .filter(|lease| !closed.contains(lease.action_id.as_str()))
                .count(),
        }
    }
}
