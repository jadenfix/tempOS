//! Minimal beater-osd runtime entrypoint for hosted agent-kernel bootstrap.
//!
//! This is the first runnable daemon surface for `beaterOS`. It intentionally
//! implements only a strict, auditable bootstrap loop:
//!
//! 1. open/create the daemon store
//! 2. create a session
//! 3. issue a runtime-root capability
//! 4. propose + admit one action via `PolicyEngine`
//! 5. emit one receipt anchored to the same admission boundary
//! 6. project and verify the resulting read model
//!
//! Future `beater-osd` slices (sandbox, scheduler, model routing, hardware
//! surfaces) should extend this CLI surface, but this command exists to keep the
//! runtime contract executable immediately.

use std::collections::{BTreeMap, BTreeSet};
use std::fs;
use std::io::{Read, Write};
use std::net::{SocketAddr, TcpListener, TcpStream};
use std::path::{Path, PathBuf};
use std::process::ExitCode;
use std::time::Duration as StdDuration;

use beater_os_core::{
    ActionKind, ActionManifest, AgentSession, Budget, CapabilityGrant, CapabilityReceiptInput,
    CapabilityScope, CapabilitySelector, DataClass, DecisionResult, DelegationMode,
    GrantConstraints, PolicyDecision, ResourceKind, RiskClass, SessionStatus, SideEffectClass,
    TaintLabel,
};
use beater_os_runtime::{AgentRuntime, RuntimeBundle, RuntimeError};
use beater_os_sandbox::{SandboxLimits, safe_path_environment, validate_environment};
use beater_os_tool_gateway::{
    ExecutionReplayEvidence, GatewayError, LocalToolInvocation, execute_local_tool,
    local_shell_tool_digest_with_environment,
};
use beater_osd::{DAEMON_POLICY_VERSION, DaemonError, LocalShellToolRegistration, Store};
use chrono::{Duration, TimeDelta, Utc};
use serde::{Deserialize, Serialize};
use thiserror::Error;

const DEFAULT_BOOTSTRAP_SESSION_ID: &str = "runtime-bootstrap-session";
const RUNTIME_ROOT_GRANT_ID: &str = "runtime-root-cap";

#[derive(Debug)]
struct Cli {
    command: String,
    root: PathBuf,
    session_id: Option<String>,
    json: bool,
    bind: String,
    token_file: Option<PathBuf>,
    once: bool,
}

#[derive(Debug, Serialize)]
struct RuntimeSmokeReport {
    command: String,
    session_id: String,
    store_root: String,
    decision: String,
    proposal_seq: u64,
    decision_seq: u64,
    journal_records: usize,
    projected_grants: usize,
    projected_manifests: usize,
    projected_receipts: usize,
    receipt_id: String,
    receipt_seq: u64,
}

const USAGE: &str = "\
beater-osd-http — loopback HTTP control plane for the beaterOS daemon

USAGE:
    beater-osd-http [runtime-smoke] [--root <path>] [--session-id <id>] [--json]
    beater-osd-http serve --root <path> --token-file <path> [--bind 127.0.0.1:8787] [--once]

COMMANDS:
    runtime-smoke   Exercise the core daemon contract: session -> grant -> admit -> receipt
    serve           Serve the loopback local control-plane API
";

const DEFAULT_CONTROL_BIND: &str = "127.0.0.1:8787";
const MAX_CONTROL_REQUEST_BYTES: usize = 16 * 1024;
const MIN_CONTROL_TOKEN_BYTES: usize = 16;
const DEFAULT_EXECUTE_TIMEOUT_SECS: u64 = 30;
const MAX_EXECUTE_TIMEOUT_SECS: u64 = 30;

fn main() -> ExitCode {
    let args: Vec<String> = std::env::args().collect();
    match run(&args) {
        Ok(code) => code,
        Err(message) => {
            eprintln!("beater-osd: {message}");
            ExitCode::FAILURE
        }
    }
}

fn run(args: &[String]) -> Result<ExitCode, String> {
    let mut cli = parse_cli(args)?;
    if cli.command == "help" || cli.command == "--help" || cli.command == "-h" {
        println!("{USAGE}");
        return Ok(ExitCode::SUCCESS);
    }
    if cli.command == "serve" {
        let root = canonicalize_or_error(&cli.root)?;
        let token_file = cli
            .token_file
            .as_ref()
            .ok_or_else(|| "serve requires --token-file <path>".to_string())?;
        return run_control_server(root, &cli.bind, token_file, cli.once);
    }
    if cli.command != "runtime-smoke" {
        return Err(format!(
            "{USAGE}unsupported command: {}\nexpected: runtime-smoke or serve",
            cli.command
        ));
    }

    let root = canonicalize_or_error(&cli.root)?;
    let report = run_runtime_smoke(root, cli.session_id.take())?;
    if cli.json {
        let output = serde_json::to_string_pretty(&report)
            .map_err(|err| format!("failed to serialize runtime smoke report: {err}"))?;
        println!("{output}");
        return Ok(ExitCode::SUCCESS);
    }

    println!("runtime-smoke OK");
    println!("  command: {}", report.command);
    println!("  session: {}", report.session_id);
    println!("  decision: {}", report.decision);
    println!("  proposal seq: {}", report.proposal_seq);
    println!("  decision seq: {}", report.decision_seq);
    println!("  journal records: {}", report.journal_records);
    println!(
        "  projection: grants={}, manifests={}, receipts={}",
        report.projected_grants, report.projected_manifests, report.projected_receipts,
    );
    println!("  store root: {}", report.store_root);
    println!(
        "  receipt: {} (seq {})",
        report.receipt_id, report.receipt_seq
    );

    Ok(ExitCode::SUCCESS)
}

fn run_runtime_smoke(
    root: PathBuf,
    session_id_override: Option<String>,
) -> Result<RuntimeSmokeReport, String> {
    let session_id = session_id_override.unwrap_or_else(|| {
        format!(
            "{DEFAULT_BOOTSTRAP_SESSION_ID}-{}-{}",
            Utc::now().timestamp_millis(),
            std::process::id()
        )
    });

    let store = Store::open(&root)
        .map_err(|err| format!("unable to open runtime store at {}: {err}", root.display()))?;
    let created_at = Utc::now();
    let session = build_bootstrap_session(&session_id, &root, created_at);

    store
        .create_session(&session)
        .map_err(|err: DaemonError| format!("create_session failed: {err}"))?;

    let grant = build_bootstrap_grant(&session, created_at);
    store
        .issue_grant(&session_id, grant, Utc::now())
        .map_err(|err: DaemonError| format!("issue runtime grant failed: {err}"))?;

    let manifest = build_bootstrap_manifest(&session_id, created_at);
    let manifest_hash = manifest
        .digest()
        .map_err(|err| format!("failed to hash bootstrap manifest: {err}"))?;
    let outcome = store
        .admit_action(&session_id, manifest.clone())
        .map_err(|err: DaemonError| format!("action admission failed: {err}"))?;
    ensure_decision_allowed(&outcome.decision, &manifest_hash)?;

    let receipt = store
        .append_receipt(
            &session_id,
            build_bootstrap_receipt_input(&manifest),
            Utc::now(),
        )
        .map_err(|err: DaemonError| format!("append_receipt failed: {err}"))?;

    let projection = store
        .project(&session_id)
        .map_err(|err| format!("project failed: {err}"))?;
    let journal = store
        .load_journal(&session_id)
        .map_err(|err| format!("load_journal failed: {err}"))?;

    let receipts = store
        .load_receipts(&session_id)
        .map_err(|err| format!("load_receipts failed: {err}"))?;

    if projection.grants.len() != 1 {
        return Err(format!(
            "projection invariants broken: expected 1 grant, found {}",
            projection.grants.len()
        ));
    }
    if projection.manifests.len() != 1 {
        return Err(format!(
            "projection invariants broken: expected 1 manifest, found {}",
            projection.manifests.len()
        ));
    }
    if receipts.receipts().len() != 1 {
        return Err(format!(
            "projection invariants broken: expected 1 persisted receipt, found {}",
            receipts.receipts().len()
        ));
    }

    Ok(RuntimeSmokeReport {
        command: "runtime-smoke".to_string(),
        session_id,
        store_root: root.display().to_string(),
        decision: decision_result_to_string(&outcome.decision.result).to_string(),
        proposal_seq: outcome.proposal_record.seq,
        decision_seq: outcome.decision_record.seq,
        journal_records: journal.records().len(),
        projected_grants: projection.grants.len(),
        projected_manifests: projection.manifests.len(),
        projected_receipts: projection.receipts.len(),
        receipt_id: receipt.receipt_id,
        receipt_seq: receipt.seq,
    })
}

fn build_bootstrap_session(
    session_id: &str,
    root: &Path,
    created_at: chrono::DateTime<Utc>,
) -> AgentSession {
    AgentSession {
        session_id: session_id.to_string(),
        created_at,
        created_by: "runtime@beaterosd".to_string(),
        agent_id: "agent:runtime".to_string(),
        workspace_id: "workspace:beaterosd".to_string(),
        goal: "Host-side agent-kernel bootstrap smoke".to_string(),
        constraints: Vec::new(),
        policy_profile: "default".to_string(),
        initial_capability_ids: BTreeSet::from([RUNTIME_ROOT_GRANT_ID.to_string()]),
        budget: Budget::default(),
        model_policy: Default::default(),
        memory_scope: None,
        journal_root: root.display().to_string(),
        status: SessionStatus::Running,
    }
}

fn build_bootstrap_grant(session: &AgentSession, now: chrono::DateTime<Utc>) -> CapabilityGrant {
    CapabilityGrant {
        grant_id: RUNTIME_ROOT_GRANT_ID.to_string(),
        issuer: session.created_by.clone(),
        holder: session.agent_id.clone(),
        session_id: session.session_id.clone(),
        parent_grant_id: None,
        scope: CapabilityScope {
            selector: CapabilitySelector {
                resource_kind: ResourceKind::FilePath,
                resource_id: "*".to_string(),
            },
            actions: BTreeSet::from([ActionKind::Read, ActionKind::Write, ActionKind::Execute]),
        },
        denied_actions: BTreeSet::new(),
        constraints: GrantConstraints::default(),
        expires_at: now + TimeDelta::hours(1),
        delegation: DelegationMode::None,
        approval: Default::default(),
        revocation_handle: format!("{RUNTIME_ROOT_GRANT_ID}-revoke"),
        policy_version: DAEMON_POLICY_VERSION.to_string(),
        reason: "runtime bootstrap capability".to_string(),
        revoked: false,
    }
}

fn build_bootstrap_manifest(session_id: &str, now: chrono::DateTime<Utc>) -> ActionManifest {
    let target = CapabilitySelector {
        resource_kind: ResourceKind::FilePath,
        resource_id: "/tmp/beateros-runtime-smoke.out".to_string(),
    };
    ActionManifest {
        action_id: format!("{session_id}-bootstrap-action"),
        session_id: session_id.to_string(),
        tool_id: "tool:beater-osd-runtime".to_string(),
        action_kind: ActionKind::Write,
        target: target.clone(),
        resolved_target: Some(target),
        inputs_digest: "beaterosd-runtime-smoke:input".to_string(),
        inputs_summary: "runtime bootstrap admission".to_string(),
        expected_outputs: Vec::new(),
        expected_side_effects: BTreeSet::from([SideEffectClass::LocalWrite]),
        required_grants: BTreeSet::from([RUNTIME_ROOT_GRANT_ID.to_string()]),
        requested_budget: Budget {
            max_model_cents: None,
            max_tool_calls: None,
            max_wall_ms: Some(5_000),
            max_payment_minor_units: None,
        },
        risk_class: RiskClass::Low,
        data_classes: BTreeSet::from([DataClass::Internal]),
        taint: BTreeSet::new(),
        idempotency_key: Some(format!("bootstrap-{session_id}-{}", now.timestamp())),
        payment_intent: None,
        compensation_plan: None,
        human_explanation: "Bootstrapping runtime authority boundary for local agent kernel"
            .to_string(),
    }
}

fn build_bootstrap_receipt_input(manifest: &ActionManifest) -> CapabilityReceiptInput {
    let now = Utc::now();
    CapabilityReceiptInput {
        receipt_id: Some(format!("receipt-{}", manifest.action_id)),
        action_id: manifest.action_id.clone(),
        tool_id: manifest.tool_id.clone(),
        target: manifest.target.clone(),
        started_at: now,
        finished_at: now + Duration::seconds(1),
        status: "ok".to_string(),
        input_digest: manifest.inputs_digest.clone(),
        output_digest: "beaterosd-runtime-smoke:output".to_string(),
        side_effect_summary: "runtime bootstrap completed".to_string(),
        side_effects: vec![SideEffectClass::LocalWrite],
        external_ids: vec![format!("runtime-smoke-{}", manifest.action_id)],
        artifact_refs: Vec::new(),
        payment_receipt: None,
    }
}

fn ensure_decision_allowed(decision: &PolicyDecision, manifest_hash: &str) -> Result<(), String> {
    if decision.result != DecisionResult::Allowed {
        return Err(format!(
            "runtime admission denied: {} (manifest_hash={manifest_hash})",
            decision.explanation
        ));
    }
    if decision.manifest_hash != manifest_hash {
        return Err(format!(
            "runtime decision hash mismatch: expected {manifest_hash}, found {}",
            decision.manifest_hash
        ));
    }
    Ok(())
}

fn decision_result_to_string(result: &DecisionResult) -> &'static str {
    match result {
        DecisionResult::Allowed => "allowed",
        DecisionResult::Denied => "denied",
        DecisionResult::NeedsApproval => "needs_approval",
        DecisionResult::NeedsSimulation => "needs_simulation",
        DecisionResult::NeedsNarrowedGrant => "needs_narrowed_grant",
    }
}

fn run_control_server(
    root: PathBuf,
    bind: &str,
    token_file: &Path,
    once: bool,
) -> Result<ExitCode, String> {
    let bind: SocketAddr = bind
        .parse()
        .map_err(|err| format!("invalid --bind address {bind:?}: {err}"))?;
    if !bind.ip().is_loopback() {
        return Err("serve refuses non-loopback bind addresses".to_string());
    }
    let token = load_control_token(token_file)?;
    let store = Store::open(&root)
        .map_err(|err| format!("unable to open runtime store at {}: {err}", root.display()))?;
    let listener = TcpListener::bind(bind).map_err(|err| format!("bind {bind} failed: {err}"))?;
    println!(
        "beater-osd control API listening on {}",
        listener.local_addr().map_err(|err| err.to_string())?
    );

    for stream in listener.incoming() {
        let stream = stream.map_err(|err| format!("accept failed: {err}"))?;
        if let Err(err) = handle_control_stream(stream, &store, &token) {
            eprintln!("beater-osd control request refused: {err}");
        }
        if once {
            break;
        }
    }
    Ok(ExitCode::SUCCESS)
}

fn load_control_token(path: &Path) -> Result<String, String> {
    let token = fs::read_to_string(path)
        .map_err(|err| format!("could not read --token-file {}: {err}", path.display()))?
        .trim()
        .to_string();
    if token.len() < MIN_CONTROL_TOKEN_BYTES || token.chars().any(char::is_whitespace) {
        return Err(format!(
            "control token in {} must be at least {MIN_CONTROL_TOKEN_BYTES} non-whitespace bytes",
            path.display()
        ));
    }
    Ok(token)
}

fn handle_control_stream(mut stream: TcpStream, store: &Store, token: &str) -> Result<(), String> {
    stream
        .set_read_timeout(Some(StdDuration::from_secs(2)))
        .map_err(|err| err.to_string())?;
    stream
        .set_write_timeout(Some(StdDuration::from_secs(2)))
        .map_err(|err| err.to_string())?;
    let request = read_control_request(&mut stream)?;
    let response = route_control_request(store, token, &request);
    stream
        .write_all(response.as_bytes())
        .map_err(|err| format!("write response failed: {err}"))?;
    Ok(())
}

fn read_control_request(stream: &mut TcpStream) -> Result<ControlRequest, String> {
    let mut bytes = Vec::new();
    let mut chunk = [0u8; 1024];
    let mut header_end = None;
    loop {
        let n = stream
            .read(&mut chunk)
            .map_err(|err| format!("read request failed: {err}"))?;
        if n == 0 {
            break;
        }
        bytes.extend_from_slice(&chunk[..n]);
        if bytes.len() > MAX_CONTROL_REQUEST_BYTES {
            return Err("control request exceeded size cap".to_string());
        }
        if header_end.is_none() {
            header_end = bytes
                .windows(4)
                .position(|window| window == b"\r\n\r\n")
                .map(|idx| idx + 4);
        }
        if let Some(end) = header_end {
            let content_length = parse_content_length(&bytes[..end])?;
            let total = end
                .checked_add(content_length)
                .ok_or_else(|| "control request length overflow".to_string())?;
            if total > MAX_CONTROL_REQUEST_BYTES {
                return Err("control request exceeded size cap".to_string());
            }
            if bytes.len() >= total {
                bytes.truncate(total);
                break;
            }
        }
    }
    let header_end = header_end.ok_or_else(|| "request header terminator missing".to_string())?;
    let head_text = std::str::from_utf8(&bytes[..header_end])
        .map_err(|_| "request is not utf-8".to_string())?;
    let head = head_text
        .strip_suffix("\r\n\r\n")
        .ok_or_else(|| "request header terminator missing".to_string())?;
    let mut lines = head.split("\r\n");
    let request_line = lines
        .next()
        .ok_or_else(|| "missing request line".to_string())?;
    let mut parts = request_line.split_whitespace();
    let method = parts.next().unwrap_or_default().to_string();
    let path = parts.next().unwrap_or_default().to_string();
    let version = parts.next().unwrap_or_default().to_string();
    if parts.next().is_some() || method.is_empty() || path.is_empty() || version != "HTTP/1.1" {
        return Err("malformed HTTP/1.1 request line".to_string());
    }
    let mut headers = BTreeMap::new();
    for line in lines {
        let (name, value) = line
            .split_once(':')
            .ok_or_else(|| format!("malformed header line {line:?}"))?;
        headers.insert(name.trim().to_ascii_lowercase(), value.trim().to_string());
    }
    Ok(ControlRequest {
        method,
        path,
        headers,
        body: bytes[header_end..].to_vec(),
    })
}

fn parse_content_length(header_bytes: &[u8]) -> Result<usize, String> {
    let text = std::str::from_utf8(header_bytes).map_err(|_| "request is not utf-8".to_string())?;
    let mut content_length = None;
    for line in text.split("\r\n").skip(1) {
        let Some((name, value)) = line.split_once(':') else {
            continue;
        };
        let name = name.trim();
        if name.eq_ignore_ascii_case("transfer-encoding")
            && value.to_ascii_lowercase().contains("chunked")
        {
            return Err("chunked transfer encoding is not supported".to_string());
        }
        if name.eq_ignore_ascii_case("content-length") {
            if content_length.is_some() {
                return Err("duplicate content-length".to_string());
            }
            content_length = Some(
                value
                    .trim()
                    .parse::<usize>()
                    .map_err(|_| "invalid content-length".to_string())?,
            );
        }
    }
    Ok(content_length.unwrap_or(0))
}

fn route_control_request(store: &Store, token: &str, request: &ControlRequest) -> String {
    let (status, body) = match authorize_control_request(token, request) {
        Ok(()) => handle_authorized_control_request(store, request),
        Err(response) => response,
    };
    control_response(status, body)
}

fn authorize_control_request(token: &str, request: &ControlRequest) -> Result<(), (u16, String)> {
    let host = request
        .headers
        .get("host")
        .map(String::as_str)
        .unwrap_or("");
    if !host_allowed(host) {
        return Err((403, json_error("bad_host", "Host must be loopback")));
    }
    if let Some(origin) = request.headers.get("origin")
        && !origin_allowed(origin)
    {
        return Err((403, json_error("bad_origin", "Origin must be loopback")));
    }
    if path_without_query(&request.path) == "/healthz" {
        if request.method == "GET" {
            return Ok(());
        }
        return Err((
            405,
            json_error("method_not_allowed", "healthz supports only GET"),
        ));
    }
    let expected = format!("Bearer {token}");
    if request.headers.get("authorization") != Some(&expected) {
        return Err((
            401,
            json_error("unauthorized", "missing or invalid bearer token"),
        ));
    }
    Ok(())
}

fn handle_authorized_control_request(store: &Store, request: &ControlRequest) -> (u16, String) {
    let path = path_without_query(&request.path);
    match (request.method.as_str(), path) {
        ("GET", "/healthz") => (200, serde_json::json!({ "status": "ok" }).to_string()),
        ("GET", "/v1/sessions") => match store.list_sessions() {
            Ok(sessions) => (200, serde_json::json!({ "sessions": sessions }).to_string()),
            Err(err) => (500, json_error("store_error", &err.to_string())),
        },
        ("POST", "/v1/runtime/bundles") => runtime_bundle_route(store, request),
        ("GET", path) if path.starts_with("/v1/sessions/") => {
            let session_id = path.trim_start_matches("/v1/sessions/");
            if session_id.contains('/') {
                return (404, json_error("not_found", "unknown control-plane route"));
            }
            match store.project(session_id) {
                Ok(projection) => {
                    let scheduler = scheduler_projection(&projection);
                    (
                        200,
                        serde_json::json!({
                            "session_id": projection.session.session_id,
                            "agent_id": projection.session.agent_id,
                            "workspace_id": projection.session.workspace_id,
                            "status": projection.session.status,
                            "grants": projection.grants.len(),
                            "actions": projection.manifests.len(),
                            "decisions": projection.decisions.len(),
                            "pending_allowed_actions": scheduler.pending_allowed_action_ids.len(),
                            "pending_allowed_action_ids": scheduler.pending_allowed_action_ids,
                            "runnable_pending_actions": scheduler.runnable_pending_action_ids.len(),
                            "runnable_pending_action_ids": scheduler.runnable_pending_action_ids,
                            "execution_leases": projection.execution_leases.len(),
                            "open_execution_leases": scheduler.open_execution_lease_ids.len(),
                            "open_execution_lease_ids": scheduler.open_execution_lease_ids,
                            "execution_reconciliations": projection.execution_reconciliations.len(),
                            "recovery_blocked": scheduler.recovery_blocked,
                            "admission_blocked": scheduler.admission_blocked,
                            "admission_blockers": scheduler.admission_blockers,
                            "receipts": projection.receipts.len(),
                        })
                        .to_string(),
                    )
                }
                Err(DaemonError::SessionNotFound(_)) => (
                    404,
                    json_error("session_not_found", "session does not exist"),
                ),
                Err(err) => (500, json_error("store_error", &err.to_string())),
            }
        }
        ("POST", path)
            if path.starts_with("/v1/sessions/")
                && path.ends_with("/actions/execute-local-shell") =>
        {
            let session_id = path
                .trim_start_matches("/v1/sessions/")
                .trim_end_matches("/actions/execute-local-shell");
            if session_id.is_empty() || session_id.contains('/') {
                return (404, json_error("not_found", "unknown control-plane route"));
            }
            execute_local_shell_route(store, session_id, request)
        }
        ("GET" | "POST", _) => (404, json_error("not_found", "unknown control-plane route")),
        _ => (
            405,
            json_error("method_not_allowed", "unsupported method for route"),
        ),
    }
}

struct SchedulerProjection {
    pending_allowed_action_ids: Vec<String>,
    runnable_pending_action_ids: Vec<String>,
    open_execution_lease_ids: Vec<String>,
    recovery_blocked: bool,
    admission_blocked: bool,
    admission_blockers: Vec<String>,
}

fn scheduler_projection(projection: &beater_osd::SessionProjection) -> SchedulerProjection {
    let mut closed_actions: BTreeSet<&str> = projection
        .receipts
        .iter()
        .map(|receipt| receipt.action_id.as_str())
        .collect();
    closed_actions.extend(
        projection
            .execution_reconciliations
            .iter()
            .map(|reconciliation| reconciliation.action_id.as_str()),
    );
    let open_execution_leases: BTreeMap<&str, &str> = projection
        .execution_leases
        .iter()
        .filter(|lease| !closed_actions.contains(lease.action_id.as_str()))
        .map(|lease| (lease.action_id.as_str(), lease.lease_id.as_str()))
        .collect();
    let latest_decisions: BTreeMap<&str, bool> = projection
        .decisions
        .iter()
        .map(|decision| {
            (
                decision.action_id.as_str(),
                decision.result == DecisionResult::Allowed,
            )
        })
        .collect();
    let pending_allowed_action_ids: Vec<String> = latest_decisions
        .iter()
        .filter(|(action_id, allowed)| **allowed && !closed_actions.contains(*action_id))
        .map(|(action_id, _)| (*action_id).to_string())
        .collect();
    let runnable_pending_action_ids: Vec<String> = pending_allowed_action_ids
        .iter()
        .filter(|action_id| !open_execution_leases.contains_key(action_id.as_str()))
        .cloned()
        .collect();
    let open_execution_lease_ids: Vec<String> = open_execution_leases
        .values()
        .map(|lease_id| (*lease_id).to_string())
        .collect();
    let recovery_blocked = !open_execution_lease_ids.is_empty();
    let mut admission_blockers = Vec::new();
    if projection.session.status != SessionStatus::Running {
        admission_blockers.push(format!("session_status:{:?}", projection.session.status));
    }
    if recovery_blocked {
        admission_blockers.push("open_execution_lease".to_string());
    }
    SchedulerProjection {
        pending_allowed_action_ids,
        runnable_pending_action_ids,
        open_execution_lease_ids,
        recovery_blocked,
        admission_blocked: !admission_blockers.is_empty(),
        admission_blockers,
    }
}

fn runtime_bundle_route(store: &Store, request: &ControlRequest) -> (u16, String) {
    if !request.headers.contains_key("content-length") {
        return (
            400,
            json_error("missing_content_length", "POST requires Content-Length"),
        );
    }
    let bundle = match serde_json::from_slice::<RuntimeBundle>(&request.body) {
        Ok(bundle) => bundle,
        Err(err) => return (400, json_error("bad_json", &err.to_string())),
    };
    let runtime = AgentRuntime::from_store(store.clone());
    match runtime.run_bundle(bundle) {
        Ok(outcome) => (
            200,
            serde_json::to_string(&outcome).unwrap_or_else(|err| {
                json_error(
                    "serialize_error",
                    &format!("could not serialize response: {err}"),
                )
            }),
        ),
        Err(RuntimeError::Refused(message)) => (403, json_error("refused", &message)),
        Err(RuntimeError::InvalidTtl(ttl)) => (
            400,
            json_error("bad_request", &format!("invalid ttl seconds: {ttl}")),
        ),
        Err(RuntimeError::Daemon(err)) => (500, json_error("store_error", &err.to_string())),
        Err(RuntimeError::Core(err)) => (500, json_error("core_error", &err.to_string())),
    }
}

fn execute_local_shell_route(
    store: &Store,
    session_id: &str,
    request: &ControlRequest,
) -> (u16, String) {
    if !request.headers.contains_key("content-length") {
        return (
            400,
            json_error("missing_content_length", "POST requires Content-Length"),
        );
    }
    let payload = match serde_json::from_slice::<ExecuteLocalShellRequest>(&request.body) {
        Ok(payload) => payload,
        Err(err) => return (400, json_error("bad_json", &err.to_string())),
    };
    match execute_local_shell_request(store, session_id, payload) {
        Ok(response) => (
            200,
            serde_json::to_string(&response).unwrap_or_else(|err| {
                json_error(
                    "serialize_error",
                    &format!("could not serialize response: {err}"),
                )
            }),
        ),
        Err(ControlExecutionError::BadRequest(message)) => {
            (400, json_error("bad_request", &message))
        }
        Err(ControlExecutionError::Refused(message)) => (403, json_error("refused", &message)),
        Err(ControlExecutionError::Store(err)) => {
            (500, json_error("store_error", &err.to_string()))
        }
        Err(ControlExecutionError::Gateway(err)) => {
            (500, json_error("gateway_error", &err.to_string()))
        }
    }
}

fn execute_local_shell_request(
    store: &Store,
    session_id: &str,
    payload: ExecuteLocalShellRequest,
) -> Result<ExecuteLocalShellResponse, ControlExecutionError> {
    let projection = store.project(session_id)?;
    let action_id = payload
        .action_id
        .unwrap_or_else(|| format!("daemon-exec-{}", Utc::now().timestamp_millis()));
    if projection.manifest(&action_id).is_some() {
        return Err(ControlExecutionError::Refused(format!(
            "action {action_id} was already proposed in this session"
        )));
    }
    let command = required_non_empty("command", payload.command)?;
    let cwd = required_non_empty("cwd", payload.cwd)?;
    if command.contains('/') {
        return Err(ControlExecutionError::Refused(
            "HTTP local shell execution accepts PATH-resolved command names only".to_string(),
        ));
    }
    let tool_id = payload.tool.unwrap_or_else(|| "shell".to_string());
    let command_args = payload.args;
    let mut environment = safe_path_environment();
    for (name, value) in payload.env {
        if name == "PATH" {
            return Err(ControlExecutionError::Refused(
                "PATH is reserved for the sandbox safe system search path".to_string(),
            ));
        }
        if environment.contains_key(&name) {
            return Err(ControlExecutionError::Refused(format!(
                "duplicate environment variable {name:?}"
            )));
        }
        environment.insert(name, value);
    }
    let defaults = SandboxLimits::default();
    validate_environment(&environment, &defaults)
        .map_err(|err| ControlExecutionError::BadRequest(err.to_string()))?;
    let required_grants: BTreeSet<String> = payload.grants.into_iter().collect();
    if required_grants.is_empty() {
        return Err(ControlExecutionError::BadRequest(
            "execute-local-shell requires at least one grant".to_string(),
        ));
    }
    ensure_cwd_inside_grants(&projection, &required_grants, &cwd)?;
    let risk_class = payload.risk.unwrap_or(RiskClass::Low);
    let expected_side_effects: BTreeSet<SideEffectClass> =
        payload.side_effects.into_iter().collect();
    let data_classes: BTreeSet<DataClass> = payload.data_classes.into_iter().collect();
    let taint: BTreeSet<TaintLabel> = payload.taint.into_iter().collect();
    let timeout_secs = payload.timeout_secs.unwrap_or(DEFAULT_EXECUTE_TIMEOUT_SECS);
    if timeout_secs == 0 || timeout_secs > MAX_EXECUTE_TIMEOUT_SECS {
        return Err(ControlExecutionError::BadRequest(format!(
            "timeout_secs must be between 1 and {MAX_EXECUTE_TIMEOUT_SECS}"
        )));
    }
    let max_output_bytes = payload
        .max_output_bytes
        .unwrap_or(defaults.max_output_bytes);
    if max_output_bytes > defaults.max_output_bytes {
        return Err(ControlExecutionError::BadRequest(format!(
            "max_output_bytes must be at most {}",
            defaults.max_output_bytes
        )));
    }
    let limits = SandboxLimits {
        timeout: StdDuration::from_secs(timeout_secs),
        max_output_bytes,
        ..defaults
    };
    validate_environment(&environment, &limits)
        .map_err(|err| ControlExecutionError::BadRequest(err.to_string()))?;

    let computed_digest =
        local_shell_tool_digest_with_environment(&cwd, &command, &command_args, &environment)?;
    let expected_tool_digest = payload
        .tool_digest
        .unwrap_or_else(|| computed_digest.clone());
    let tool_version = payload.tool_version.unwrap_or_else(|| {
        let prefix_len = expected_tool_digest.len().min(16);
        format!("local-{}", &expected_tool_digest[..prefix_len])
    });
    let registry = store.register_local_shell_tool(LocalShellToolRegistration {
        workspace_id: projection.session.workspace_id.clone(),
        tool_id: tool_id.clone(),
        version: tool_version.clone(),
        content_digest: expected_tool_digest.clone(),
        side_effects: expected_side_effects.clone(),
        risk_class,
    })?;
    let outcome = execute_local_tool(
        store,
        &registry,
        LocalToolInvocation {
            session_id: session_id.to_string(),
            tool_id,
            version: tool_version,
            expected_tool_digest: Some(expected_tool_digest),
            command,
            args: command_args,
            cwd,
            environment,
            required_grants,
            revoked_handles: payload.revoked_handles.into_iter().collect(),
            action_id: action_id.clone(),
            risk_class,
            expected_side_effects,
            data_classes,
            taint,
            idempotency_key: payload.idempotency_key,
            compensation_plan: payload.compensation_plan,
            receipt_id: payload.receipt_id,
            human_explanation: payload
                .explanation
                .unwrap_or_else(|| "executed via beater-osd-http control API".to_string()),
            limits,
        },
    )?;
    let resolved = outcome
        .manifest
        .resolved_target
        .as_ref()
        .map(|target| target.resource_id.clone())
        .unwrap_or_else(|| outcome.manifest.target.resource_id.clone());
    let execution = outcome
        .execution
        .as_ref()
        .map(|execution| ExecutionResponse {
            status: execution.status_str().to_string(),
            exit_code: execution.exit_code,
            stdout_digest: execution.stdout_digest(),
            stdout_truncated: execution.stdout_truncated,
            stderr_truncated: execution.stderr_truncated,
            created: execution.diff.created.clone(),
            modified: execution.diff.modified.clone(),
            deleted: execution.diff.deleted.clone(),
        });
    let receipt = outcome.receipt.as_ref().map(|receipt| ReceiptResponse {
        receipt_id: receipt.receipt_id.clone(),
        seq: receipt.seq,
        receipt_hash: receipt.receipt_hash.clone(),
    });
    let evidence = outcome
        .evidence
        .as_ref()
        .map(ExecutionEvidenceResponse::from);
    Ok(ExecuteLocalShellResponse {
        action_id: outcome.manifest.action_id,
        decision: decision_result_to_string(&outcome.decision.result).to_string(),
        explanation: outcome.decision.explanation,
        resolved,
        execution,
        receipt,
        evidence,
    })
}

fn ensure_cwd_inside_grants(
    projection: &beater_osd::SessionProjection,
    required_grants: &BTreeSet<String>,
    cwd: &str,
) -> Result<(), ControlExecutionError> {
    let cwd = fs::canonicalize(cwd).map_err(|err| {
        ControlExecutionError::Refused(format!("cwd must exist and be canonicalizable: {err}"))
    })?;
    let prefixes = projection
        .active_grants(Utc::now())
        .into_iter()
        .filter(|grant| required_grants.contains(&grant.grant_id))
        .flat_map(|grant| grant_confinement_prefixes(&grant))
        .collect::<Vec<_>>();
    if prefixes.is_empty() {
        return Err(ControlExecutionError::Refused(
            "named grants define no filesystem confinement prefix".to_string(),
        ));
    }
    if prefixes.iter().any(|prefix| path_inside(&cwd, prefix)) {
        return Ok(());
    }
    Err(ControlExecutionError::Refused(
        "cwd is outside named grant confinement prefixes".to_string(),
    ))
}

fn grant_confinement_prefixes(grant: &CapabilityGrant) -> Vec<PathBuf> {
    let mut prefixes = grant
        .constraints
        .path_prefixes
        .iter()
        .map(PathBuf::from)
        .collect::<Vec<_>>();
    let selector = &grant.scope.selector;
    if selector.resource_kind == ResourceKind::FilePath && selector.resource_id != "*" {
        prefixes.push(PathBuf::from(&selector.resource_id));
    }
    prefixes
}

fn path_inside(path: &Path, prefix: &Path) -> bool {
    path == prefix || path.starts_with(prefix)
}

fn required_non_empty(field: &str, value: String) -> Result<String, ControlExecutionError> {
    if value.trim().is_empty() {
        return Err(ControlExecutionError::BadRequest(format!(
            "{field} must not be empty"
        )));
    }
    Ok(value)
}

#[derive(Debug, Error)]
enum ControlExecutionError {
    #[error("{0}")]
    BadRequest(String),
    #[error("{0}")]
    Refused(String),
    #[error(transparent)]
    Store(#[from] DaemonError),
    #[error(transparent)]
    Gateway(#[from] GatewayError),
}

#[derive(Debug, Deserialize)]
#[serde(deny_unknown_fields)]
struct ExecuteLocalShellRequest {
    #[serde(default)]
    action_id: Option<String>,
    #[serde(default)]
    tool: Option<String>,
    #[serde(default)]
    tool_version: Option<String>,
    #[serde(default)]
    tool_digest: Option<String>,
    command: String,
    #[serde(default)]
    args: Vec<String>,
    cwd: String,
    #[serde(default)]
    env: BTreeMap<String, String>,
    grants: Vec<String>,
    #[serde(default)]
    revoked_handles: Vec<String>,
    #[serde(default)]
    risk: Option<RiskClass>,
    #[serde(default)]
    side_effects: Vec<SideEffectClass>,
    #[serde(default)]
    data_classes: Vec<DataClass>,
    #[serde(default)]
    taint: Vec<TaintLabel>,
    #[serde(default)]
    idempotency_key: Option<String>,
    #[serde(default)]
    compensation_plan: Option<String>,
    #[serde(default)]
    receipt_id: Option<String>,
    #[serde(default)]
    explanation: Option<String>,
    #[serde(default)]
    timeout_secs: Option<u64>,
    #[serde(default)]
    max_output_bytes: Option<usize>,
}

#[derive(Debug, Serialize)]
struct ExecuteLocalShellResponse {
    action_id: String,
    decision: String,
    explanation: String,
    resolved: String,
    execution: Option<ExecutionResponse>,
    receipt: Option<ReceiptResponse>,
    evidence: Option<ExecutionEvidenceResponse>,
}

#[derive(Debug, Serialize)]
struct ExecutionResponse {
    status: String,
    exit_code: Option<i32>,
    stdout_digest: String,
    stdout_truncated: bool,
    stderr_truncated: bool,
    created: Vec<String>,
    modified: Vec<String>,
    deleted: Vec<String>,
}

#[derive(Debug, Serialize)]
struct ReceiptResponse {
    receipt_id: String,
    seq: u64,
    receipt_hash: String,
}

#[derive(Debug, Serialize)]
struct ExecutionEvidenceResponse {
    session_id: String,
    action_id: String,
    tool_ref: String,
    manifest_hash: String,
    proposal_seq: u64,
    proposal_hash: String,
    decision_seq: u64,
    decision_hash: String,
    admission_journal_root_hash: String,
    lease_id: String,
    lease_seq: u64,
    lease_hash: String,
    receipt_journal_seq: u64,
    receipt_journal_hash: String,
    receipt_seq: u64,
    receipt_hash: String,
    receipt_root_hash: String,
    final_journal_root_hash: String,
}

impl From<&ExecutionReplayEvidence> for ExecutionEvidenceResponse {
    fn from(evidence: &ExecutionReplayEvidence) -> Self {
        Self {
            session_id: evidence.session_id.clone(),
            action_id: evidence.action_id.clone(),
            tool_ref: evidence.tool_ref.clone(),
            manifest_hash: evidence.manifest_hash.clone(),
            proposal_seq: evidence.proposal_seq,
            proposal_hash: evidence.proposal_hash.clone(),
            decision_seq: evidence.decision_seq,
            decision_hash: evidence.decision_hash.clone(),
            admission_journal_root_hash: evidence.admission_journal_root_hash.clone(),
            lease_id: evidence.lease_id.clone(),
            lease_seq: evidence.lease_seq,
            lease_hash: evidence.lease_hash.clone(),
            receipt_journal_seq: evidence.receipt_journal_seq,
            receipt_journal_hash: evidence.receipt_journal_hash.clone(),
            receipt_seq: evidence.receipt_seq,
            receipt_hash: evidence.receipt_hash.clone(),
            receipt_root_hash: evidence.receipt_root_hash.clone(),
            final_journal_root_hash: evidence.final_journal_root_hash.clone(),
        }
    }
}

fn control_response(status: u16, body: String) -> String {
    let reason = match status {
        200 => "OK",
        400 => "Bad Request",
        401 => "Unauthorized",
        403 => "Forbidden",
        404 => "Not Found",
        405 => "Method Not Allowed",
        _ => "Internal Server Error",
    };
    format!(
        "HTTP/1.1 {status} {reason}\r\ncontent-type: application/json\r\ncache-control: no-store\r\ncontent-length: {}\r\nconnection: close\r\n\r\n{body}",
        body.len()
    )
}

fn json_error(code: &str, message: &str) -> String {
    serde_json::json!({ "error": { "code": code, "message": message } }).to_string()
}

fn path_without_query(path: &str) -> &str {
    path.split_once('?').map(|(path, _)| path).unwrap_or(path)
}

fn host_allowed(host: &str) -> bool {
    let host = host.trim();
    if host.eq_ignore_ascii_case("localhost") {
        return true;
    }
    let without_port = host.rsplit_once(':').map(|(host, _)| host).unwrap_or(host);
    matches!(without_port, "127.0.0.1" | "[::1]" | "localhost")
}

fn origin_allowed(origin: &str) -> bool {
    let Some(rest) = origin.strip_prefix("http://") else {
        return false;
    };
    let host = rest.split('/').next().unwrap_or_default();
    host_allowed(host)
}

#[derive(Debug)]
struct ControlRequest {
    method: String,
    path: String,
    headers: BTreeMap<String, String>,
    body: Vec<u8>,
}

fn parse_cli(args: &[String]) -> Result<Cli, String> {
    let mut command = "runtime-smoke".to_string();
    let mut idx = 1;

    if args.len() > 1 && !args[1].starts_with('-') {
        command = args[1].clone();
        idx = 2;
    }

    let mut root = std::env::var("BEATEROS_HOME")
        .map(PathBuf::from)
        .unwrap_or_else(|_| PathBuf::from(".beaterosd"));
    let mut session_id = None;
    let mut json = false;
    let mut bind = DEFAULT_CONTROL_BIND.to_string();
    let mut token_file = None;
    let mut once = false;

    while idx < args.len() {
        match args[idx].as_str() {
            "--help" | "-h" => {
                command = "help".to_string();
                idx += 1;
            }
            "--json" => {
                json = true;
                idx += 1;
            }
            "--root" => {
                let Some(value) = args.get(idx + 1) else {
                    return Err("--root requires <path>".to_string());
                };
                root = PathBuf::from(value);
                idx += 2;
            }
            "--session-id" => {
                let Some(value) = args.get(idx + 1) else {
                    return Err("--session-id requires <id>".to_string());
                };
                session_id = Some(value.to_string());
                idx += 2;
            }
            "--bind" => {
                let Some(value) = args.get(idx + 1) else {
                    return Err("--bind requires <addr:port>".to_string());
                };
                bind = value.to_string();
                idx += 2;
            }
            "--token-file" => {
                let Some(value) = args.get(idx + 1) else {
                    return Err("--token-file requires <path>".to_string());
                };
                token_file = Some(PathBuf::from(value));
                idx += 2;
            }
            "--once" => {
                once = true;
                idx += 1;
            }
            value if value.starts_with('-') => {
                return Err(format!("unsupported option: {value}"));
            }
            other => {
                return Err(format!("unsupported positional argument: {other}\n{USAGE}"));
            }
        }
    }

    Ok(Cli {
        command,
        root,
        session_id,
        json,
        bind,
        token_file,
        once,
    })
}

fn canonicalize_or_error(path: &Path) -> Result<PathBuf, String> {
    if path.is_absolute() {
        return Ok(path.to_path_buf());
    }
    let cwd = std::env::current_dir().map_err(|err| format!("could not determine cwd: {err}"))?;
    Ok(cwd.join(path))
}
