use std::collections::BTreeSet;
use std::path::{Component, Path};

use beater_os_core::{
    ActionKind, ActionManifest, AgentSession, Budget, CapabilityGrant, CapabilityReceiptInput,
    CapabilityScope, CapabilitySelector, DataClass, DecisionResult, GrantConstraints, ResourceKind,
    RiskClass, SessionStatus, SideEffectClass, ToolManifest, hash_json,
};
use beater_os_sandbox::{SandboxLimits, safe_path_environment, validate_environment};
use beater_os_tool_gateway::{
    GatewayError, LocalToolInvocation, execute_local_tool, local_shell_tool_digest_with_environment,
};
use beater_os_tool_registry::{
    RegisteredTool, RegistryPolicy, TestStatus, ToolRegistry, ToolTrust,
};
use beater_osd::{DAEMON_POLICY_VERSION, SessionTransition, Store};
use chrono::{DateTime, TimeDelta, Utc};
use uuid::Uuid;

use crate::args::{self, ParsedArgs};
use crate::error::{CliError, CliResult};

/// The policy version this CLI stamps onto grants and admission contexts.
/// Kept as a single constant so grants, approvals, and decisions stay
/// consistent, mirroring the invariants the core policy engine checks.
pub const POLICY_VERSION: &str = DAEMON_POLICY_VERSION;

const DEFAULT_GRANT_TTL_SECS: u64 = 3600;

fn revoked_handles(args: &ParsedArgs) -> BTreeSet<String> {
    args.csv("revoked-handle").into_iter().collect()
}

/// Dispatch a parsed command against the store.
pub fn dispatch(store: &Store, args: &ParsedArgs) -> CliResult<String> {
    let group = args.positional(0).unwrap_or("");
    let sub = args.positional(1).unwrap_or("");
    match (group, sub) {
        ("session", "create") => session_create(store, args),
        ("session", "list") => session_list(store, args),
        ("session", "show") => session_show(store, args),
        ("session", "pause") => session_transition(store, args, SessionTransition::Pause),
        ("session", "resume") => session_transition(store, args, SessionTransition::Resume),
        ("session", "cancel") => session_transition(store, args, SessionTransition::Cancel),
        ("grant", "issue") => grant_issue(store, args),
        ("grant", "revoke") => grant_revoke(store, args),
        ("action", "propose") => action_propose(store, args),
        ("action", "execute") => action_execute(store, args),
        ("receipt", "record") => receipt_record(store, args),
        ("journal", "verify") => journal_verify(store, args),
        ("trace", "show") => trace_show(store, args),
        _ => Err(CliError::Usage(format!(
            "unknown command '{group} {sub}'. Run `beaterosctl help` for usage."
        ))),
    }
}

fn session_create(store: &Store, args: &ParsedArgs) -> CliResult<String> {
    let now = Utc::now();
    let agent_id = args.require("agent")?.to_string();
    let created_by = args.get_or("created-by", &agent_id).to_string();
    let session_id = args
        .get_or("session", &Uuid::new_v4().to_string())
        .to_string();
    let initial_capability_ids: BTreeSet<String> = {
        let provided = args.csv("initial-capability-id");
        if provided.is_empty() {
            BTreeSet::from([default_root_grant_id(&session_id)])
        } else {
            provided.into_iter().collect()
        }
    };
    let session = AgentSession {
        session_id,
        created_at: now,
        created_by,
        agent_id,
        workspace_id: args.require("workspace")?.to_string(),
        goal: args.require("goal")?.to_string(),
        constraints: args.csv("constraint"),
        policy_profile: args.get_or("policy-profile", "default").to_string(),
        initial_capability_ids,
        budget: Budget::default(),
        model_policy: Default::default(),
        memory_scope: args.get("memory-scope").map(str::to_string),
        journal_root: store.root().display().to_string(),
        status: SessionStatus::Running,
    };
    store.create_session(&session)?;
    Ok(format!(
        "created session {}\n  agent:     {}\n  workspace: {}\n  goal:      {}",
        session.session_id, session.agent_id, session.workspace_id, session.goal
    ))
}

fn session_list(store: &Store, _args: &ParsedArgs) -> CliResult<String> {
    let ids = store.list_sessions()?;
    if ids.is_empty() {
        return Ok("no sessions in store".to_string());
    }
    let mut lines = vec![format!("{} session(s):", ids.len())];
    for id in ids {
        let projection = store.project(&id)?;
        lines.push(format!(
            "  {}  [{:?}]  {}",
            id, projection.session.status, projection.session.goal
        ));
    }
    Ok(lines.join("\n"))
}

fn session_show(store: &Store, args: &ParsedArgs) -> CliResult<String> {
    let session_id = require_session(store, args)?;
    let projection = store.project(&session_id)?;
    let now = Utc::now();
    let session = &projection.session;
    Ok(format!(
        "session {}\n  agent:      {}\n  created_by: {}\n  workspace:  {}\n  status:     {:?}\n  policy:     {}\n  goal:       {}\n  grants:     {} ({} active)\n  actions:    {}\n  decisions:  {}\n  receipts:   {}",
        session.session_id,
        session.agent_id,
        session.created_by,
        session.workspace_id,
        session.status,
        session.policy_profile,
        session.goal,
        projection.grants.len(),
        projection.active_grants(now).len(),
        projection.manifests.len(),
        projection.decisions.len(),
        projection.receipts.len(),
    ))
}

fn session_transition(
    store: &Store,
    args: &ParsedArgs,
    transition: SessionTransition,
) -> CliResult<String> {
    let session_id = require_session(store, args)?;
    let record = store.transition_session(&session_id, transition, Utc::now())?;
    let projection = store.project(&session_id)?;
    Ok(format!(
        "session {} {:?}\n  status: {:?}\n  journal seq: {}",
        session_id, transition, projection.session.status, record.seq
    ))
}

fn grant_issue(store: &Store, args: &ParsedArgs) -> CliResult<String> {
    let now = Utc::now();
    let session_id = require_session(store, args)?;
    let projection = store.project(&session_id)?;

    let resource_kind: ResourceKind = args::require_enum(args, "resource-kind")?;
    let raw_resource_id = args.require("resource-id")?;
    let resource_id = if resource_kind == ResourceKind::FilePath && raw_resource_id != "*" {
        canonicalize_file_authority_or_lexical("resource-id", raw_resource_id)?
    } else {
        raw_resource_id.to_string()
    };

    let mut actions = BTreeSet::new();
    for token in args.csv("actions") {
        actions.insert(args::parse_enum::<ActionKind>("actions", &token)?);
    }
    if actions.is_empty() {
        return Err(CliError::Usage(
            "a grant must allow at least one --actions value".to_string(),
        ));
    }

    let mut constraints = GrantConstraints::default();
    if let Some(max_risk) = args.get("max-risk") {
        constraints.max_risk = Some(args::parse_enum::<RiskClass>("max-risk", max_risk)?);
    }
    if let Some(max_data) = args.get("max-data-class") {
        constraints.max_data_class =
            Some(args::parse_enum::<DataClass>("max-data-class", max_data)?);
    }
    for prefix in args.csv("path-prefix") {
        constraints
            .path_prefixes
            .insert(canonicalize_existing_file_authority(
                "path-prefix",
                &prefix,
            )?);
    }
    for host in args.csv("network-allow") {
        constraints.network_allowlist.insert(host);
    }

    let ttl = args::get_u64_or(args, "expires-in-secs", DEFAULT_GRANT_TTL_SECS)?;
    let expires_at = now
        .checked_add_signed(
            TimeDelta::try_seconds(ttl as i64)
                .ok_or_else(|| CliError::invalid("expires-in-secs", ttl.to_string()))?,
        )
        .ok_or_else(|| CliError::invalid("expires-in-secs", ttl.to_string()))?;

    let grant_id = args
        .get("grant-id")
        .map(str::to_string)
        .unwrap_or_else(|| default_root_grant_id(&session_id));
    let grant = CapabilityGrant {
        grant_id,
        issuer: projection.session.created_by.clone(),
        holder: projection.session.agent_id.clone(),
        session_id: session_id.clone(),
        parent_grant_id: None,
        scope: CapabilityScope {
            selector: CapabilitySelector {
                resource_kind,
                resource_id,
            },
            actions,
        },
        denied_actions: BTreeSet::new(),
        constraints,
        expires_at,
        delegation: beater_os_core::DelegationMode::None,
        approval: Default::default(),
        revocation_handle: args
            .get_or("revocation-handle", &Uuid::new_v4().to_string())
            .to_string(),
        policy_version: POLICY_VERSION.to_string(),
        reason: args.get_or("reason", "issued via beaterosctl").to_string(),
        revoked: false,
    };

    store.issue_grant(&session_id, grant.clone(), now)?;

    Ok(format!(
        "issued grant {}\n  holder:  {}\n  scope:   {:?} {} -> {:?}\n  revokes: {}\n  expires: {}",
        grant.grant_id,
        grant.holder,
        grant.scope.selector.resource_kind,
        grant.scope.selector.resource_id,
        grant.scope.actions,
        grant.revocation_handle,
        grant.expires_at
    ))
}

fn grant_revoke(store: &Store, args: &ParsedArgs) -> CliResult<String> {
    let session_id = require_session(store, args)?;
    let projection = store.project(&session_id)?;
    let grant_id = args.require("grant-id")?.to_string();
    let grant = projection
        .grants
        .iter()
        .find(|grant| grant.grant_id == grant_id)
        .ok_or_else(|| {
            CliError::Refused(format!(
                "grant {grant_id} has not been issued in session {session_id}"
            ))
        })?
        .clone();
    let revoked_by = args.get_or("revoked-by", &projection.session.created_by);
    let reason = args.require("reason")?;
    let record = store.revoke_grant(&session_id, &grant_id, revoked_by, reason, Utc::now())?;
    Ok(format!(
        "revoked grant {}\n  handle:      {}\n  revoked_by:  {}\n  reason:      {}\n  journal seq: {}",
        grant.grant_id, grant.revocation_handle, revoked_by, reason, record.seq
    ))
}

fn action_propose(store: &Store, args: &ParsedArgs) -> CliResult<String> {
    let session_id = require_session(store, args)?;
    let projection = store.project(&session_id)?;

    // Fail closed on a duplicate action id: core forbids proposing the same
    // action twice, so appending a second ActionProposed would permanently
    // break `journal verify` on an append-only log.
    let action_id = args
        .get_or("action-id", &Uuid::new_v4().to_string())
        .to_string();
    if projection.manifest(&action_id).is_some() {
        return Err(CliError::Refused(format!(
            "action {action_id} was already proposed in this session"
        )));
    }

    let action_kind: ActionKind = args::require_enum(args, "kind")?;
    let target = CapabilitySelector {
        resource_kind: args::require_enum(args, "target-kind")?,
        resource_id: args.require("target")?.to_string(),
    };
    // `resolved_target` is a KERNEL-DERIVED field (final.md §7.4): the canonical,
    // symlink-resolved target must be computed by a mediation point (the sandbox
    // / gateway lane), never inferred from the agent's own claimed path. Raw
    // non-execute proposals may include it as evidence, but core still requires
    // the requested path to remain inside path-prefix grants so it cannot launder
    // authority through a caller-claimed resolved path. Mediated Execute actions
    // use `resolved_target` as authority because the sandbox lane derives it
    // before admission.
    let resolved_target = args.get("resolved-target").map(|value| CapabilitySelector {
        resource_kind: target.resource_kind.clone(),
        resource_id: value.to_string(),
    });
    if action_kind == ActionKind::Execute && resolved_target.is_some() {
        return Err(CliError::Refused(
            "raw execute proposals cannot supply --resolved-target; use action execute so the sandbox mediates the resolved target"
                .to_string(),
        ));
    }

    let inputs_summary = args.get_or("summary", "").to_string();
    let inputs_digest = hash_json(&inputs_summary)?;

    let required_grants: BTreeSet<String> = args.csv("grants").into_iter().collect();

    let risk_class: RiskClass = match args.get("risk") {
        Some(value) => args::parse_enum("risk", value)?,
        None => RiskClass::Low,
    };

    let mut expected_side_effects = BTreeSet::new();
    let declared = args.csv("side-effects");
    if declared.is_empty() {
        if let Some(default_effect) = default_side_effect(&action_kind) {
            expected_side_effects.insert(default_effect);
        }
    } else {
        for token in declared {
            expected_side_effects
                .insert(args::parse_enum::<SideEffectClass>("side-effects", &token)?);
        }
    }

    let mut data_classes = BTreeSet::new();
    for token in args.csv("data-classes") {
        data_classes.insert(args::parse_enum::<DataClass>("data-classes", &token)?);
    }

    let mut taint = BTreeSet::new();
    for token in args.csv("taint") {
        taint.insert(args::parse_enum("taint", &token)?);
    }

    let manifest = ActionManifest {
        action_id,
        session_id: session_id.clone(),
        tool_id: args.require("tool")?.to_string(),
        action_kind,
        target,
        resolved_target,
        inputs_digest,
        inputs_summary,
        expected_outputs: Vec::new(),
        expected_side_effects,
        required_grants,
        requested_budget: Budget::default(),
        risk_class,
        data_classes,
        taint,
        idempotency_key: args.get("idempotency-key").map(str::to_string),
        payment_intent: None,
        compensation_plan: args.get("compensation-plan").map(str::to_string),
        human_explanation: args
            .get_or("explanation", "proposed via beaterosctl")
            .to_string(),
    };

    let decision = store
        .admit_action_with_revoked_handles(&session_id, manifest.clone(), revoked_handles(args))?
        .decision;

    let mut out = vec![
        format!("action {}", manifest.action_id),
        format!("  decision:    {:?}", decision.result),
        format!("  explanation: {}", decision.explanation),
    ];
    if !decision.matched_rules.is_empty() {
        out.push(format!(
            "  rules:       {}",
            decision.matched_rules.join(", ")
        ));
    }
    if let Some(review) = &decision.required_review {
        out.push(format!("  needs review:     {review}"));
    }
    if let Some(sim) = &decision.required_simulation {
        out.push(format!("  needs simulation: {sim}"));
    }
    Ok(out.join("\n"))
}

/// Default wall-clock timeout for a sandboxed execution, in seconds.
const DEFAULT_EXECUTE_TIMEOUT_SECS: u64 = 30;

/// Run a scoped shell action through the registered tool gateway.
///
/// The CLI parses operator intent and constructs a local tool invocation. The
/// gateway owns registry resolution, confinement, manifest derivation,
/// admission, sandbox execution, and receipt append (final.md §8, §10.6, §13.8).
///
/// The current CLI builds an invocation-scoped in-memory registry entry so the
/// runtime path already exercises registry resolution and workspace allowlists.
/// A daemon-owned persistent registry is the next authority-boundary slice.
fn action_execute(store: &Store, args: &ParsedArgs) -> CliResult<String> {
    let session_id = require_session(store, args)?;
    let projection = store.project(&session_id)?;

    let action_id = args
        .get_or("action-id", &Uuid::new_v4().to_string())
        .to_string();
    if projection.manifest(&action_id).is_some() {
        return Err(CliError::Refused(format!(
            "action {action_id} was already proposed in this session"
        )));
    }

    let tool_id = args.require("tool")?.to_string();
    let command = args.require("command")?.to_string();
    let command_args: Vec<String> = args.all("arg");
    let cwd = args.require("cwd")?.to_string();
    let mut environment = safe_path_environment();
    for raw in args.all("env") {
        let (name, value) = parse_env_assignment(&raw)?;
        if name == "PATH" {
            return Err(CliError::Refused(
                "PATH is reserved for the sandbox's safe system search path".to_string(),
            ));
        }
        if environment.contains_key(&name) {
            return Err(CliError::Refused(format!(
                "duplicate environment variable {name:?}"
            )));
        }
        environment.insert(name, value);
    }
    validate_environment(&environment, &SandboxLimits::default())?;

    let required_grants: BTreeSet<String> = args.csv("grants").into_iter().collect();
    if required_grants.is_empty() {
        return Err(CliError::Usage(
            "action execute requires at least one --grants value".to_string(),
        ));
    }

    let risk_class: RiskClass = match args.get("risk") {
        Some(value) => args::parse_enum("risk", value)?,
        None => RiskClass::Low,
    };

    let mut expected_side_effects = BTreeSet::new();
    for token in args.csv("side-effects") {
        expected_side_effects.insert(args::parse_enum::<SideEffectClass>("side-effects", &token)?);
    }

    let mut data_classes = BTreeSet::new();
    for token in args.csv("data-classes") {
        data_classes.insert(args::parse_enum::<DataClass>("data-classes", &token)?);
    }

    let mut taint = BTreeSet::new();
    for token in args.csv("taint") {
        taint.insert(args::parse_enum("taint", &token)?);
    }

    let timeout_secs = args::get_u64_or(args, "timeout-secs", DEFAULT_EXECUTE_TIMEOUT_SECS)?;
    let defaults = SandboxLimits::default();
    let max_output_bytes = match args.get("max-output-bytes") {
        Some(max_output) => max_output
            .parse::<usize>()
            .map_err(|_| CliError::invalid("max-output-bytes", max_output))?,
        None => defaults.max_output_bytes,
    };
    let limits = SandboxLimits {
        timeout: std::time::Duration::from_secs(timeout_secs),
        max_output_bytes,
        ..defaults
    };
    validate_environment(&environment, &limits)?;

    let tool_version = args.get_or("tool-version", "local").to_string();
    let computed_digest =
        local_shell_tool_digest_with_environment(&cwd, &command, &command_args, &environment)?;
    let expected_tool_digest = args
        .get("tool-digest")
        .map(str::to_string)
        .unwrap_or_else(|| computed_digest.clone());
    let registry = local_shell_registry(
        &tool_id,
        &tool_version,
        &expected_tool_digest,
        &projection.session.workspace_id,
        &expected_side_effects,
        risk_class,
    )?;

    let outcome = execute_local_tool(
        store,
        &registry,
        LocalToolInvocation {
            session_id: session_id.clone(),
            tool_id,
            version: tool_version,
            expected_tool_digest: Some(expected_tool_digest),
            command,
            args: command_args,
            cwd,
            environment,
            required_grants,
            revoked_handles: revoked_handles(args),
            action_id: action_id.clone(),
            risk_class,
            expected_side_effects,
            data_classes,
            taint,
            idempotency_key: args.get("idempotency-key").map(str::to_string),
            compensation_plan: args.get("compensation-plan").map(str::to_string),
            receipt_id: args.get("receipt-id").map(str::to_string),
            human_explanation: args
                .get_or("explanation", "executed via beaterOS tool gateway")
                .to_string(),
            limits,
        },
    )
    .map_err(|err| match err {
        GatewayError::Sandbox(source) => CliError::Sandbox(source),
        other => CliError::Gateway(other),
    })?;

    let decision = outcome.decision;
    let manifest = outcome.manifest;
    let resolved = manifest
        .resolved_target
        .as_ref()
        .map(|target| target.resource_id.as_str())
        .unwrap_or(manifest.target.resource_id.as_str());
    let mut out = vec![
        format!("action {}", manifest.action_id),
        format!("  decision:    {:?}", decision.result),
        format!("  explanation: {}", decision.explanation),
        format!("  resolved:    {resolved}"),
    ];

    if decision.result != DecisionResult::Allowed {
        if let Some(review) = &decision.required_review {
            out.push(format!("  needs review:     {review}"));
        }
        if let Some(sim) = &decision.required_simulation {
            out.push(format!("  needs simulation: {sim}"));
        }
        out.push("  execution:   skipped (action not admitted)".to_string());
        return Ok(out.join("\n"));
    }

    let execution = outcome
        .execution
        .ok_or_else(|| CliError::Refused("allowed gateway action did not execute".to_string()))?;
    let receipt = outcome.receipt.ok_or_else(|| {
        CliError::Refused("allowed gateway action did not emit a receipt".to_string())
    })?;

    out.push(format!("  execution:   {}", execution.status_str()));
    out.push(format!("  exit_code:   {:?}", execution.exit_code));
    out.push(format!(
        "  fs-diff:     created={:?} modified={:?} deleted={:?}",
        execution.diff.created, execution.diff.modified, execution.diff.deleted
    ));
    if execution.stdout_truncated || execution.stderr_truncated {
        out.push("  note:        output truncated at cap".to_string());
    }
    out.push(format!(
        "  receipt:     {} hash={}",
        receipt.receipt_id, receipt.receipt_hash
    ));
    Ok(out.join("\n"))
}

fn local_shell_registry(
    tool_id: &str,
    version: &str,
    digest: &str,
    workspace_id: &str,
    side_effects: &BTreeSet<SideEffectClass>,
    risk_class: RiskClass,
) -> CliResult<ToolRegistry> {
    let mut registry = ToolRegistry::new(RegistryPolicy {
        require_signature: false,
        ..Default::default()
    });
    registry.register(RegisteredTool {
        manifest: ToolManifest {
            tool_id: tool_id.to_string(),
            publisher: "beaterosctl.local".to_string(),
            version: version.to_string(),
            transport: "local_shell".to_string(),
            required_capabilities: Vec::new(),
            side_effects: side_effects.clone(),
            risk_class,
            sandbox_required: true,
        },
        content_digest: digest.to_string(),
        signature: None,
        test_status: TestStatus::Passing,
        trust: ToolTrust::Trusted,
        registered_at: Utc::now(),
        notes: "invocation-scoped local shell registry entry".to_string(),
    })?;
    registry.pin(tool_id, version)?;
    registry.set_workspace_allowlist(workspace_id, [tool_id.to_string()]);
    Ok(registry)
}

/// Canonicalize file-path authority before it is written into a grant.
///
/// The sandbox resolves working directories and prefixes with `realpath`.
/// Storing grant authority in the same namespace avoids false denials on macOS
/// aliases such as `/var` -> `/private/var`, while still failing closed for
/// relative paths and `..` components. Existing paths are stored in the
/// canonical realpath namespace used by the sandbox; missing absolute paths are
/// retained as lexical authority for compatibility with existing proposal flows.
fn canonicalize_file_authority_or_lexical(field: &str, value: &str) -> CliResult<String> {
    let path = Path::new(value);
    if !path.is_absolute()
        || path.components().any(|component| {
            matches!(
                component,
                Component::CurDir | Component::ParentDir | Component::Prefix(_)
            )
        })
    {
        return Err(CliError::invalid(field, value));
    }
    match std::fs::canonicalize(path) {
        Ok(canonical) => Ok(canonical.display().to_string()),
        Err(err) if err.kind() == std::io::ErrorKind::NotFound => Ok(value.to_string()),
        Err(err) => Err(CliError::Io(err)),
    }
}

fn canonicalize_existing_file_authority(field: &str, value: &str) -> CliResult<String> {
    let path = Path::new(value);
    if !path.is_absolute()
        || path.components().any(|component| {
            matches!(
                component,
                Component::CurDir | Component::ParentDir | Component::Prefix(_)
            )
        })
    {
        return Err(CliError::invalid(field, value));
    }
    std::fs::canonicalize(path)
        .map(|canonical| canonical.display().to_string())
        .map_err(CliError::Io)
}

fn parse_env_assignment(raw: &str) -> CliResult<(String, String)> {
    let (name, value) = raw
        .split_once('=')
        .ok_or_else(|| CliError::invalid("env", raw))?;
    Ok((name.to_string(), value.to_string()))
}

fn receipt_record(store: &Store, args: &ParsedArgs) -> CliResult<String> {
    let now = Utc::now();
    let session_id = require_session(store, args)?;
    let projection = store.project(&session_id)?;
    let action_id = args.require("action")?.to_string();

    let manifest = projection
        .manifest(&action_id)
        .ok_or_else(|| CliError::Refused(format!("action {action_id} was never proposed")))?
        .clone();

    // Fail closed: a receipt may only be recorded for an action that was
    // admitted. This mirrors the causality rule the core journal verifier
    // enforces, but refuses at write time rather than only at audit time.
    match projection.latest_decision(&action_id).map(|d| &d.result) {
        Some(DecisionResult::Allowed) => {}
        other => {
            return Err(CliError::Refused(format!(
                "action {action_id} was not admitted (latest decision: {})",
                describe_decision(other)
            )));
        }
    }

    let status = args.get_or("status", "ok").to_string();
    let side_effect_summary = args
        .get_or("summary", &manifest.human_explanation)
        .to_string();
    let output_digest = match args.get("output-digest") {
        Some(value) => value.to_string(),
        None => hash_json(&format!("{status}:{side_effect_summary}"))?,
    };

    // Receipts may only declare side effects the manifest predeclared. Core's
    // causality verifier rejects any receipt whose effects are not a subset of
    // the manifest's, so we must fail closed here rather than write a record
    // that would permanently break `journal verify` on an append-only log.
    let side_effects: Vec<SideEffectClass> = if args.has_flag("side-effects") {
        let mut out = Vec::new();
        for token in args.csv("side-effects") {
            let effect = args::parse_enum::<SideEffectClass>("side-effects", &token)?;
            if !manifest.expected_side_effects.contains(&effect) {
                return Err(CliError::Refused(format!(
                    "side effect {effect:?} was not declared by action {action_id}; \
                     receipts may only report predeclared effects"
                )));
            }
            out.push(effect);
        }
        out
    } else {
        manifest.expected_side_effects.iter().cloned().collect()
    };

    let started_at: DateTime<Utc> = match args.get("started-at") {
        Some(value) => value
            .parse()
            .map_err(|_| CliError::invalid("started-at", value))?,
        None => now,
    };

    // The daemon store computes the chained receipt and appends its journal
    // event under the same runtime writer lock.
    let receipt = store.append_receipt(
        &session_id,
        CapabilityReceiptInput {
            receipt_id: args.get("receipt-id").map(str::to_string),
            action_id: action_id.clone(),
            tool_id: manifest.tool_id.clone(),
            target: manifest
                .resolved_target
                .clone()
                .unwrap_or_else(|| manifest.target.clone()),
            started_at,
            finished_at: now,
            status,
            input_digest: manifest.inputs_digest.clone(),
            output_digest,
            side_effect_summary,
            side_effects,
            external_ids: args.csv("external-id"),
            artifact_refs: args.csv("artifact"),
        },
        now,
    )?;

    Ok(format!(
        "recorded receipt {} for action {}\n  status:  {}\n  effects: {:?}\n  hash:    {}",
        receipt.receipt_id,
        receipt.action_id,
        receipt.status,
        receipt.side_effects,
        receipt.receipt_hash
    ))
}

fn journal_verify(store: &Store, args: &ParsedArgs) -> CliResult<String> {
    let session_id = require_session(store, args)?;
    let journal = store.load_journal(&session_id)?;
    let report = journal.verify_chain()?;
    let ledger = store.load_receipts(&session_id)?;
    ledger.verify_chain()?;
    Ok(format!(
        "journal OK\n  events:        {}\n  journal root:  {}\n  receipts:      {}\n  receipt root:  {}",
        report.records,
        report.root_hash,
        ledger.receipts().len(),
        ledger.root_hash()
    ))
}

fn trace_show(store: &Store, args: &ParsedArgs) -> CliResult<String> {
    let session_id = require_session(store, args)?;
    let projection = store.project(&session_id)?;
    let now = Utc::now();
    let session = &projection.session;

    let mut lines = vec![
        format!("=== beaterOS trace: {} ===", session.session_id),
        format!("goal:      {}", session.goal),
        format!(
            "agent:     {}  workspace: {}",
            session.agent_id, session.workspace_id
        ),
        format!(
            "status:    {:?}  policy: {}",
            session.status, session.policy_profile
        ),
        String::new(),
        format!("grants ({}):", projection.grants.len()),
    ];
    for grant in &projection.grants {
        let state = if projection
            .revoked_handles
            .contains(&grant.revocation_handle)
        {
            "revoked"
        } else if grant.is_active_at(now) {
            "active"
        } else {
            "inactive"
        };
        lines.push(format!(
            "  - {} [{}] handle={} {:?} {} -> {:?}",
            grant.grant_id,
            state,
            grant.revocation_handle,
            grant.scope.selector.resource_kind,
            grant.scope.selector.resource_id,
            grant.scope.actions
        ));
    }

    lines.push(String::new());
    lines.push(format!("actions ({}):", projection.manifests.len()));
    for manifest in &projection.manifests {
        lines.push(format!(
            "  - {} {:?} {:?} {}",
            manifest.action_id,
            manifest.action_kind,
            manifest.target.resource_kind,
            manifest.target.resource_id
        ));
        if let Some(resolved_target) = &manifest.resolved_target {
            lines.push(format!(
                "      resolved: {:?} {}",
                resolved_target.resource_kind, resolved_target.resource_id
            ));
        }
        if let Some(decision) = projection.latest_decision(&manifest.action_id) {
            lines.push(format!(
                "      decision: {:?} — {}",
                decision.result, decision.explanation
            ));
        }
        for receipt in projection
            .receipts
            .iter()
            .filter(|receipt| receipt.action_id == manifest.action_id)
        {
            lines.push(format!(
                "      receipt:  {} status={} effects={:?}",
                receipt.receipt_id, receipt.status, receipt.side_effects
            ));
        }
    }

    Ok(lines.join("\n"))
}

/// Resolve the `--session` flag, verifying the session exists.
fn require_session(store: &Store, args: &ParsedArgs) -> CliResult<String> {
    let session_id = args.require("session")?.to_string();
    if !store.session_exists(&session_id)? {
        return Err(CliError::SessionNotFound(session_id));
    }
    Ok(session_id)
}

fn default_root_grant_id(session_id: &str) -> String {
    format!("{session_id}-root-grant")
}

/// The default declared side effect for an action kind, if any.
fn default_side_effect(action_kind: &ActionKind) -> Option<SideEffectClass> {
    match action_kind {
        ActionKind::Write => Some(SideEffectClass::LocalWrite),
        ActionKind::Deploy => Some(SideEffectClass::Deployment),
        ActionKind::Spend => Some(SideEffectClass::Payment),
        ActionKind::Communicate => Some(SideEffectClass::HumanCommunication),
        ActionKind::Remember => Some(SideEffectClass::MemoryWrite),
        ActionKind::Delegate => Some(SideEffectClass::Delegation),
        ActionKind::Read
        | ActionKind::Execute
        | ActionKind::Navigate
        | ActionKind::Submit
        | ActionKind::AskHuman => None,
    }
}

fn describe_decision(decision: Option<&DecisionResult>) -> String {
    match decision {
        Some(result) => format!("{result:?}"),
        None => "missing".to_string(),
    }
}
