use std::collections::BTreeSet;

use beater_os_core::{
    ActionKind, ActionManifest, AdmissionContext, AgentSession, Budget, CapabilityGrant,
    CapabilityReceiptInput, CapabilityScope, CapabilitySelector, DataClass, DecisionResult,
    GrantConstraints, JournalEvent, PolicyEngine, ResourceKind, RiskClass, SessionStatus,
    SideEffectClass, hash_json,
};
use chrono::{DateTime, TimeDelta, Utc};
use uuid::Uuid;

use crate::args::{self, ParsedArgs};
use crate::error::{CliError, CliResult};
use crate::store::Store;

/// The policy version this CLI stamps onto grants and admission contexts.
/// Kept as a single constant so grants, approvals, and decisions stay
/// consistent, mirroring the invariants the core policy engine checks.
pub const POLICY_VERSION: &str = "beateros-policy-v0";

const DEFAULT_GRANT_TTL_SECS: u64 = 3600;

/// Dispatch a parsed command against the store.
pub fn dispatch(store: &Store, args: &ParsedArgs) -> CliResult<String> {
    let group = args.positional(0).unwrap_or("");
    let sub = args.positional(1).unwrap_or("");
    match (group, sub) {
        ("session", "create") => session_create(store, args),
        ("session", "list") => session_list(store, args),
        ("session", "show") => session_show(store, args),
        ("grant", "issue") => grant_issue(store, args),
        ("action", "propose") => action_propose(store, args),
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
    let session = AgentSession {
        session_id: args
            .get_or("session", &Uuid::new_v4().to_string())
            .to_string(),
        created_at: now,
        created_by,
        agent_id,
        workspace_id: args.require("workspace")?.to_string(),
        goal: args.require("goal")?.to_string(),
        constraints: args.csv("constraint"),
        policy_profile: args.get_or("policy-profile", "default").to_string(),
        initial_capability_ids: BTreeSet::new(),
        budget: Budget::default(),
        model_policy: Default::default(),
        memory_scope: args.get("memory-scope").map(str::to_string),
        journal_root: store.root().display().to_string(),
        status: SessionStatus::Created,
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

fn grant_issue(store: &Store, args: &ParsedArgs) -> CliResult<String> {
    let now = Utc::now();
    let session_id = require_session(store, args)?;
    let projection = store.project(&session_id)?;

    let resource_kind: ResourceKind = args::require_enum(args, "resource-kind")?;
    let resource_id = args.require("resource-id")?.to_string();

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
        constraints.path_prefixes.insert(prefix);
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

    let grant = CapabilityGrant {
        grant_id: args
            .get_or("grant-id", &Uuid::new_v4().to_string())
            .to_string(),
        issuer: projection.session.created_by.clone(),
        holder: projection.session.agent_id.clone(),
        session_id: session_id.clone(),
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
        revocation_handle: Uuid::new_v4().to_string(),
        policy_version: POLICY_VERSION.to_string(),
        reason: args.get_or("reason", "issued via beaterosctl").to_string(),
        revoked: false,
    };

    store.append_event(
        &session_id,
        JournalEvent::CapabilityGranted {
            grant: grant.clone(),
        },
        now,
    )?;

    Ok(format!(
        "issued grant {}\n  holder:  {}\n  scope:   {:?} {} -> {:?}\n  expires: {}",
        grant.grant_id,
        grant.holder,
        grant.scope.selector.resource_kind,
        grant.scope.selector.resource_id,
        grant.scope.actions,
        grant.expires_at
    ))
}

fn action_propose(store: &Store, args: &ParsedArgs) -> CliResult<String> {
    let now = Utc::now();
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
    // / gateway lane), never inferred from the agent's own claimed path. The CLI
    // is the agent surface, so it leaves `resolved_target` unset unless a real
    // mediation point supplies one via `--resolved-target`. Consequently a
    // path-prefix grant fails closed here (core's `path_constraints_allow`
    // requires a resolved target) until the sandbox lane (slice 5) sets it —
    // rather than admitting against the agent's unverified, un-canonicalized path.
    let resolved_target = args.get("resolved-target").map(|value| CapabilitySelector {
        resource_kind: target.resource_kind.clone(),
        resource_id: value.to_string(),
    });

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
        compensation_plan: args.get("compensation-plan").map(str::to_string),
        human_explanation: args
            .get_or("explanation", "proposed via beaterosctl")
            .to_string(),
    };

    // Journal the proposal before admission so the decision has a cause.
    store.append_event(
        &session_id,
        JournalEvent::ActionProposed {
            manifest: manifest.clone(),
        },
        now,
    )?;

    let ctx = AdmissionContext {
        now,
        actor_id: projection.session.agent_id.clone(),
        session_id: session_id.clone(),
        policy_version: POLICY_VERSION.to_string(),
        grants: projection.active_grants(now),
        approvals: Vec::new(),
        simulations: Vec::new(),
    };
    // `admit` is fallible because it digests the manifest; propagate any
    // hashing error rather than pretending a decision was reached.
    let decision = PolicyEngine::new().admit(&manifest, &ctx)?;

    store.append_event(
        &session_id,
        JournalEvent::PolicyDecided {
            decision: decision.clone(),
        },
        now,
    )?;

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

    // Compute the chained receipt without touching disk, journal it (the
    // journal is the source of truth the trace is projected from), then persist
    // the derived ledger line. If a crash interleaves, the journal still holds
    // the receipt and the ledger can be rebuilt from it.
    let receipt = store.stage_receipt(
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
    )?;

    store.append_event(
        &session_id,
        JournalEvent::ReceiptAppended {
            receipt: receipt.clone(),
        },
        now,
    )?;
    store.persist_receipt(&session_id, &receipt)?;

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
        let state = if grant.is_active_at(now) {
            "active"
        } else {
            "inactive"
        };
        lines.push(format!(
            "  - {} [{}] {:?} {} -> {:?}",
            grant.grant_id,
            state,
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
    if !store.session_exists(&session_id) {
        return Err(CliError::SessionNotFound(session_id));
    }
    Ok(session_id)
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
