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

use std::collections::BTreeSet;
use std::path::{Path, PathBuf};
use std::process::ExitCode;

use beater_os_core::{
    ActionKind, ActionManifest, AgentSession, Budget, CapabilityGrant, CapabilityReceiptInput,
    CapabilityScope, CapabilitySelector, DataClass, DecisionResult, DelegationMode,
    GrantConstraints, PolicyDecision, ResourceKind, RiskClass, SessionStatus, SideEffectClass,
};
use beater_osd::{DAEMON_POLICY_VERSION, DaemonError, Store};
use chrono::{Duration, TimeDelta, Utc};
use serde::Serialize;

const DEFAULT_BOOTSTRAP_SESSION_ID: &str = "runtime-bootstrap-session";
const RUNTIME_ROOT_GRANT_ID: &str = "runtime-root-cap";

#[derive(Debug)]
struct Cli {
    command: String,
    root: PathBuf,
    session_id: Option<String>,
    json: bool,
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
beater-osd — runtime bootstrap surface for the beaterOS daemon

USAGE:
    beater-osd [runtime-smoke] [--root <path>] [--session-id <id>] [--json]

COMMANDS:
    runtime-smoke   Exercise the core daemon contract: session -> grant -> admit -> receipt
";

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
    if cli.command != "runtime-smoke" {
        return Err(format!(
            "{USAGE}unsupported command: {}\nexpected: runtime-smoke",
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
    })
}

fn canonicalize_or_error(path: &Path) -> Result<PathBuf, String> {
    if path.is_absolute() {
        return Ok(path.to_path_buf());
    }
    let cwd = std::env::current_dir().map_err(|err| format!("could not determine cwd: {err}"))?;
    Ok(cwd.join(path))
}
