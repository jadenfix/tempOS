use std::collections::{BTreeMap, BTreeSet};
use std::error::Error;
use std::fs;

use beater_os_core::{
    ActionKind, Budget, CapabilitySelector, DataClass, ResourceKind, RiskClass, SideEffectClass,
    TaintLabel,
};
use beater_os_runtime::{
    AgentRuntime, GrantRequest, RuntimeBundle, RuntimeLocalShellWorkerLoopRequest,
    RuntimeLocalShellWorkerLoopStopReason, RuntimeLocalShellWorkerRequest, RuntimeStep,
    SessionStart, default_root_grant_id,
};
use beater_os_sandbox::safe_path_environment;
use beater_os_tool_gateway::local_shell_tool_digest_with_environment;
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
        "beater-os-runtime-worker-loop-smoke-{}",
        Uuid::new_v4()
    ));
    let workdir = root.join("work");
    fs::create_dir_all(&workdir)?;

    let runtime = AgentRuntime::open(&root)?;
    let session_id = "runtime-worker-loop-smoke-session".to_string();
    let grant_id = default_root_grant_id(&session_id);
    let command = "sh".to_string();
    let args = vec![
        "-c".to_string(),
        "printf runtime-worker-loop > worker-loop-out.txt".to_string(),
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
        "runtime-worker-loop-action-1",
        "runtime-worker-loop-action-2",
    ]
    .into_iter()
    .map(|action_id| RuntimeStep {
        session_id: session_id.clone(),
        action_id: Some(action_id.to_string()),
        tool_id: Some("shell".to_string()),
        action_kind: ActionKind::Execute,
        target: target.clone(),
        resolved_target: Some(target.clone()),
        inputs_summary: "execute bounded runtime worker loop smoke".to_string(),
        inputs_digest: Some(command_digest.clone()),
        expected_outputs: Vec::new(),
        expected_side_effects: BTreeSet::from([SideEffectClass::LocalWrite]),
        required_grants: BTreeSet::from([grant_id.clone()]),
        requested_budget: Budget {
            max_model_cents: None,
            max_tool_calls: Some(1),
            max_wall_ms: Some(30_000),
            max_payment_minor_units: None,
        },
        risk_class: RiskClass::Low,
        data_classes: BTreeSet::from([DataClass::Internal]),
        taint: BTreeSet::from([TaintLabel::TrustedUserInstruction]),
        idempotency_key: Some(action_id.to_string()),
        compensation_plan: None,
        human_explanation: "typed runtime worker loop local-shell action".to_string(),
        external_revoked_handles: BTreeSet::new(),
        observation: None,
    })
    .collect();

    let bundle = runtime.run_bundle(RuntimeBundle {
        session_id: Some(session_id.clone()),
        session: Some(SessionStart::new(
            "agent:runtime-worker-loop-smoke",
            "workspace:runtime-worker-loop-smoke",
            "prove typed runtime worker loop dispatch",
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
            "expected two runnable actions before worker loop, found {}",
            bundle.projection.runnable_pending_actions
        )
        .into());
    }

    let loop_outcome = runtime.run_local_shell_worker_loop(RuntimeLocalShellWorkerLoopRequest {
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
            initial_lease_ms: None,
            heartbeat_interval_ms: None,
            heartbeat_extend_ms: None,
            worker_id: None,
            heartbeat_evidence_refs: Vec::new(),
        },
    })?;

    if loop_outcome.stop_reason != RuntimeLocalShellWorkerLoopStopReason::NoRunnableAction {
        return Err(format!(
            "expected loop to stop with no runnable action, got {:?}",
            loop_outcome.stop_reason
        )
        .into());
    }
    if loop_outcome.executions.len() != 2
        || loop_outcome.projection.receipts != 2
        || loop_outcome.projection.runnable_pending_actions != 0
        || loop_outcome.projection.open_execution_leases != 0
    {
        return Err(format!(
            "unexpected worker loop projection: executions={} receipts={} runnable={} open_leases={}",
            loop_outcome.executions.len(),
            loop_outcome.projection.receipts,
            loop_outcome.projection.runnable_pending_actions,
            loop_outcome.projection.open_execution_leases
        )
        .into());
    }

    let output_path = workdir.join("worker-loop-out.txt");
    let output = fs::read_to_string(&output_path)?;
    if output != "runtime-worker-loop" {
        return Err(format!("unexpected worker loop output: {output:?}").into());
    }

    let report = json!({
        "status": "ok",
        "session_id": loop_outcome.session_id,
        "stop_reason": loop_outcome.stop_reason,
        "executions": loop_outcome.executions.len(),
        "receipts": loop_outcome.projection.receipts,
        "runnable_pending_actions": loop_outcome.projection.runnable_pending_actions,
        "open_execution_leases": loop_outcome.projection.open_execution_leases,
        "output": output_path.display().to_string(),
    });
    let _ = fs::remove_dir_all(root);
    if as_json {
        println!("{}", serde_json::to_string_pretty(&report)?);
    } else {
        println!("runtime local-shell worker loop smoke OK");
        println!("  session: {}", report["session_id"]);
        println!("  executions: {}", report["executions"]);
        println!("  stop: {}", report["stop_reason"]);
    }
    Ok(())
}
