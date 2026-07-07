use std::collections::{BTreeMap, BTreeSet};
use std::error::Error;
use std::fs;
use std::thread;
use std::time::Duration;

use beater_os_core::{
    ActionKind, Budget, CapabilitySelector, DataClass, ResourceKind, RiskClass, SideEffectClass,
    TaintLabel,
};
use beater_os_runtime::{
    AgentRuntime, GrantRequest, RuntimeBundle, RuntimeLocalShellWorkerLoopRequest,
    RuntimeLocalShellWorkerLoopStopReason, RuntimeLocalShellWorkerRequest, RuntimeStep,
    RuntimeSupervisedLocalShellWorkerCycleRequest, SessionStart, default_root_grant_id,
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
        "beater-os-runtime-supervised-cycle-smoke-{}",
        Uuid::new_v4()
    ));
    let workdir = root.join("work");
    fs::create_dir_all(&workdir)?;

    let runtime = AgentRuntime::open(&root)?;
    let session_id = "runtime-supervised-cycle-smoke-session".to_string();
    let lost_action_id = "runtime-supervised-cycle-lost-action".to_string();
    let run_action_id = "runtime-supervised-cycle-run-action".to_string();
    let lost_lease_id = "runtime-supervised-cycle-lost-lease".to_string();
    let grant_id = default_root_grant_id(&session_id);
    let command = "sh".to_string();
    let args = vec![
        "-c".to_string(),
        "printf runtime-supervised-cycle > supervised-out.txt".to_string(),
    ];
    let cwd = workdir.display().to_string();
    let environment = safe_path_environment();
    let command_digest =
        local_shell_tool_digest_with_environment(&cwd, &command, &args, &environment)?;
    let target = CapabilitySelector {
        resource_kind: ResourceKind::FilePath,
        resource_id: cwd.clone(),
    };

    let steps: Vec<RuntimeStep> = [
        (lost_action_id.clone(), 1_u64),
        (run_action_id.clone(), 30_000_u64),
    ]
    .into_iter()
    .map(|(action_id, max_wall_ms)| RuntimeStep {
        session_id: session_id.clone(),
        action_id: Some(action_id.clone()),
        tool_id: Some("shell".to_string()),
        action_kind: ActionKind::Execute,
        target: target.clone(),
        resolved_target: Some(target.clone()),
        inputs_summary: "execute supervised runtime worker cycle smoke".to_string(),
        inputs_digest: Some(command_digest.clone()),
        expected_outputs: Vec::new(),
        expected_side_effects: BTreeSet::from([SideEffectClass::LocalWrite]),
        required_grants: BTreeSet::from([grant_id.clone()]),
        requested_budget: Budget {
            max_model_cents: None,
            max_tool_calls: Some(1),
            max_wall_ms: Some(max_wall_ms),
            max_payment_minor_units: None,
        },
        risk_class: RiskClass::Low,
        data_classes: BTreeSet::from([DataClass::Internal]),
        taint: BTreeSet::from([TaintLabel::TrustedUserInstruction]),
        idempotency_key: Some(action_id),
        compensation_plan: None,
        human_explanation: "supervised runtime worker cycle local-shell action".to_string(),
        external_revoked_handles: BTreeSet::new(),
        observation: None,
    })
    .collect();

    let bundle = runtime.run_bundle(RuntimeBundle {
        session_id: Some(session_id.clone()),
        session: Some(SessionStart::new(
            "agent:runtime-supervised-cycle-smoke",
            "workspace:runtime-supervised-cycle-smoke",
            "prove supervised recovery plus worker dispatch",
        )),
        grants: vec![GrantRequest::new(
            ResourceKind::FilePath,
            cwd.clone(),
            [ActionKind::Execute],
        )],
        steps,
    })?;
    if bundle.projection.runnable_pending_actions != 2 {
        return Err(format!(
            "expected two runnable actions before stale lease claim, found {}",
            bundle.projection.runnable_pending_actions
        )
        .into());
    }

    runtime
        .store()
        .register_local_shell_tool(LocalShellToolRegistration {
            workspace_id: "workspace:runtime-supervised-cycle-smoke".to_string(),
            tool_id: "shell".to_string(),
            version: format!("local-{}", &command_digest[..16]),
            content_digest: command_digest.clone(),
            side_effects: BTreeSet::from([SideEffectClass::LocalWrite]),
            risk_class: RiskClass::Low,
        })?;
    let lost_action = runtime
        .store()
        .claimable_execution_actions(&session_id)?
        .into_iter()
        .find(|action| action.action_id == lost_action_id)
        .ok_or("lost action was not claimable")?;
    let lost_lease = runtime.store().claim_execution_lease_from_admission(
        &session_id,
        ExecutionLeaseClaimRequest {
            lease_id: lost_lease_id.clone(),
            action_id: lost_action.action_id,
            expected_manifest_hash: lost_action.manifest_hash,
            expected_decision_id: lost_action.decision_id,
            expected_tool_version: lost_action.expected_tool_version,
            expected_tool_digest: lost_action.expected_tool_digest,
        },
        chrono::Utc::now(),
    )?;

    let live_supervised = runtime.run_supervised_local_shell_worker_cycle(
        RuntimeSupervisedLocalShellWorkerCycleRequest {
            max_recoveries: 1,
            recovery_reason: "supervised smoke observed expired worker lease".to_string(),
            reconciled_by: Some("agent:runtime-supervised-cycle-smoke".to_string()),
            recovery_evidence_refs: vec!["smoke://supervised-worker-lost".to_string()],
            worker_loop: RuntimeLocalShellWorkerLoopRequest {
                max_actions: 8,
                worker: RuntimeLocalShellWorkerRequest {
                    session_id: session_id.clone(),
                    action_id: None,
                    lease_id: None,
                    tool: Some("shell".to_string()),
                    tool_version: None,
                    tool_digest: Some(command_digest.clone()),
                    command: command.clone(),
                    args: args.clone(),
                    cwd: cwd.clone(),
                    env: BTreeMap::new(),
                    side_effects: BTreeSet::from([SideEffectClass::LocalWrite]),
                    risk: Some(RiskClass::Low),
                    receipt_id: None,
                    timeout_secs: Some(30),
                    max_output_bytes: None,
                },
            },
        },
    )?;
    if !live_supervised.recoveries.is_empty()
        || !live_supervised.worker_loop.executions.is_empty()
        || live_supervised.worker_loop.stop_reason
            != RuntimeLocalShellWorkerLoopStopReason::RecoveryBlocked
        || live_supervised.projection.open_execution_leases != 1
        || live_supervised.projection.live_open_execution_leases != 1
        || live_supervised
            .projection
            .expired_recoverable_execution_leases
            != 0
    {
        return Err(format!(
            "live lease should block without recovery: recoveries={} executions={} stop={:?} open_leases={} live_leases={} expired_recoverable_leases={}",
            live_supervised.recoveries.len(),
            live_supervised.worker_loop.executions.len(),
            live_supervised.worker_loop.stop_reason,
            live_supervised.projection.open_execution_leases,
            live_supervised.projection.live_open_execution_leases,
            live_supervised.projection.expired_recoverable_execution_leases
        )
        .into());
    }

    while chrono::Utc::now() <= lost_lease.lease.expires_at {
        thread::sleep(Duration::from_millis(100));
    }

    let supervised = runtime.run_supervised_local_shell_worker_cycle(
        RuntimeSupervisedLocalShellWorkerCycleRequest {
            max_recoveries: 1,
            recovery_reason: "supervised smoke observed expired worker lease".to_string(),
            reconciled_by: Some("agent:runtime-supervised-cycle-smoke".to_string()),
            recovery_evidence_refs: vec!["smoke://supervised-worker-lost".to_string()],
            worker_loop: RuntimeLocalShellWorkerLoopRequest {
                max_actions: 8,
                worker: RuntimeLocalShellWorkerRequest {
                    session_id: session_id.clone(),
                    action_id: None,
                    lease_id: None,
                    tool: Some("shell".to_string()),
                    tool_version: None,
                    tool_digest: Some(command_digest.clone()),
                    command,
                    args,
                    cwd,
                    env: BTreeMap::new(),
                    side_effects: BTreeSet::from([SideEffectClass::LocalWrite]),
                    risk: Some(RiskClass::Low),
                    receipt_id: None,
                    timeout_secs: Some(30),
                    max_output_bytes: None,
                },
            },
        },
    )?;

    if supervised.recoveries.len() != 1 {
        return Err(format!(
            "expected one supervised recovery, found {}",
            supervised.recoveries.len()
        )
        .into());
    }
    if supervised.worker_loop.stop_reason != RuntimeLocalShellWorkerLoopStopReason::NoRunnableAction
        || supervised.worker_loop.executions.len() != 1
        || supervised.worker_loop.executions[0].action_id != run_action_id
        || supervised.projection.execution_reconciliations != 1
        || supervised.projection.receipts != 1
        || supervised.projection.runnable_pending_actions != 0
        || supervised.projection.open_execution_leases != 0
        || supervised.projection.live_open_execution_leases != 0
        || supervised.projection.expired_recoverable_execution_leases != 0
    {
        return Err(format!(
            "unexpected supervised projection: recoveries={} stop={:?} executions={} receipts={} reconciliations={} runnable={} open_leases={} live_leases={} expired_recoverable_leases={}",
            supervised.recoveries.len(),
            supervised.worker_loop.stop_reason,
            supervised.worker_loop.executions.len(),
            supervised.projection.receipts,
            supervised.projection.execution_reconciliations,
            supervised.projection.runnable_pending_actions,
            supervised.projection.open_execution_leases,
            supervised.projection.live_open_execution_leases,
            supervised.projection.expired_recoverable_execution_leases
        )
        .into());
    }

    let output_path = workdir.join("supervised-out.txt");
    let output = fs::read_to_string(&output_path)?;
    if output != "runtime-supervised-cycle" {
        return Err(format!("unexpected supervised worker output: {output:?}").into());
    }

    let report = json!({
        "status": "ok",
        "session_id": supervised.session_id,
        "recoveries": supervised.recoveries.len(),
        "executions": supervised.worker_loop.executions.len(),
        "executed_action": supervised.worker_loop.executions[0].action_id,
        "stop_reason": supervised.worker_loop.stop_reason,
        "receipts": supervised.projection.receipts,
        "execution_reconciliations": supervised.projection.execution_reconciliations,
        "runnable_pending_actions": supervised.projection.runnable_pending_actions,
        "open_execution_leases": supervised.projection.open_execution_leases,
        "live_open_execution_leases": supervised.projection.live_open_execution_leases,
        "expired_recoverable_execution_leases": supervised.projection.expired_recoverable_execution_leases,
        "output": output_path.display().to_string(),
    });
    let _ = fs::remove_dir_all(root);
    if as_json {
        println!("{}", serde_json::to_string_pretty(&report)?);
    } else {
        println!("runtime supervised local-shell worker cycle smoke OK");
        println!("  session: {}", report["session_id"]);
        println!("  recoveries: {}", report["recoveries"]);
        println!("  executions: {}", report["executions"]);
    }
    Ok(())
}
