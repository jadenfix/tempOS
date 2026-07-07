use std::collections::BTreeSet;
use std::path::{Component, Path};

use beater_os_core::{
    ActionKind, ActionManifest, AgentSession, Budget, CapabilityGrant, CapabilityReceiptInput,
    CapabilityScope, CapabilitySelector, DataClass, DecisionResult, ExecutionLeaseReconciliation,
    ExecutionLeaseResolution, GrantConstraints, HashValue, PaymentIntent, PaymentMandate,
    PaymentReceiptEvidence, PaymentSettlementStatus, ResourceKind, RiskClass, SessionStatus,
    SideEffectClass, SimulationEvidence, hash_json,
};
use beater_os_sandbox::{SandboxLimits, safe_path_environment, validate_environment};
use beater_os_tool_gateway::{
    GatewayError, LocalToolInvocation, execute_local_tool, local_shell_tool_digest_with_environment,
};
use beater_osd::{DAEMON_POLICY_VERSION, LocalShellToolRegistration, SessionTransition, Store};
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
        ("payment-mandate", "issue") => payment_mandate_issue(store, args),
        ("payment-spend", "propose") => payment_spend_propose(store, args),
        ("action", "propose") => action_propose(store, args),
        ("action", "execute") => action_execute(store, args),
        ("execution-lease", "reconcile") => execution_lease_reconcile(store, args),
        ("simulation", "record") => simulation_record(store, args),
        ("receipt", "record") => receipt_record(store, args),
        ("journal", "verify") => journal_verify(store, args),
        ("trace", "show") => trace_show(store, args),
        ("trace", "export") => trace_export(store, args),
        _ => Err(CliError::Usage(format!(
            "unknown command '{group} {sub}'. Run `beaterosctl help` for usage."
        ))),
    }
}

fn payment_mandate_issue(store: &Store, args: &ParsedArgs) -> CliResult<String> {
    let now = Utc::now();
    let session_id = require_session(store, args)?;
    let projection = store.project(&session_id)?;

    let max_minor_units = require_positive_u64(args, "max-minor-units")?;
    let approval_threshold_minor_units =
        args::get_u64_or(args, "approval-threshold-minor-units", max_minor_units)?;
    if approval_threshold_minor_units > max_minor_units {
        return Err(CliError::Refused(format!(
            "payment mandate approval threshold {approval_threshold_minor_units} exceeds ceiling {max_minor_units}"
        )));
    }
    let expires_at = parse_rfc3339(args.require("expires-at")?, "expires-at")?;
    if expires_at <= now {
        return Err(CliError::Refused(
            "payment mandate expiry must be in the future".to_string(),
        ));
    }

    let counterparty_policy = args.require("counterparty-policy")?.to_string();
    validate_counterparty_policy(&counterparty_policy)?;
    let allowed_adapter_ids = required_non_empty_set(args.csv("adapter"), "adapter")?;
    let allowed_envelope_formats =
        required_non_empty_set(args.csv("envelope-format"), "envelope-format")?;

    let mandate = PaymentMandate {
        mandate_id: require_non_empty(args, "mandate")?.to_string(),
        issuer: args
            .get("issuer")
            .unwrap_or(projection.session.created_by.as_str())
            .to_string(),
        holder: projection.session.agent_id.clone(),
        session_id: session_id.clone(),
        rail: require_non_empty(args, "rail")?.to_string(),
        asset: require_non_empty(args, "asset")?.to_string(),
        max_minor_units,
        counterparty_policy,
        purpose: require_non_empty(args, "purpose")?.to_string(),
        expires_at,
        approval_threshold_minor_units,
        idempotency_key: require_non_empty(args, "payment-idempotency-key")?.to_string(),
        receipt_requirement: "required".to_string(),
        allowed_adapter_ids,
        allowed_envelope_formats,
    };

    let record = store.issue_payment_mandate(&session_id, mandate.clone(), now)?;
    Ok(format!(
        "issued payment mandate {}\n  rail:       {}\n  asset:      {}\n  ceiling:    {}\n  receipt:    {}\n  journal seq: {}",
        mandate.mandate_id,
        mandate.rail,
        mandate.asset,
        mandate.max_minor_units,
        mandate.receipt_requirement,
        record.seq
    ))
}

fn payment_spend_propose(store: &Store, args: &ParsedArgs) -> CliResult<String> {
    let session_id = require_session(store, args)?;
    let projection = store.project(&session_id)?;
    let now = Utc::now();
    let action_id = args
        .get_or("action-id", &Uuid::new_v4().to_string())
        .to_string();
    let mandate_id = require_non_empty(args, "mandate")?;
    let mandate = projection
        .mandates
        .iter()
        .find(|mandate| mandate.mandate_id == mandate_id)
        .ok_or_else(|| {
            CliError::Refused(format!(
                "payment mandate {mandate_id} has not been issued in session {session_id}"
            ))
        })?;
    if mandate.expires_at <= now {
        return Err(CliError::Refused(format!(
            "payment mandate {} is expired",
            mandate.mandate_id
        )));
    }
    if mandate.receipt_requirement != "required" {
        return Err(CliError::Refused(format!(
            "payment mandate {} has unsupported receipt_requirement {}",
            mandate.mandate_id, mandate.receipt_requirement
        )));
    }

    let amount_minor_units = require_positive_u64(args, "amount-minor-units")?;
    if amount_minor_units > mandate.max_minor_units {
        return Err(CliError::Refused(format!(
            "payment amount {amount_minor_units} exceeds mandate ceiling {}",
            mandate.max_minor_units
        )));
    }
    let adapter_id = require_non_empty(args, "adapter-id")?.to_string();
    if !mandate.allowed_adapter_ids.is_empty() && !mandate.allowed_adapter_ids.contains(&adapter_id)
    {
        return Err(CliError::Refused(format!(
            "adapter {adapter_id} is not allowed by payment mandate {}",
            mandate.mandate_id
        )));
    }
    let envelope_format = require_non_empty(args, "envelope-format")?.to_string();
    if !mandate.allowed_envelope_formats.is_empty()
        && !mandate.allowed_envelope_formats.contains(&envelope_format)
    {
        return Err(CliError::Refused(format!(
            "envelope format {envelope_format} is not allowed by payment mandate {}",
            mandate.mandate_id
        )));
    }

    let counterparty_ref = require_non_empty(args, "counterparty-ref")?.to_string();
    let counterparty_binding_hash = require_lower_hex_hash(args, "counterparty-binding-hash")?;
    if !counterparty_allowed(
        &mandate.counterparty_policy,
        &counterparty_ref,
        &counterparty_binding_hash,
    ) {
        return Err(CliError::Refused(format!(
            "counterparty {counterparty_ref} is not allowed by payment mandate {}",
            mandate.mandate_id
        )));
    }

    let envelope_hash = require_lower_hex_hash(args, "envelope-hash")?;
    let envelope_expires_at = match args.get("envelope-expires-at") {
        Some(value) => Some(parse_rfc3339(value, "envelope-expires-at")?),
        None => None,
    };
    if matches!(envelope_expires_at, Some(expires_at) if expires_at <= now) {
        return Err(CliError::Refused(
            "payment envelope expiry must be in the future".to_string(),
        ));
    }

    let required_grants: BTreeSet<String> = args.csv("grants").into_iter().collect();
    if required_grants.is_empty() {
        return Err(CliError::Usage(
            "payment-spend propose requires at least one --grants value".to_string(),
        ));
    }

    let summary = args
        .get_or("summary", "payment spend proposed via beaterosctl")
        .to_string();
    let manifest = ActionManifest {
        action_id: action_id.clone(),
        session_id: session_id.clone(),
        tool_id: args.get_or("tool", "tool:payment").to_string(),
        action_kind: ActionKind::Spend,
        target: CapabilitySelector {
            resource_kind: ResourceKind::PaymentRail,
            resource_id: mandate.rail.clone(),
        },
        resolved_target: None,
        inputs_digest: hash_json(&summary)?,
        inputs_summary: summary,
        expected_outputs: Vec::new(),
        expected_side_effects: BTreeSet::from([SideEffectClass::Payment]),
        required_grants,
        requested_budget: Budget {
            max_payment_minor_units: Some(amount_minor_units),
            ..Budget::default()
        },
        risk_class: RiskClass::Critical,
        data_classes: BTreeSet::from([DataClass::Financial]),
        taint: BTreeSet::new(),
        idempotency_key: Some(mandate.idempotency_key.clone()),
        payment_intent: Some(PaymentIntent {
            mandate_id: mandate.mandate_id.clone(),
            rail: mandate.rail.clone(),
            adapter_id,
            adapter_version: args.get("adapter-version").map(str::to_string),
            asset: mandate.asset.clone(),
            amount_minor_units,
            counterparty_ref,
            counterparty_binding_hash,
            purpose: mandate.purpose.clone(),
            payment_idempotency_key: mandate.idempotency_key.clone(),
            envelope_format,
            envelope_hash,
            envelope_expires_at,
        }),
        compensation_plan: args.get("compensation-plan").map(str::to_string),
        human_explanation: args
            .get_or("explanation", "payment spend proposed via beaterosctl")
            .to_string(),
    };

    let decision = store
        .admit_action_with_revoked_handles(&session_id, manifest.clone(), revoked_handles(args))?
        .decision;
    let mut out = vec![
        format!("payment action {}", manifest.action_id),
        format!("  mandate:    {}", mandate.mandate_id),
        format!("  amount:     {} {}", amount_minor_units, mandate.asset),
        format!("  decision:   {:?}", decision.result),
        format!("  explanation: {}", decision.explanation),
    ];
    if !decision.matched_rules.is_empty() {
        out.push(format!(
            "  rules:      {}",
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
        "session {}\n  agent:      {}\n  created_by: {}\n  workspace:  {}\n  status:     {:?}\n  policy:     {}\n  goal:       {}\n  grants:     {} ({} active)\n  mandates:   {}\n  actions:    {}\n  decisions:  {}\n  receipts:   {}",
        session.session_id,
        session.agent_id,
        session.created_by,
        session.workspace_id,
        session.status,
        session.policy_profile,
        session.goal,
        projection.grants.len(),
        projection.active_grants(now).len(),
        projection.mandates.len(),
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
    let raw_path_prefixes = args.csv("path-prefix");
    let raw_resource_id = match args.get("resource-id") {
        Some(resource_id) => resource_id,
        None if resource_kind == ResourceKind::FilePath && !raw_path_prefixes.is_empty() => "*",
        None => return Err(CliError::MissingFlag("resource-id".to_string())),
    };
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
    for prefix in raw_path_prefixes {
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
/// The CLI asks the daemon store to persist the exact local-shell tool digest,
/// then gives the gateway a registry loaded from daemon-owned durable storage.
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

    let computed_digest =
        local_shell_tool_digest_with_environment(&cwd, &command, &command_args, &environment)?;
    let expected_tool_digest = args
        .get("tool-digest")
        .map(str::to_string)
        .unwrap_or_else(|| computed_digest.clone());
    let tool_version = args
        .get("tool-version")
        .map(str::to_string)
        .unwrap_or_else(|| {
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

fn simulation_record(store: &Store, args: &ParsedArgs) -> CliResult<String> {
    let now = Utc::now();
    let session_id = require_session(store, args)?;
    let projection = store.project(&session_id)?;
    let action_id = args.require("action")?.to_string();
    let manifest = projection
        .manifest(&action_id)
        .ok_or_else(|| CliError::Refused(format!("action {action_id} was never proposed")))?
        .clone();
    let latest_decision = projection.latest_decision(&action_id).ok_or_else(|| {
        CliError::Refused(format!(
            "action {action_id} has no policy decision requiring simulation"
        ))
    })?;
    if latest_decision.result != DecisionResult::NeedsSimulation {
        return Err(CliError::Refused(format!(
            "action {action_id} latest decision is {:?}, not NeedsSimulation",
            latest_decision.result
        )));
    }
    let scenario_id = match args.get("scenario-id") {
        Some(value) if !value.trim().is_empty() => value.to_string(),
        Some(value) => return Err(CliError::invalid("scenario-id", value)),
        None => latest_decision.required_simulation.clone().ok_or_else(|| {
            CliError::Refused(format!(
                "action {action_id} latest NeedsSimulation decision has no scenario id"
            ))
        })?,
    };
    let passed_at = match args.get("passed-at") {
        Some(value) => parse_rfc3339(value, "passed-at")?,
        None => now,
    };
    let simulation = SimulationEvidence {
        simulation_id: args
            .get("simulation-id")
            .map(str::to_string)
            .unwrap_or_else(|| format!("sim-{action_id}")),
        action_id: action_id.clone(),
        manifest_hash: manifest.digest()?,
        scenario_id,
        passed_at,
        policy_version: POLICY_VERSION.to_string(),
    };
    let record = store.record_simulation(&session_id, simulation.clone(), now)?;
    Ok(format!(
        "recorded simulation {} for action {}\n  scenario:   {}\n  journal seq: {}",
        simulation.simulation_id, simulation.action_id, simulation.scenario_id, record.seq
    ))
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

    let payment_receipt = payment_receipt_from_args(args, &projection, &manifest)?;

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
            payment_receipt,
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

fn execution_lease_reconcile(store: &Store, args: &ParsedArgs) -> CliResult<String> {
    let session_id = require_session(store, args)?;
    let projection = store.project(&session_id)?;
    let action_id = require_non_empty(args, "action")?.to_string();
    let lease_id = require_non_empty(args, "lease-id")?.to_string();
    let resolution = args::require_enum::<ExecutionLeaseResolution>(args, "resolution")?;
    if resolution != ExecutionLeaseResolution::OutcomeUnknown {
        return Err(CliError::Refused(
            "only outcome_unknown execution lease reconciliation is currently supported"
                .to_string(),
        ));
    }
    let reason = require_non_empty(args, "reason")?.to_string();
    let reconciled_by = args
        .get("reconciled-by")
        .filter(|value| !value.trim().is_empty() && *value != "true")
        .unwrap_or(projection.session.created_by.as_str())
        .to_string();
    let evidence_refs = args.all("evidence");
    if evidence_refs
        .iter()
        .any(|evidence| evidence.trim().is_empty() || evidence == "true")
    {
        return Err(CliError::invalid("evidence", ""));
    }
    let Some(open_lease) = store
        .open_execution_leases(&session_id)?
        .into_iter()
        .find(|lease| lease.action_id == action_id && lease.lease_id == lease_id)
    else {
        return Err(CliError::Refused(format!(
            "session {session_id} has no open execution lease {lease_id} for action {action_id}"
        )));
    };
    let reconciliation_id = args
        .get("reconciliation-id")
        .filter(|value| !value.trim().is_empty() && *value != "true")
        .map(str::to_string)
        .unwrap_or_else(|| format!("reconcile-{lease_id}"));
    let record = store.reconcile_execution_lease(
        &session_id,
        ExecutionLeaseReconciliation {
            reconciliation_id: reconciliation_id.clone(),
            lease_id: lease_id.clone(),
            session_id: session_id.clone(),
            action_id: action_id.clone(),
            manifest_hash: open_lease.manifest_hash,
            decision_id: open_lease.decision_id,
            resolution,
            reconciled_by,
            reason: reason.clone(),
            evidence_refs,
            reconciled_at: Utc::now(),
        },
        Utc::now(),
    )?;
    let resolution_label = match resolution {
        ExecutionLeaseResolution::OutcomeUnknown => "outcome_unknown",
    };
    Ok(format!(
        "reconciled execution lease {lease_id}\n  reconciliation: {reconciliation_id}\n  action:         {action_id}\n  resolution:     {resolution_label}\n  reason:         {reason}\n  journal seq:    {}",
        record.seq
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
    lines.push(format!("payment mandates ({}):", projection.mandates.len()));
    for mandate in &projection.mandates {
        lines.push(format!(
            "  - {} rail={} asset={} ceiling={} receipt={}",
            mandate.mandate_id,
            mandate.rail,
            mandate.asset,
            mandate.max_minor_units,
            mandate.receipt_requirement
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
        if let Some(intent) = &manifest.payment_intent {
            lines.push(format!(
                "      payment:  mandate={} amount={} {} adapter={}",
                intent.mandate_id, intent.amount_minor_units, intent.asset, intent.adapter_id
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
            if let Some(payment_receipt) = &receipt.payment_receipt {
                lines.push(format!(
                    "        payment receipt: status={:?} rail_hash={}",
                    payment_receipt.settlement_status, payment_receipt.rail_receipt_hash
                ));
            }
        }
    }

    Ok(lines.join("\n"))
}

fn trace_export(store: &Store, args: &ParsedArgs) -> CliResult<String> {
    let session_id = args.require("session")?.to_string();
    let export = store.export_session_trace(&session_id)?;
    let journal_root_hash = export
        .journal
        .records
        .last()
        .map(|record| record.hash.as_str())
        .unwrap_or("empty");
    let bundle_id = args
        .get("bundle-id")
        .map(str::to_string)
        .unwrap_or_else(|| format!("{session_id}:{journal_root_hash}"));
    if bundle_id.trim().is_empty() {
        return Err(CliError::Invalid {
            field: "bundle-id".to_string(),
            value: bundle_id,
        });
    }
    let description = args.get("description").map(str::to_string);
    let policy_version = export
        .projection
        .decisions
        .last()
        .map(|decision| decision.policy_version.clone())
        .or_else(|| {
            export
                .projection
                .grants
                .last()
                .map(|grant| grant.policy_version.clone())
        })
        .unwrap_or_else(|| POLICY_VERSION.to_string());
    let bundle = beater_os_audit::TraceBundle {
        bundle_id,
        description,
        policy_version,
        sessions: vec![export.projection.session],
        grants: export.projection.grants,
        payment_mandates: export.projection.mandates,
        approvals: export.projection.approvals,
        simulations: export.projection.simulations,
        manifests: export.projection.manifests,
        decisions: export.projection.decisions,
        receipts: export.projection.receipts,
        journal: export.journal.records,
    };
    Ok(beater_os_audit::trace_bundle_to_json(&bundle)?)
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

fn payment_receipt_from_args(
    args: &ParsedArgs,
    projection: &beater_osd::SessionProjection,
    manifest: &ActionManifest,
) -> CliResult<Option<Box<PaymentReceiptEvidence>>> {
    let has_payment_flags = args.has_flag("rail-receipt-hash")
        || args.has_flag("settlement-status")
        || args.has_flag("settled-at");
    let Some(intent) = manifest.payment_intent.as_ref() else {
        if has_payment_flags {
            return Err(CliError::Refused(
                "payment receipt flags are only valid for payment actions".to_string(),
            ));
        }
        return Ok(None);
    };
    if !manifest
        .expected_side_effects
        .contains(&SideEffectClass::Payment)
    {
        return Err(CliError::Refused(format!(
            "payment action {} must declare the payment side effect before receipt recording",
            manifest.action_id
        )));
    }
    let mandate = projection
        .mandates
        .iter()
        .find(|mandate| mandate.mandate_id == intent.mandate_id)
        .ok_or_else(|| {
            CliError::Refused(format!(
                "payment action {} references missing mandate {}",
                manifest.action_id, intent.mandate_id
            ))
        })?;
    if mandate.receipt_requirement != "required" {
        return Err(CliError::Refused(format!(
            "payment mandate {} has unsupported receipt_requirement {}",
            mandate.mandate_id, mandate.receipt_requirement
        )));
    }
    let settlement_status_raw = require_non_empty(args, "settlement-status")?;
    let settlement_status =
        args::parse_enum::<PaymentSettlementStatus>("settlement-status", settlement_status_raw)?;
    let settled_at = match args.get("settled-at") {
        Some(value) => Some(parse_rfc3339(value, "settled-at")?),
        None => None,
    };
    match settlement_status {
        PaymentSettlementStatus::Settled if settled_at.is_none() => {
            return Err(CliError::MissingFlag("settled-at".to_string()));
        }
        PaymentSettlementStatus::Submitted
        | PaymentSettlementStatus::Failed
        | PaymentSettlementStatus::Canceled
            if settled_at.is_some() =>
        {
            return Err(CliError::Refused(format!(
                "settled-at is only valid when settlement-status is settled, got {settlement_status_raw}"
            )));
        }
        _ => {}
    }
    let rail_receipt_hash = require_lower_hex_hash(args, "rail-receipt-hash")?;
    Ok(Some(Box::new(PaymentReceiptEvidence {
        manifest_hash: manifest.digest()?,
        mandate_id: intent.mandate_id.clone(),
        rail: intent.rail.clone(),
        adapter_id: intent.adapter_id.clone(),
        adapter_version: intent.adapter_version.clone(),
        asset: intent.asset.clone(),
        amount_minor_units: intent.amount_minor_units,
        counterparty_ref: intent.counterparty_ref.clone(),
        counterparty_binding_hash: intent.counterparty_binding_hash.clone(),
        purpose: intent.purpose.clone(),
        payment_idempotency_key: intent.payment_idempotency_key.clone(),
        envelope_format: intent.envelope_format.clone(),
        envelope_hash: intent.envelope_hash.clone(),
        rail_receipt_hash,
        settlement_status,
        settled_at,
    })))
}

fn require_non_empty<'a>(args: &'a ParsedArgs, field: &str) -> CliResult<&'a str> {
    let value = args.require(field)?;
    if value.trim().is_empty() || value == "true" {
        return Err(CliError::invalid(field, value));
    }
    Ok(value)
}

fn require_positive_u64(args: &ParsedArgs, field: &str) -> CliResult<u64> {
    let raw = args.require(field)?;
    let value = raw
        .parse::<u64>()
        .map_err(|_| CliError::invalid(field, raw))?;
    if value == 0 {
        return Err(CliError::invalid(field, raw));
    }
    Ok(value)
}

fn parse_rfc3339(value: &str, field: &str) -> CliResult<DateTime<Utc>> {
    value
        .parse::<DateTime<Utc>>()
        .map_err(|_| CliError::invalid(field, value))
}

fn require_lower_hex_hash(args: &ParsedArgs, field: &str) -> CliResult<HashValue> {
    let value = require_non_empty(args, field)?;
    if !is_lower_hex_hash(value) {
        return Err(CliError::invalid(field, value));
    }
    Ok(value.to_string())
}

fn is_lower_hex_hash(value: &str) -> bool {
    value.len() == 64
        && value
            .bytes()
            .all(|byte| byte.is_ascii_digit() || (b'a'..=b'f').contains(&byte))
}

fn required_non_empty_set(values: Vec<String>, field: &str) -> CliResult<BTreeSet<String>> {
    if values.is_empty() {
        return Err(CliError::MissingFlag(field.to_string()));
    }
    let mut set = BTreeSet::new();
    for value in values {
        if value.trim().is_empty() {
            return Err(CliError::invalid(field, value));
        }
        set.insert(value);
    }
    Ok(set)
}

fn validate_counterparty_policy(policy: &str) -> CliResult<()> {
    if policy == "any" {
        return Ok(());
    }
    if let Some(value) = policy.strip_prefix("exact:")
        && !value.is_empty()
    {
        return Ok(());
    }
    if let Some(value) = policy.strip_prefix("prefix:")
        && !value.is_empty()
    {
        return Ok(());
    }
    if let Some(value) = policy.strip_prefix("hash:")
        && is_lower_hex_hash(value)
    {
        return Ok(());
    }
    if let Some(value) = policy.strip_prefix("allowlist:") {
        let entries: Vec<&str> = value.split(',').collect();
        if !entries.is_empty() && entries.iter().all(|entry| !entry.trim().is_empty()) {
            return Ok(());
        }
    }
    Err(CliError::invalid("counterparty-policy", policy))
}

fn counterparty_allowed(policy: &str, counterparty_ref: &str, binding_hash: &str) -> bool {
    if policy == "any" {
        return true;
    }
    if let Some(value) = policy.strip_prefix("exact:") {
        return counterparty_ref == value;
    }
    if let Some(value) = policy.strip_prefix("prefix:") {
        return counterparty_ref.starts_with(value);
    }
    if let Some(value) = policy.strip_prefix("hash:") {
        return binding_hash == value;
    }
    if let Some(value) = policy.strip_prefix("allowlist:") {
        return value
            .split(',')
            .any(|entry| counterparty_ref == entry.trim());
    }
    false
}
