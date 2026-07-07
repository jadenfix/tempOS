use std::collections::BTreeSet;
use std::error::Error;
use std::fs;
use std::process::Command;
use std::thread;
use std::time::Duration;

use beater_os_core::{
    ActionKind, Budget, CapabilitySelector, DataClass, ResourceKind, RiskClass, SideEffectClass,
    TaintLabel,
};
use beater_os_runtime::{
    AgentRuntime, GrantRequest, RuntimeBundle, RuntimeStep, SessionStart, default_root_grant_id,
};
use beater_os_sandbox::safe_path_environment;
use beater_os_tool_gateway::local_shell_tool_digest_with_environment;
use beater_osd::{ExecutionLeaseClaimRequest, LocalShellToolRegistration};
use serde_json::{Value, json};
use uuid::Uuid;

const SESSION_ID: &str = "runtime-supervisor-service-smoke-session";

fn main() -> Result<(), Box<dyn Error>> {
    let mut as_json = false;
    for arg in std::env::args().skip(1) {
        match arg.as_str() {
            "--json" => as_json = true,
            other => return Err(format!("unsupported argument: {other}").into()),
        }
    }

    let root = std::env::temp_dir().join(format!(
        "beater-os-runtime-supervisor-service-smoke-{}",
        Uuid::new_v4()
    ));
    let workdir = root.join("work");
    fs::create_dir_all(&workdir)?;

    let runtime = AgentRuntime::open(&root)?;
    let lost_action_id = "runtime-supervisor-service-lost-action".to_string();
    let run_action_id = "runtime-supervisor-service-run-action".to_string();
    let lost_lease_id = "runtime-supervisor-service-lost-lease".to_string();
    let grant_id = default_root_grant_id(SESSION_ID);
    let command = "sh".to_string();
    let args = vec![
        "-c".to_string(),
        "printf runtime-supervisor-service > supervisor-service-out.txt".to_string(),
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
        session_id: SESSION_ID.to_string(),
        action_id: Some(action_id.clone()),
        tool_id: Some("shell".to_string()),
        action_kind: ActionKind::Execute,
        target: target.clone(),
        resolved_target: Some(target.clone()),
        inputs_summary: "execute runtime supervisor service smoke".to_string(),
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
        human_explanation: "runtime supervisor service local-shell action".to_string(),
        external_revoked_handles: BTreeSet::new(),
        observation: None,
    })
    .collect();

    let bundle = runtime.run_bundle(RuntimeBundle {
        session_id: Some(SESSION_ID.to_string()),
        session: Some(SessionStart::new(
            "agent:runtime-supervisor-service-smoke",
            "workspace:runtime-supervisor-service-smoke",
            "prove bounded runtime worker supervisor service",
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
            "expected two runnable actions before supervisor run, found {}",
            bundle.projection.runnable_pending_actions
        )
        .into());
    }

    runtime
        .store()
        .register_local_shell_tool(LocalShellToolRegistration {
            workspace_id: "workspace:runtime-supervisor-service-smoke".to_string(),
            tool_id: "shell".to_string(),
            version: format!("local-{}", &command_digest[..16]),
            content_digest: command_digest.clone(),
            side_effects: BTreeSet::from([SideEffectClass::LocalWrite]),
            risk_class: RiskClass::Low,
        })?;
    let lost_action = runtime
        .store()
        .claimable_execution_actions(SESSION_ID)?
        .into_iter()
        .find(|action| action.action_id == lost_action_id)
        .ok_or("lost action was not claimable")?;
    let lost_lease = runtime.store().claim_execution_lease_from_admission(
        SESSION_ID,
        ExecutionLeaseClaimRequest {
            lease_id: lost_lease_id,
            action_id: lost_action.action_id,
            expected_manifest_hash: lost_action.manifest_hash,
            expected_decision_id: lost_action.decision_id,
            expected_tool_version: lost_action.expected_tool_version,
            expected_tool_digest: lost_action.expected_tool_digest,
            initial_lease_ms: None,
        },
        chrono::Utc::now(),
    )?;
    while chrono::Utc::now() <= lost_lease.lease.expires_at {
        thread::sleep(Duration::from_millis(100));
    }

    let output = Command::new("cargo")
        .args([
            "run",
            "-q",
            "-p",
            "beater-os-runtime",
            "--bin",
            "beater-os-runtime-worker",
            "--",
            "supervise-local-shell",
            "--root",
            &root.display().to_string(),
            "--session-id",
            SESSION_ID,
            "--cwd",
            &cwd,
            "--command",
            &command,
            "--arg",
            &args[0],
            "--arg",
            &args[1],
            "--tool",
            "shell",
            "--tool-digest",
            &command_digest,
            "--side-effect",
            "local_write",
            "--timeout-secs",
            "30",
            "--max-actions",
            "8",
            "--max-recoveries",
            "1",
            "--max-cycles",
            "1",
            "--worker-id",
            "worker:runtime-supervisor-service-smoke",
            "--initial-lease-ms",
            "500",
            "--heartbeat-interval-ms",
            "100",
            "--heartbeat-extend-ms",
            "1000",
            "--heartbeat-evidence-ref",
            "smoke://runtime-supervisor-service-heartbeat",
            "--reconciled-by",
            "agent:runtime-supervisor-service-smoke",
            "--recovery-reason",
            "runtime supervisor service smoke observed expired worker lease",
            "--recovery-evidence-ref",
            "smoke://runtime-supervisor-service-lost",
            "--json",
        ])
        .output()?;
    if !output.status.success() {
        return Err(format!(
            "beater-os-runtime-worker exited {}\nSTDOUT:\n{}\nSTDERR:\n{}",
            output.status,
            String::from_utf8_lossy(&output.stdout),
            String::from_utf8_lossy(&output.stderr)
        )
        .into());
    }
    let report: Value = serde_json::from_slice(&output.stdout)?;
    if report["recoveries"] != 1
        || report["executions"] != 1
        || report["receipts"] != 1
        || report["execution_reconciliations"] != 1
        || report["runnable_pending_actions"] != 0
        || report["open_execution_leases"] != 0
        || report["live_open_execution_leases"] != 0
        || report["expired_recoverable_execution_leases"] != 0
    {
        return Err(format!("unexpected supervisor report: {report}").into());
    }
    let output_path = workdir.join("supervisor-service-out.txt");
    let output_text = fs::read_to_string(&output_path)?;
    if output_text != "runtime-supervisor-service" {
        return Err(format!("unexpected supervisor output: {output_text:?}").into());
    }

    let final_report = json!({
        "status": "ok",
        "session_id": SESSION_ID,
        "cycles": report["cycles"],
        "recoveries": report["recoveries"],
        "executions": report["executions"],
        "receipts": report["receipts"],
        "execution_reconciliations": report["execution_reconciliations"],
        "output": output_path.display().to_string(),
    });
    let _ = fs::remove_dir_all(root);
    if as_json {
        println!("{}", serde_json::to_string_pretty(&final_report)?);
    } else {
        println!("runtime worker supervisor service smoke OK");
        println!("  session: {}", final_report["session_id"]);
        println!("  executions: {}", final_report["executions"]);
        println!("  recoveries: {}", final_report["recoveries"]);
    }
    Ok(())
}
