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
use serde_json::{Value, json};
use uuid::Uuid;

const TOKEN: &str = "beateros-http-pending-worker-smoke-token";
const SESSION_ID: &str = "http-pending-worker-smoke-session";
const ACTION_ID: &str = "http-pending-worker-smoke-action";
const RECEIPT_ID: &str = "http-pending-worker-smoke-receipt";

fn main() -> Result<(), Box<dyn Error>> {
    let mut as_json = false;
    for arg in std::env::args().skip(1) {
        match arg.as_str() {
            "--json" => as_json = true,
            other => return Err(format!("unsupported argument: {other}").into()),
        }
    }

    let root = std::env::temp_dir().join(format!(
        "beater-os-http-pending-worker-smoke-{}",
        Uuid::new_v4()
    ));
    let workdir = root.join("work");
    fs::create_dir_all(&workdir)?;
    let token_file = root.join("token");
    fs::write(&token_file, TOKEN)?;

    let runtime = AgentRuntime::open(&root)?;
    let grant_id = default_root_grant_id(SESSION_ID);
    let command = "sh".to_string();
    let args = vec![
        "-c".to_string(),
        "printf http-pending-worker-smoke > http-worker-out.txt".to_string(),
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
        session_id: Some(SESSION_ID.to_string()),
        session: Some(SessionStart::new(
            "agent:http-pending-worker-smoke",
            "workspace:http-pending-worker-smoke",
            "prove HTTP pending local-shell worker dispatch",
        )),
        grants: vec![GrantRequest::new(
            ResourceKind::FilePath,
            cwd.clone(),
            [ActionKind::Execute],
        )],
        steps: vec![RuntimeStep {
            session_id: SESSION_ID.to_string(),
            action_id: Some(ACTION_ID.to_string()),
            tool_id: Some("shell".to_string()),
            action_kind: ActionKind::Execute,
            target: target.clone(),
            resolved_target: Some(target),
            inputs_summary: "execute HTTP pending worker smoke".to_string(),
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
            idempotency_key: Some(ACTION_ID.to_string()),
            compensation_plan: None,
            human_explanation: "HTTP pending local-shell worker action".to_string(),
            external_revoked_handles: BTreeSet::new(),
            observation: None,
        }],
    })?;
    if bundle.projection.runnable_pending_actions != 1 {
        return Err(format!(
            "expected one runnable action before HTTP worker dispatch, found {}",
            bundle.projection.runnable_pending_actions
        )
        .into());
    }

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

    let body = json!({
        "action_id": ACTION_ID,
        "tool": "shell",
        "tool_digest": command_digest,
        "command": command,
        "args": args,
        "cwd": cwd,
        "grants": [grant_id],
        "side_effects": ["local_write"],
        "receipt_id": RECEIPT_ID,
        "timeout_secs": 30,
    });
    let response = match post_json(
        port,
        &format!("/v1/sessions/{SESSION_ID}/actions/execute-local-shell"),
        &body,
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
    if response.status != 200 {
        return Err(format!(
            "expected 200 from HTTP pending worker dispatch, got {}: {}",
            response.status, response.body
        )
        .into());
    }
    if response.body["dispatch"] != "runnable_pending_action" {
        return Err(format!("expected pending-action dispatch: {}", response.body).into());
    }
    if response.body["execution"]["status"] != "ok" {
        return Err(format!("expected ok execution: {}", response.body).into());
    }
    if response.body["evidence"] != Value::Null {
        return Err(format!(
            "pending-action HTTP dispatch should not fabricate inline evidence: {}",
            response.body
        )
        .into());
    }
    let output_path = workdir.join("http-worker-out.txt");
    let output_text = fs::read_to_string(&output_path)?;
    if output_text != "http-pending-worker-smoke" {
        return Err(format!("unexpected HTTP worker output: {output_text:?}").into());
    }
    let projection = runtime.store().project(SESSION_ID)?;
    let receipts = projection.receipts.len();
    let runnable_pending_actions = projection
        .decisions
        .iter()
        .filter(|decision| {
            decision.result == beater_os_core::DecisionResult::Allowed
                && !projection
                    .receipts
                    .iter()
                    .any(|receipt| receipt.action_id == decision.action_id)
                && !projection
                    .execution_reconciliations
                    .iter()
                    .any(|reconciliation| reconciliation.action_id == decision.action_id)
                && !projection
                    .execution_leases
                    .iter()
                    .any(|lease| lease.action_id == decision.action_id)
        })
        .count();
    if receipts != 1 || runnable_pending_actions != 0 {
        return Err(format!(
            "unexpected projection after HTTP worker dispatch: receipts={} runnable={}",
            receipts, runnable_pending_actions
        )
        .into());
    }

    let report = json!({
        "status": "ok",
        "session_id": SESSION_ID,
        "action_id": ACTION_ID,
        "dispatch": response.body["dispatch"],
        "execution_status": response.body["execution"]["status"],
        "receipt_id": response.body["receipt"]["receipt_id"],
        "receipt_hash": response.body["receipt"]["receipt_hash"],
        "output": output_path.display().to_string(),
        "receipts": receipts,
        "runnable_pending_actions": runnable_pending_actions,
    });
    let _ = fs::remove_dir_all(root);
    if as_json {
        println!("{}", serde_json::to_string_pretty(&report)?);
    } else {
        println!("HTTP pending local-shell worker smoke OK");
        println!("  session: {}", report["session_id"]);
        println!("  action: {}", report["action_id"]);
        println!("  receipt: {}", report["receipt_id"]);
    }
    Ok(())
}

struct HttpResponse {
    status: u16,
    body: Value,
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
