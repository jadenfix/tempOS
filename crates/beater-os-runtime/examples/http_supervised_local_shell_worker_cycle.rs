use std::collections::BTreeSet;
use std::error::Error;
use std::fs;
use std::io::{Read, Write};
use std::net::{TcpListener, TcpStream};
use std::process::{Command, Stdio};
use std::thread;
use std::time::{Duration, Instant};

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

const TOKEN: &str = "beateros-http-supervised-worker-smoke-token";
const SESSION_ID: &str = "http-supervised-worker-smoke-session";

fn main() -> Result<(), Box<dyn Error>> {
    let mut as_json = false;
    for arg in std::env::args().skip(1) {
        match arg.as_str() {
            "--json" => as_json = true,
            other => return Err(format!("unsupported argument: {other}").into()),
        }
    }

    let root = std::env::temp_dir().join(format!(
        "beater-os-http-supervised-worker-smoke-{}",
        Uuid::new_v4()
    ));
    let workdir = root.join("work");
    fs::create_dir_all(&workdir)?;
    let token_file = root.join("token");
    fs::write(&token_file, TOKEN)?;

    let runtime = AgentRuntime::open(&root)?;
    let lost_action_id = "http-supervised-worker-lost-action".to_string();
    let run_action_id = "http-supervised-worker-run-action".to_string();
    let lost_lease_id = "http-supervised-worker-lost-lease".to_string();
    let grant_id = default_root_grant_id(SESSION_ID);
    let command = "sh".to_string();
    let args = vec![
        "-c".to_string(),
        "printf http-supervised-worker > http-supervised-worker-out.txt".to_string(),
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
        inputs_summary: "execute HTTP supervised worker cycle smoke".to_string(),
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
        human_explanation: "HTTP supervised worker local-shell action".to_string(),
        external_revoked_handles: BTreeSet::new(),
        observation: None,
    })
    .collect();

    let bundle = runtime.run_bundle(RuntimeBundle {
        session_id: Some(SESSION_ID.to_string()),
        session: Some(SessionStart::new(
            "agent:http-supervised-worker-smoke",
            "workspace:http-supervised-worker-smoke",
            "prove HTTP supervised recovery plus worker dispatch",
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
            workspace_id: "workspace:http-supervised-worker-smoke".to_string(),
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
            lease_id: lost_lease_id.clone(),
            action_id: lost_action.action_id,
            expected_manifest_hash: lost_action.manifest_hash,
            expected_decision_id: lost_action.decision_id,
            expected_tool_version: lost_action.expected_tool_version,
            expected_tool_digest: lost_action.expected_tool_digest,
        },
        chrono::Utc::now(),
    )?;

    let live_projection = one_shot_get_session(&root, &token_file)?;
    if live_projection.status != 200
        || live_projection.body["open_execution_leases"] != 1
        || live_projection.body["live_open_execution_leases"] != 1
        || live_projection.body["expired_recoverable_execution_leases"] != 0
        || live_projection.body["open_execution_lease_statuses"][0]["status"] != "live_open"
        || live_projection.body["recovery_blocked"] != true
        || live_projection.body["admission_blocked"] != true
    {
        return Err(format!(
            "live HTTP session projection should expose a non-recoverable open lease: {}",
            live_projection.body
        )
        .into());
    }

    let body = json!({
        "tool": "shell",
        "tool_digest": command_digest,
        "command": command,
        "args": args,
        "cwd": cwd,
        "side_effects": ["local_write"],
        "timeout_secs": 30,
        "max_actions": 8,
        "recover_expired_leases": true,
        "max_recoveries": 1,
        "recovery_reason": "HTTP supervised smoke observed expired worker lease",
        "reconciled_by": "agent:http-supervised-worker-smoke",
        "recovery_evidence_refs": ["smoke://http-supervised-worker-lost"],
    });

    let live_response = one_shot_request(&root, &token_file, &body)?;
    if live_response.status != 200 {
        return Err(format!(
            "expected 200 from live HTTP supervised loop, got {}: {}",
            live_response.status, live_response.body
        )
        .into());
    }
    if !live_response.body["recoveries"]
        .as_array()
        .ok_or("live recoveries must be an array")?
        .is_empty()
        || !live_response.body["worker_loop"]["executions"]
            .as_array()
            .ok_or("live executions must be an array")?
            .is_empty()
        || live_response.body["worker_loop"]["stop_reason"] != "recovery_blocked"
        || live_response.body["projection"]["open_execution_leases"] != 1
        || live_response.body["projection"]["live_open_execution_leases"] != 1
        || live_response.body["projection"]["expired_recoverable_execution_leases"] != 0
    {
        return Err(format!(
            "live lease should block without recovery or execution: {}",
            live_response.body
        )
        .into());
    }

    while chrono::Utc::now() <= lost_lease.lease.expires_at {
        thread::sleep(Duration::from_millis(100));
    }

    let expired_projection = one_shot_get_session(&root, &token_file)?;
    if expired_projection.status != 200
        || expired_projection.body["open_execution_leases"] != 1
        || expired_projection.body["live_open_execution_leases"] != 0
        || expired_projection.body["expired_recoverable_execution_leases"] != 1
        || expired_projection.body["open_execution_lease_statuses"][0]["status"]
            != "expired_recoverable"
        || expired_projection.body["recovery_blocked"] != true
        || expired_projection.body["admission_blocked"] != true
    {
        return Err(format!(
            "expired HTTP session projection should expose a recoverable open lease: {}",
            expired_projection.body
        )
        .into());
    }

    let recovered_response = one_shot_request(&root, &token_file, &body)?;
    if recovered_response.status != 200 {
        return Err(format!(
            "expected 200 from expired HTTP supervised loop, got {}: {}",
            recovered_response.status, recovered_response.body
        )
        .into());
    }
    let recoveries = recovered_response.body["recoveries"]
        .as_array()
        .ok_or("recoveries must be an array")?;
    let executions = recovered_response.body["worker_loop"]["executions"]
        .as_array()
        .ok_or("executions must be an array")?;
    if recoveries.len() != 1
        || executions.len() != 1
        || executions[0]["action_id"] != run_action_id
        || recovered_response.body["worker_loop"]["stop_reason"] != "no_runnable_action"
        || recovered_response.body["projection"]["receipts"] != 1
        || recovered_response.body["projection"]["execution_reconciliations"] != 1
        || recovered_response.body["projection"]["runnable_pending_actions"] != 0
        || recovered_response.body["projection"]["open_execution_leases"] != 0
        || recovered_response.body["projection"]["live_open_execution_leases"] != 0
        || recovered_response.body["projection"]["expired_recoverable_execution_leases"] != 0
    {
        return Err(format!(
            "unexpected recovered HTTP supervised projection: {}",
            recovered_response.body
        )
        .into());
    }
    let output_path = workdir.join("http-supervised-worker-out.txt");
    let output_text = fs::read_to_string(&output_path)?;
    if output_text != "http-supervised-worker" {
        return Err(format!("unexpected HTTP supervised output: {output_text:?}").into());
    }

    let report = json!({
        "status": "ok",
        "session_id": SESSION_ID,
        "recoveries": recoveries.len(),
        "executions": executions.len(),
        "executed_action": executions[0]["action_id"],
        "stop_reason": recovered_response.body["worker_loop"]["stop_reason"],
        "receipts": recovered_response.body["projection"]["receipts"],
        "execution_reconciliations": recovered_response.body["projection"]["execution_reconciliations"],
        "runnable_pending_actions": recovered_response.body["projection"]["runnable_pending_actions"],
        "open_execution_leases": recovered_response.body["projection"]["open_execution_leases"],
        "live_open_execution_leases_before_recovery": live_projection.body["live_open_execution_leases"],
        "expired_recoverable_execution_leases_before_recovery": expired_projection.body["expired_recoverable_execution_leases"],
        "output": output_path.display().to_string(),
    });
    let _ = fs::remove_dir_all(root);
    if as_json {
        println!("{}", serde_json::to_string_pretty(&report)?);
    } else {
        println!("HTTP supervised local-shell worker cycle smoke OK");
        println!("  session: {}", report["session_id"]);
        println!("  recoveries: {}", report["recoveries"]);
        println!("  executions: {}", report["executions"]);
    }
    Ok(())
}

struct HttpResponse {
    status: u16,
    body: Value,
}

fn one_shot_get_session(
    root: &std::path::Path,
    token_file: &std::path::Path,
) -> Result<HttpResponse, Box<dyn Error>> {
    let port = free_loopback_port()?;
    let mut server = Command::new("cargo")
        .args([
            "run",
            "-q",
            "-p",
            "beater-osd-http",
            "--",
            "serve",
            "--root",
            &root.display().to_string(),
            "--token-file",
            &token_file.display().to_string(),
            "--bind",
            &format!("127.0.0.1:{port}"),
            "--once",
        ])
        .stdout(Stdio::piped())
        .stderr(Stdio::piped())
        .spawn()?;
    let response = match get_json(port, &format!("/v1/sessions/{SESSION_ID}"), TOKEN) {
        Ok(response) => response,
        Err(err) => {
            stop_server(&mut server);
            return Err(err);
        }
    };
    let output = server.wait_with_output()?;
    if !output.status.success() {
        return Err(format!(
            "beater-osd-http exited {}\nSTDOUT:\n{}\nSTDERR:\n{}",
            output.status,
            String::from_utf8_lossy(&output.stdout),
            String::from_utf8_lossy(&output.stderr)
        )
        .into());
    }
    Ok(response)
}

fn one_shot_request(
    root: &std::path::Path,
    token_file: &std::path::Path,
    body: &Value,
) -> Result<HttpResponse, Box<dyn Error>> {
    let port = free_loopback_port()?;
    let mut server = Command::new("cargo")
        .args([
            "run",
            "-q",
            "-p",
            "beater-osd-http",
            "--",
            "serve",
            "--root",
            &root.display().to_string(),
            "--token-file",
            &token_file.display().to_string(),
            "--bind",
            &format!("127.0.0.1:{port}"),
            "--once",
        ])
        .stdout(Stdio::piped())
        .stderr(Stdio::piped())
        .spawn()?;
    let response = match post_json(
        port,
        &format!("/v1/sessions/{SESSION_ID}/actions/execute-local-shell-loop"),
        body,
        TOKEN,
    ) {
        Ok(response) => response,
        Err(err) => {
            stop_server(&mut server);
            return Err(err);
        }
    };
    let output = server.wait_with_output()?;
    if !output.status.success() {
        return Err(format!(
            "beater-osd-http exited {}\nSTDOUT:\n{}\nSTDERR:\n{}",
            output.status,
            String::from_utf8_lossy(&output.stdout),
            String::from_utf8_lossy(&output.stderr)
        )
        .into());
    }
    Ok(response)
}

fn free_loopback_port() -> Result<u16, Box<dyn Error>> {
    let listener = TcpListener::bind("127.0.0.1:0")?;
    Ok(listener.local_addr()?.port())
}

fn post_json(
    port: u16,
    path: &str,
    body: &Value,
    token: &str,
) -> Result<HttpResponse, Box<dyn Error>> {
    let encoded = body.to_string();
    let deadline = Instant::now() + Duration::from_secs(20);
    let mut last_error = None;
    while Instant::now() < deadline {
        match TcpStream::connect(("127.0.0.1", port)) {
            Ok(mut stream) => {
                let request = format!(
                    "POST {path} HTTP/1.1\r\nHost: 127.0.0.1:{port}\r\nAuthorization: Bearer {token}\r\nContent-Type: application/json\r\nContent-Length: {}\r\nConnection: close\r\n\r\n{encoded}",
                    encoded.len()
                );
                stream.write_all(request.as_bytes())?;
                let mut response = String::new();
                stream.read_to_string(&mut response)?;
                return parse_response(&response);
            }
            Err(err) => {
                last_error = Some(err);
                thread::sleep(Duration::from_millis(100));
            }
        }
    }
    Err(format!("server did not accept request: {last_error:?}").into())
}

fn get_json(port: u16, path: &str, token: &str) -> Result<HttpResponse, Box<dyn Error>> {
    let deadline = Instant::now() + Duration::from_secs(20);
    let mut last_error = None;
    while Instant::now() < deadline {
        match TcpStream::connect(("127.0.0.1", port)) {
            Ok(mut stream) => {
                let request = format!(
                    "GET {path} HTTP/1.1\r\nHost: 127.0.0.1:{port}\r\nAuthorization: Bearer {token}\r\nConnection: close\r\n\r\n"
                );
                stream.write_all(request.as_bytes())?;
                let mut response = String::new();
                stream.read_to_string(&mut response)?;
                return parse_response(&response);
            }
            Err(err) => {
                last_error = Some(err);
                thread::sleep(Duration::from_millis(100));
            }
        }
    }
    Err(format!("server did not accept request: {last_error:?}").into())
}

fn parse_response(response: &str) -> Result<HttpResponse, Box<dyn Error>> {
    let (head, body) = response
        .split_once("\r\n\r\n")
        .ok_or("HTTP response missing header terminator")?;
    let status = head
        .lines()
        .next()
        .and_then(|line| line.split_whitespace().nth(1))
        .ok_or("HTTP response missing status")?
        .parse::<u16>()?;
    Ok(HttpResponse {
        status,
        body: serde_json::from_str(body)?,
    })
}

fn stop_server(server: &mut std::process::Child) {
    let _ = server.kill();
    let _ = server.wait();
}
