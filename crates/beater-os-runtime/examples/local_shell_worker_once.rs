use std::collections::{BTreeMap, BTreeSet};
use std::error::Error;
use std::fs;

use beater_os_core::{
    ActionKind, Budget, CapabilitySelector, DataClass, ResourceKind, RiskClass, SideEffectClass,
    TaintLabel,
};
use beater_os_runtime::{
    AgentRuntime, GrantRequest, RuntimeBundle, RuntimeLocalShellWorkerRequest, RuntimeStep,
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

    let root =
        std::env::temp_dir().join(format!("beater-os-runtime-worker-smoke-{}", Uuid::new_v4()));
    let workdir = root.join("work");
    fs::create_dir_all(&workdir)?;

    let runtime = AgentRuntime::open(&root)?;
    let session_id = "runtime-worker-smoke-session".to_string();
    let action_id = "runtime-worker-smoke-action".to_string();
    let grant_id = default_root_grant_id(&session_id);
    let command = "sh".to_string();
    let args = vec![
        "-c".to_string(),
        "printf runtime-worker-smoke > worker-out.txt".to_string(),
    ];
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
            "agent:runtime-worker-smoke",
            "workspace:runtime-worker-smoke",
            "prove typed runtime worker-once dispatch",
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
            resolved_target: Some(target.clone()),
            inputs_summary: "execute typed runtime worker smoke".to_string(),
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
            idempotency_key: Some(action_id.clone()),
            compensation_plan: None,
            human_explanation: "typed runtime worker local-shell action".to_string(),
            external_revoked_handles: BTreeSet::new(),
            observation: None,
        }],
    })?;
    if bundle.projection.runnable_pending_actions != 1 {
        return Err(format!(
            "expected one runnable action before worker dispatch, found {}",
            bundle.projection.runnable_pending_actions
        )
        .into());
    }

    let worker = runtime
        .run_local_shell_worker_once(RuntimeLocalShellWorkerRequest {
            session_id: session_id.clone(),
            action_id: Some(action_id.clone()),
            lease_id: Some("runtime-worker-smoke-lease".to_string()),
            tool: Some("shell".to_string()),
            tool_version: None,
            tool_digest: Some(command_digest.clone()),
            command,
            args,
            cwd,
            env: BTreeMap::new(),
            side_effects: BTreeSet::from([SideEffectClass::LocalWrite]),
            risk: Some(RiskClass::Low),
            receipt_id: Some("runtime-worker-smoke-receipt".to_string()),
            timeout_secs: Some(30),
            max_output_bytes: None,
            initial_lease_ms: None,
            heartbeat_interval_ms: None,
            heartbeat_extend_ms: None,
            worker_id: None,
            heartbeat_evidence_refs: Vec::new(),
        })?
        .ok_or("runtime worker found no claimable action")?;

    let output_path = workdir.join("worker-out.txt");
    let output = fs::read_to_string(&output_path)?;
    if output != "runtime-worker-smoke" {
        return Err(format!("unexpected worker output: {output:?}").into());
    }
    if worker.projection.receipts != 1 || worker.projection.runnable_pending_actions != 0 {
        return Err(format!(
            "unexpected worker projection: receipts={} runnable={}",
            worker.projection.receipts, worker.projection.runnable_pending_actions
        )
        .into());
    }

    let report = json!({
        "status": "ok",
        "session_id": worker.session_id,
        "action_id": worker.action_id,
        "lease_id": worker.lease_id,
        "tool_ref": worker.tool_ref,
        "execution_status": worker.execution.status,
        "receipt_id": worker.receipt.receipt_id,
        "receipt_hash": worker.receipt.receipt_hash,
        "manifest_hash": worker.manifest_hash,
        "output": output_path.display().to_string(),
        "receipts": worker.projection.receipts,
        "runnable_pending_actions": worker.projection.runnable_pending_actions,
    });

    let _ = fs::remove_dir_all(root);
    if as_json {
        println!("{}", serde_json::to_string_pretty(&report)?);
    } else {
        println!("runtime local-shell worker smoke OK");
        println!("  session: {}", report["session_id"]);
        println!("  action: {}", report["action_id"]);
        println!("  receipt: {}", report["receipt_id"]);
    }
    Ok(())
}
