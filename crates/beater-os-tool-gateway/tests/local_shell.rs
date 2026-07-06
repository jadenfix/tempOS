#![allow(clippy::expect_used, clippy::unwrap_used)]

use std::collections::BTreeSet;
use std::fs;
use std::path::PathBuf;

use beater_os_core::{
    ActionKind, AgentSession, Budget, CapabilityGrant, CapabilityScope, CapabilitySelector,
    DataClass, DelegationMode, GrantConstraints, ResourceKind, RiskClass, SessionStatus,
    SideEffectClass, ToolManifest,
};
use beater_os_sandbox::SandboxLimits;
use beater_os_tool_gateway::{
    GatewayError, LocalToolInvocation, execute_local_tool, local_shell_tool_digest,
};
use beater_os_tool_registry::{
    RegisteredTool, RegistryPolicy, TestStatus, ToolRegistry, ToolTrust,
};
use beater_osd::{DAEMON_POLICY_VERSION, Store, StoreOptions};
use chrono::{TimeDelta, Utc};
use uuid::Uuid;

struct TempDir {
    path: PathBuf,
}

impl TempDir {
    fn new(tag: &str) -> Self {
        let path = std::env::temp_dir().join(format!("beater-gateway-{tag}-{}", Uuid::new_v4()));
        fs::create_dir_all(&path).unwrap();
        Self { path }
    }

    fn canonical(&self) -> String {
        fs::canonicalize(&self.path).unwrap().display().to_string()
    }
}

impl Drop for TempDir {
    fn drop(&mut self) {
        let _ = fs::remove_dir_all(&self.path);
    }
}

fn session(root: &TempDir, session_id: &str) -> AgentSession {
    AgentSession {
        session_id: session_id.to_string(),
        created_at: Utc::now(),
        created_by: "human:owner".to_string(),
        agent_id: "agent:runtime".to_string(),
        workspace_id: "workspace:repo".to_string(),
        goal: "run registered local shell tool".to_string(),
        constraints: Vec::new(),
        policy_profile: "default".to_string(),
        initial_capability_ids: BTreeSet::from(["grant-exec".to_string()]),
        budget: Budget::default(),
        model_policy: Default::default(),
        memory_scope: None,
        journal_root: root.path.display().to_string(),
        status: SessionStatus::Running,
    }
}

fn exec_grant(session_id: &str, prefix: &str) -> CapabilityGrant {
    CapabilityGrant {
        grant_id: "grant-exec".to_string(),
        issuer: "human:owner".to_string(),
        holder: "agent:runtime".to_string(),
        session_id: session_id.to_string(),
        parent_grant_id: None,
        scope: CapabilityScope {
            selector: CapabilitySelector {
                resource_kind: ResourceKind::FilePath,
                resource_id: "*".to_string(),
            },
            actions: BTreeSet::from([ActionKind::Execute]),
        },
        denied_actions: BTreeSet::new(),
        constraints: GrantConstraints {
            max_data_class: Some(DataClass::Code),
            path_prefixes: BTreeSet::from([prefix.to_string()]),
            ..Default::default()
        },
        expires_at: Utc::now() + TimeDelta::hours(1),
        delegation: DelegationMode::None,
        approval: Default::default(),
        revocation_handle: "revoke-exec".to_string(),
        policy_version: DAEMON_POLICY_VERSION.to_string(),
        reason: "gateway test".to_string(),
        revoked: false,
    }
}

fn registry_with_allowlist(allowlist: bool, workdir: &str) -> ToolRegistry {
    let mut registry = ToolRegistry::new(RegistryPolicy {
        require_signature: false,
        ..Default::default()
    });
    let digest = local_shell_tool_digest(
        workdir,
        "sh",
        &["-c".to_string(), "printf gateway > out.txt".to_string()],
    )
    .unwrap();
    registry
        .register(RegisteredTool {
            manifest: ToolManifest {
                tool_id: "tool:shell".to_string(),
                publisher: "beater.tools".to_string(),
                version: "1.0.0".to_string(),
                transport: "local_shell".to_string(),
                required_capabilities: Vec::new(),
                side_effects: BTreeSet::from([SideEffectClass::LocalWrite]),
                risk_class: RiskClass::Low,
                sandbox_required: true,
            },
            content_digest: digest,
            signature: None,
            test_status: TestStatus::Passing,
            trust: ToolTrust::Trusted,
            registered_at: Utc::now(),
            notes: String::new(),
        })
        .unwrap();
    if allowlist {
        registry.set_workspace_allowlist("workspace:repo", ["tool:shell".to_string()]);
    }
    registry
}

fn registry(workdir: &str) -> ToolRegistry {
    registry_with_allowlist(true, workdir)
}

fn daemon_store_with_registered_shell(
    home: &TempDir,
    workdir: &str,
    command: &str,
    args: &[String],
) -> Store {
    let digest = local_shell_tool_digest(workdir, command, args).unwrap();
    let tool_ref = format!("tool:shell@1.0.0#{digest}");
    Store::open_with_options(
        &home.path,
        StoreOptions {
            tool_registry: [(
                tool_ref.clone(),
                ToolManifest {
                    tool_id: tool_ref,
                    publisher: "beater.tools".to_string(),
                    version: "1.0.0".to_string(),
                    transport: "local_shell".to_string(),
                    required_capabilities: Vec::new(),
                    side_effects: BTreeSet::from([SideEffectClass::LocalWrite]),
                    risk_class: RiskClass::Low,
                    sandbox_required: true,
                },
            )]
            .into(),
            ..StoreOptions::default()
        },
    )
    .unwrap()
}

fn registry_for_command(workdir: &str, command: &str, args: &[String]) -> ToolRegistry {
    let mut registry = ToolRegistry::new(RegistryPolicy {
        require_signature: false,
        ..Default::default()
    });
    registry
        .register(RegisteredTool {
            manifest: ToolManifest {
                tool_id: "tool:shell".to_string(),
                publisher: "beater.tools".to_string(),
                version: "1.0.0".to_string(),
                transport: "local_shell".to_string(),
                required_capabilities: Vec::new(),
                side_effects: BTreeSet::from([SideEffectClass::LocalWrite]),
                risk_class: RiskClass::Low,
                sandbox_required: true,
            },
            content_digest: local_shell_tool_digest(workdir, command, args).unwrap(),
            signature: None,
            test_status: TestStatus::Passing,
            trust: ToolTrust::Trusted,
            registered_at: Utc::now(),
            notes: String::new(),
        })
        .unwrap();
    registry.set_workspace_allowlist("workspace:repo", ["tool:shell".to_string()]);
    registry
}

fn invocation(session_id: &str, workdir: &str, action_id: &str) -> LocalToolInvocation {
    let args = vec!["-c".to_string(), "printf gateway > out.txt".to_string()];
    LocalToolInvocation {
        session_id: session_id.to_string(),
        tool_id: "tool:shell".to_string(),
        version: "1.0.0".to_string(),
        expected_tool_digest: Some(local_shell_tool_digest(workdir, "sh", &args).unwrap()),
        command: "sh".to_string(),
        args,
        cwd: workdir.to_string(),
        required_grants: BTreeSet::from(["grant-exec".to_string()]),
        action_id: action_id.to_string(),
        risk_class: RiskClass::Low,
        expected_side_effects: BTreeSet::new(),
        data_classes: BTreeSet::from([DataClass::Code]),
        taint: BTreeSet::new(),
        idempotency_key: None,
        human_explanation: "gateway local shell test".to_string(),
        limits: SandboxLimits::default(),
    }
}

fn invocation_for_command(
    session_id: &str,
    workdir: &str,
    action_id: &str,
    command: &str,
    args: Vec<String>,
) -> LocalToolInvocation {
    let mut request = invocation(session_id, workdir, action_id);
    request.command = command.to_string();
    request.args = args;
    request.expected_tool_digest =
        Some(local_shell_tool_digest(workdir, &request.command, &request.args).unwrap());
    request
}

#[test]
fn gateway_executes_registered_local_shell_tool_and_records_receipt() {
    let home = TempDir::new("home");
    let work = TempDir::new("work");
    let session_id = "sess_gateway";
    let workdir = work.canonical();
    let args = vec!["-c".to_string(), "printf gateway > out.txt".to_string()];
    let store = daemon_store_with_registered_shell(&home, &workdir, "sh", &args);
    store.create_session(&session(&home, session_id)).unwrap();
    store
        .issue_grant(session_id, exec_grant(session_id, &workdir), Utc::now())
        .unwrap();

    let outcome = execute_local_tool(
        &store,
        &registry(&workdir),
        invocation(session_id, &workdir, "act-gateway"),
    )
    .unwrap();

    assert_eq!(
        outcome.decision.result,
        beater_os_core::DecisionResult::Allowed
    );
    assert!(outcome.execution.is_some());
    let receipt = outcome.receipt.as_ref().expect("receipt");
    assert!(receipt.tool_id.starts_with("tool:shell@1.0.0#"));
    assert_eq!(receipt.target.resource_id, workdir);
    assert_eq!(receipt.side_effects, vec![SideEffectClass::LocalWrite]);
    assert_eq!(
        fs::read_to_string(PathBuf::from(&workdir).join("out.txt")).unwrap(),
        "gateway"
    );
    assert_eq!(store.load_receipts(session_id).unwrap().receipts().len(), 1);
}

#[test]
fn gateway_requires_explicit_workspace_tool_allowlist() {
    let home = TempDir::new("home-no-allowlist");
    let work = TempDir::new("work-no-allowlist");
    let store = Store::open(&home.path).unwrap();
    let session_id = "sess_gateway_no_allowlist";
    let workdir = work.canonical();
    store.create_session(&session(&home, session_id)).unwrap();
    store
        .issue_grant(session_id, exec_grant(session_id, &workdir), Utc::now())
        .unwrap();

    let err = execute_local_tool(
        &store,
        &registry_with_allowlist(false, &workdir),
        invocation(session_id, &workdir, "act-no-allowlist"),
    )
    .expect_err("runtime gateway must fail closed without workspace allowlist");

    assert!(
        matches!(
            err,
            GatewayError::MissingWorkspaceAllowlist { ref workspace_id }
                if workspace_id == "workspace:repo"
        ),
        "{err}"
    );
    assert!(!PathBuf::from(&workdir).join("out.txt").exists());
    assert_eq!(store.load_receipts(session_id).unwrap().receipts().len(), 0);
}

#[test]
fn gateway_requires_tool_digest_to_match_exact_command() {
    let home = TempDir::new("home-digest");
    let work = TempDir::new("work-digest");
    let store = Store::open(&home.path).unwrap();
    let session_id = "sess_gateway_digest";
    let workdir = work.canonical();
    store.create_session(&session(&home, session_id)).unwrap();
    store
        .issue_grant(session_id, exec_grant(session_id, &workdir), Utc::now())
        .unwrap();

    let mut request = invocation(session_id, &workdir, "act-digest");
    request.expected_tool_digest = Some("not-the-command-digest".to_string());
    let err = execute_local_tool(&store, &registry(&workdir), request)
        .expect_err("gateway must bind registered digest to the exact command");

    assert!(matches!(err, GatewayError::ToolDigestMismatch), "{err}");
    assert!(!PathBuf::from(&workdir).join("out.txt").exists());
    assert_eq!(store.load_receipts(session_id).unwrap().receipts().len(), 0);
}

#[test]
fn gateway_receipt_records_observed_not_declared_side_effects() {
    let home = TempDir::new("home-noop");
    let work = TempDir::new("work-noop");
    let session_id = "sess_gateway_noop";
    let workdir = work.canonical();
    let args = vec!["-c".to_string(), "true".to_string()];
    let store = daemon_store_with_registered_shell(&home, &workdir, "sh", &args);
    store.create_session(&session(&home, session_id)).unwrap();
    store
        .issue_grant(session_id, exec_grant(session_id, &workdir), Utc::now())
        .unwrap();
    let request = invocation_for_command(session_id, &workdir, "act-noop", "sh", args.clone());

    let outcome = execute_local_tool(
        &store,
        &registry_for_command(&workdir, "sh", &args),
        request,
    )
    .unwrap();

    let receipt = outcome.receipt.as_ref().expect("receipt");
    assert!(receipt.side_effects.is_empty());
    assert!(
        receipt
            .side_effect_summary
            .contains("declared_not_observed")
    );
}

#[test]
fn gateway_digest_changes_when_executable_bytes_change() {
    let work = TempDir::new("work-exe-digest");
    let workdir = work.canonical();
    let tool_path = PathBuf::from(&workdir).join("tool.sh");
    fs::write(&tool_path, "#!/bin/sh\nprintf one\n").unwrap();
    let args: Vec<String> = Vec::new();
    let first = local_shell_tool_digest(&workdir, "./tool.sh", &args).unwrap();

    fs::write(&tool_path, "#!/bin/sh\nprintf two\n").unwrap();
    let second = local_shell_tool_digest(&workdir, "./tool.sh", &args).unwrap();

    assert_ne!(first, second);
}
