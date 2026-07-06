#![allow(clippy::expect_used, clippy::unwrap_used)]

use std::collections::{BTreeMap, BTreeSet};
use std::fs::{self, OpenOptions};
use std::io::Write;
use std::path::PathBuf;
use std::sync::{Arc, Barrier, mpsc};
use std::thread;
use std::time::Duration;

use beater_os_core::{
    ActionKind, ActionManifest, AgentSession, ApprovalEvidence, ApprovalMode, ApprovalRequirement,
    Budget, CapabilityGrant, CapabilityReceiptInput, CapabilityScope, CapabilitySelector,
    DataClass, DecisionResult, DelegationMode, GrantConstraints, JournalEvent, PaymentIntent,
    PaymentMandate, PolicyDecision, ResourceKind, RiskClass, SessionStatus, SideEffectClass,
    SimulationEvidence, ToolManifest,
};
use beater_osd::{DaemonError, SessionTransition, Store, StoreOptions};
use chrono::{TimeDelta, Utc};
use uuid::Uuid;

struct TempDir {
    path: PathBuf,
}

impl TempDir {
    fn new(tag: &str) -> Self {
        let path = std::env::temp_dir().join(format!("beater-osd-{tag}-{}", Uuid::new_v4()));
        fs::create_dir_all(&path).unwrap();
        Self { path }
    }
}

impl Drop for TempDir {
    fn drop(&mut self) {
        let _ = fs::remove_dir_all(&self.path);
    }
}

fn session(root: &TempDir, id: &str) -> AgentSession {
    session_with_initial(root, id, ["grant-write"])
}

fn session_with_initial<const N: usize>(
    root: &TempDir,
    id: &str,
    grant_ids: [&str; N],
) -> AgentSession {
    AgentSession {
        session_id: id.to_string(),
        created_at: Utc::now(),
        created_by: "human:owner".to_string(),
        agent_id: "agent:runtime".to_string(),
        workspace_id: "workspace:repo".to_string(),
        goal: "daemon-owned admission".to_string(),
        constraints: Vec::new(),
        policy_profile: "default".to_string(),
        initial_capability_ids: grant_ids
            .into_iter()
            .map(std::string::ToString::to_string)
            .collect(),
        budget: Budget::default(),
        model_policy: Default::default(),
        memory_scope: None,
        journal_root: root.path.display().to_string(),
        status: SessionStatus::Running,
    }
}

fn grant(session_id: &str) -> CapabilityGrant {
    CapabilityGrant {
        grant_id: "grant-write".to_string(),
        issuer: "human:owner".to_string(),
        holder: "agent:runtime".to_string(),
        session_id: session_id.to_string(),
        parent_grant_id: None,
        scope: CapabilityScope {
            selector: CapabilitySelector {
                resource_kind: ResourceKind::FilePath,
                resource_id: "/workspace/out".to_string(),
            },
            actions: BTreeSet::from([ActionKind::Write]),
        },
        denied_actions: BTreeSet::new(),
        constraints: GrantConstraints::default(),
        expires_at: Utc::now() + TimeDelta::hours(1),
        delegation: DelegationMode::None,
        approval: Default::default(),
        revocation_handle: "revoke-write".to_string(),
        policy_version: "beateros-policy-v0".to_string(),
        reason: "test grant".to_string(),
        revoked: false,
    }
}

fn payment_grant(session_id: &str) -> CapabilityGrant {
    let mut grant = grant(session_id);
    grant.grant_id = "grant-spend".to_string();
    grant.scope.selector.resource_kind = ResourceKind::PaymentRail;
    grant.scope.selector.resource_id = "stablecoin:x402".to_string();
    grant.scope.actions = BTreeSet::from([ActionKind::Spend]);
    grant.constraints.max_risk = Some(RiskClass::Critical);
    grant.constraints.max_data_class = Some(DataClass::Financial);
    grant.constraints.budget.max_payment_minor_units = Some(100);
    grant.revocation_handle = "revoke-spend".to_string();
    grant
}

fn payment_mandate(session_id: &str) -> PaymentMandate {
    PaymentMandate {
        mandate_id: "mandate-spend".to_string(),
        issuer: "human:owner".to_string(),
        holder: "agent:runtime".to_string(),
        session_id: session_id.to_string(),
        rail: "stablecoin:x402".to_string(),
        asset: "USDC".to_string(),
        max_minor_units: 100,
        counterparty_policy: "prefix:vendor:".to_string(),
        purpose: "vendor payment".to_string(),
        expires_at: Utc::now() + TimeDelta::hours(1),
        approval_threshold_minor_units: 10_000,
        idempotency_key: "pay-once".to_string(),
        receipt_requirement: "required".to_string(),
        allowed_adapter_ids: BTreeSet::from(["x402".to_string()]),
        allowed_envelope_formats: BTreeSet::from(["x402-payment-v1".to_string()]),
    }
}

fn payment_manifest(session_id: &str, action_id: &str) -> ActionManifest {
    let mut manifest = manifest(session_id, action_id);
    manifest.tool_id = "tool:payment".to_string();
    manifest.action_kind = ActionKind::Spend;
    manifest.target.resource_kind = ResourceKind::PaymentRail;
    manifest.target.resource_id = "stablecoin:x402".to_string();
    manifest.resolved_target = None;
    manifest.expected_side_effects = BTreeSet::from([SideEffectClass::Payment]);
    manifest.required_grants = BTreeSet::from(["grant-spend".to_string()]);
    manifest.requested_budget.max_payment_minor_units = Some(100);
    manifest.risk_class = RiskClass::Critical;
    manifest.data_classes = BTreeSet::from([DataClass::Financial]);
    manifest.idempotency_key = Some("pay-once".to_string());
    manifest.payment_intent = Some(PaymentIntent {
        mandate_id: "mandate-spend".to_string(),
        rail: "stablecoin:x402".to_string(),
        adapter_id: "x402".to_string(),
        adapter_version: Some("v1".to_string()),
        asset: "USDC".to_string(),
        amount_minor_units: 100,
        counterparty_ref: "vendor:runtime".to_string(),
        counterparty_binding_hash:
            "2222222222222222222222222222222222222222222222222222222222222222".to_string(),
        purpose: "vendor payment".to_string(),
        payment_idempotency_key: "pay-once".to_string(),
        envelope_format: "x402-payment-v1".to_string(),
        envelope_hash: "3333333333333333333333333333333333333333333333333333333333333333"
            .to_string(),
        envelope_expires_at: None,
    });
    manifest
}

fn deploy_grant(session_id: &str) -> CapabilityGrant {
    let mut grant = grant(session_id);
    grant.grant_id = "grant-deploy".to_string();
    grant.scope.selector.resource_kind = ResourceKind::CloudResource;
    grant.scope.selector.resource_id = "staging".to_string();
    grant.scope.actions = BTreeSet::from([ActionKind::Deploy]);
    grant.constraints.max_risk = Some(RiskClass::High);
    grant.approval = ApprovalRequirement {
        mode: ApprovalMode::Human,
        threshold_risk: RiskClass::High,
        reviewer_ids: vec!["user:reviewer".to_string()],
    };
    grant.revocation_handle = "revoke-deploy".to_string();
    grant
}

fn deploy_manifest(session_id: &str, action_id: &str) -> ActionManifest {
    let mut manifest = manifest(session_id, action_id);
    manifest.tool_id = "tool:deploy".to_string();
    manifest.action_kind = ActionKind::Deploy;
    manifest.target.resource_kind = ResourceKind::CloudResource;
    manifest.target.resource_id = "staging".to_string();
    manifest.resolved_target = None;
    manifest.expected_side_effects = BTreeSet::from([SideEffectClass::Deployment]);
    manifest.required_grants = BTreeSet::from(["grant-deploy".to_string()]);
    manifest.risk_class = RiskClass::High;
    manifest.idempotency_key = Some(format!("deploy-{action_id}"));
    manifest
}

fn approval_for(manifest: &ActionManifest) -> ApprovalEvidence {
    ApprovalEvidence {
        review_id: format!("review-{}", manifest.action_id),
        action_id: manifest.action_id.clone(),
        manifest_hash: manifest.digest().unwrap(),
        grant_id: "grant-deploy".to_string(),
        reviewer_id: "user:reviewer".to_string(),
        approved_at: Utc::now(),
        policy_version: "beateros-policy-v0".to_string(),
    }
}

fn simulation_for(manifest: &ActionManifest) -> SimulationEvidence {
    SimulationEvidence {
        simulation_id: format!("sim-{}", manifest.action_id),
        action_id: manifest.action_id.clone(),
        manifest_hash: manifest.digest().unwrap(),
        scenario_id: format!(
            "action:{}:high-risk-side-effect-simulation",
            manifest.action_id
        ),
        passed_at: Utc::now(),
        policy_version: "beateros-policy-v0".to_string(),
    }
}

fn parent_grant(session_id: &str) -> CapabilityGrant {
    let mut grant = grant(session_id);
    grant.grant_id = "grant-parent".to_string();
    grant.scope.selector.resource_id = "*".to_string();
    grant.scope.actions = BTreeSet::from([ActionKind::Read, ActionKind::Write]);
    grant.delegation = DelegationMode::AttenuatedOnly;
    grant.revocation_handle = "revoke-parent".to_string();
    grant
}

fn delegated_grant(session_id: &str) -> CapabilityGrant {
    let mut grant = grant(session_id);
    grant.grant_id = "grant-child".to_string();
    grant.issuer = "agent:runtime".to_string();
    grant.parent_grant_id = Some("grant-parent".to_string());
    grant.expires_at = Utc::now() + TimeDelta::minutes(30);
    grant.revocation_handle = "revoke-child".to_string();
    grant
}

fn manifest(session_id: &str, action_id: &str) -> ActionManifest {
    ActionManifest {
        action_id: action_id.to_string(),
        session_id: session_id.to_string(),
        tool_id: "tool:test".to_string(),
        action_kind: ActionKind::Write,
        target: CapabilitySelector {
            resource_kind: ResourceKind::FilePath,
            resource_id: "/workspace/out".to_string(),
        },
        resolved_target: Some(CapabilitySelector {
            resource_kind: ResourceKind::FilePath,
            resource_id: "/workspace/out".to_string(),
        }),
        inputs_digest: "input-digest".to_string(),
        inputs_summary: "write output".to_string(),
        expected_outputs: Vec::new(),
        expected_side_effects: BTreeSet::from([SideEffectClass::LocalWrite]),
        required_grants: BTreeSet::from(["grant-write".to_string()]),
        requested_budget: Budget::default(),
        risk_class: RiskClass::Low,
        data_classes: BTreeSet::new(),
        taint: BTreeSet::new(),
        idempotency_key: Some(format!("idem-{action_id}")),
        payment_intent: None,
        compensation_plan: None,
        human_explanation: "test action".to_string(),
    }
}

fn receipt_input(action_id: &str) -> CapabilityReceiptInput {
    CapabilityReceiptInput {
        receipt_id: None,
        action_id: action_id.to_string(),
        tool_id: "tool:test".to_string(),
        target: CapabilitySelector {
            resource_kind: ResourceKind::FilePath,
            resource_id: "/workspace/out".to_string(),
        },
        started_at: Utc::now(),
        finished_at: Utc::now(),
        status: "ok".to_string(),
        input_digest: "input-digest".to_string(),
        output_digest: "output-digest".to_string(),
        side_effect_summary: "no side effects".to_string(),
        side_effects: Vec::new(),
        external_ids: Vec::new(),
        artifact_refs: Vec::new(),
    }
}

fn fake_decision(manifest: &ActionManifest) -> PolicyDecision {
    PolicyDecision {
        decision_id: format!("decision-{}", manifest.action_id),
        action_id: manifest.action_id.clone(),
        manifest_hash: manifest.digest().unwrap(),
        policy_version: "caller-supplied".to_string(),
        result: DecisionResult::Allowed,
        matched_rules: vec!["caller-forged".to_string()],
        explanation: "caller forged decision".to_string(),
        required_review: None,
        required_simulation: None,
        created_at: Utc::now(),
    }
}

fn create_store_with_session(root_tag: &str, session_id: &str) -> (TempDir, Store) {
    let root = TempDir::new(root_tag);
    let store = Store::open(&root.path).unwrap();
    store.create_session(&session(&root, session_id)).unwrap();
    (root, store)
}

fn create_store_with_initial<const N: usize>(
    root_tag: &str,
    session_id: &str,
    grant_ids: [&str; N],
) -> (TempDir, Store) {
    let root = TempDir::new(root_tag);
    let store = Store::open(&root.path).unwrap();
    store
        .create_session(&session_with_initial(&root, session_id, grant_ids))
        .unwrap();
    (root, store)
}

fn append_grant(store: &Store, session_id: &str, grant: CapabilityGrant) {
    store.issue_grant(session_id, grant, Utc::now()).unwrap();
}

fn registered_tool(
    tool_id: &str,
    risk_class: RiskClass,
    side_effects: impl IntoIterator<Item = SideEffectClass>,
) -> ToolManifest {
    ToolManifest {
        tool_id: tool_id.to_string(),
        publisher: "beater.local".to_string(),
        version: "1.0.0".to_string(),
        transport: "local".to_string(),
        required_capabilities: Vec::new(),
        side_effects: side_effects.into_iter().collect(),
        risk_class,
        sandbox_required: false,
    }
}

#[test]
fn create_session_normalizes_genesis_status_to_running() {
    let root = TempDir::new("session-genesis-status");
    let store = Store::open(&root.path).unwrap();
    let session_id = "sess_genesis_status";
    let mut caller_session = session(&root, session_id);
    caller_session.status = SessionStatus::Canceled;

    store.create_session(&caller_session).unwrap();

    let projected = store.project(session_id).unwrap();
    assert_eq!(projected.session.status, SessionStatus::Running);
    let journal = store.load_journal(session_id).unwrap();
    assert!(matches!(
        &journal.records()[0].event,
        JournalEvent::SessionCreated { session } if session.status == SessionStatus::Running
    ));
}

#[test]
fn admit_action_appends_proposal_and_decision() {
    let (_root, store) = create_store_with_session("admit-allowed", "sess_admit");
    let session_id = "sess_admit";
    append_grant(&store, session_id, grant(session_id));

    let manifest = manifest(session_id, "act-1");
    let manifest_hash = manifest.digest().unwrap();
    let outcome = store.admit_action(session_id, manifest).unwrap();

    assert_eq!(outcome.decision.result, DecisionResult::Allowed);
    assert_eq!(outcome.decision.manifest_hash, manifest_hash);
    assert_eq!(outcome.proposal_record.seq, 2);
    assert_eq!(outcome.decision_record.seq, 3);

    let journal = store.load_journal(session_id).unwrap();
    journal.verify_chain().unwrap();
    assert_eq!(journal.records().len(), 4);
    assert!(matches!(
        &journal.records()[2].event,
        JournalEvent::ActionProposed { manifest } if manifest.action_id == "act-1"
    ));
    assert!(matches!(
        &journal.records()[3].event,
        JournalEvent::PolicyDecided { decision }
            if decision.action_id == "act-1" && decision.result == DecisionResult::Allowed
    ));
}

#[test]
fn duplicate_action_id_is_refused_without_append() {
    let (_root, store) = create_store_with_session("admit-duplicate", "sess_duplicate");
    let session_id = "sess_duplicate";
    append_grant(&store, session_id, grant(session_id));

    store
        .admit_action(session_id, manifest(session_id, "act-1"))
        .unwrap();
    let before = store.load_journal(session_id).unwrap().records().len();
    let result = store.admit_action(session_id, manifest(session_id, "act-1"));

    assert!(
        matches!(result, Err(DaemonError::Refused(message)) if message.contains("already has an allowed policy decision"))
    );
    let after = store.load_journal(session_id).unwrap().records().len();
    assert_eq!(after, before);
}

#[test]
fn revoked_handle_input_denies_daemon_owned_admission() {
    let (_root, store) = create_store_with_session("admit-revoked-handle", "sess_revoked_handle");
    let session_id = "sess_revoked_handle";
    append_grant(&store, session_id, grant(session_id));

    let outcome = store
        .admit_action_with_revoked_handles(
            session_id,
            manifest(session_id, "act-revoked-handle"),
            BTreeSet::from(["revoke-write".to_string()]),
        )
        .unwrap();

    assert_eq!(outcome.decision.result, DecisionResult::Denied);
    assert!(
        outcome
            .decision
            .explanation
            .contains("revoked, expired, or missing"),
        "{}",
        outcome.decision.explanation
    );
    let journal = store.load_journal(session_id).unwrap();
    journal.verify_chain().unwrap();
    assert_eq!(journal.records().len(), 4);
}

#[test]
fn paused_session_refuses_receipt_append_after_admission() {
    let (_root, store) = create_store_with_session("receipt-paused", "sess_receipt_paused");
    let session_id = "sess_receipt_paused";
    append_grant(&store, session_id, grant(session_id));
    store
        .admit_action(session_id, manifest(session_id, "act-receipt-paused"))
        .unwrap();
    store
        .transition_session(session_id, SessionTransition::Pause, Utc::now())
        .unwrap();
    let before = store.load_journal(session_id).unwrap().records().len();

    let result = store.append_receipt(session_id, receipt_input("act-receipt-paused"), Utc::now());

    assert!(
        matches!(result, Err(DaemonError::Refused(ref message)) if message.contains("not running")),
        "{result:?}"
    );
    let after = store.load_journal(session_id).unwrap().records().len();
    assert_eq!(after, before);
}

#[test]
fn execution_lock_excludes_lifecycle_transition_until_receipt_append() {
    let root = TempDir::new("execute-lifecycle-lock");
    let store = Store::open_with_options(
        &root.path,
        StoreOptions {
            lock_timeout: Duration::from_millis(25),
            lock_poll_interval: Duration::from_millis(1),
            ..StoreOptions::default()
        },
    )
    .unwrap();
    let session_id = "sess_execute_lock";
    store.create_session(&session(&root, session_id)).unwrap();
    append_grant(&store, session_id, grant(session_id));
    store
        .admit_action(session_id, manifest(session_id, "act-execute-lock"))
        .unwrap();

    let store = Arc::new(store);
    let (callback_started_tx, callback_started_rx) = mpsc::channel();
    let (transition_result_tx, transition_result_rx) = mpsc::channel::<(bool, String)>();

    let execute_store = Arc::clone(&store);
    let execute = thread::spawn(move || {
        execute_store.execute_and_append_receipt(
            session_id,
            Utc::now(),
            |projection| -> Result<(CapabilityReceiptInput, &'static str), DaemonError> {
                assert_eq!(projection.session.status, SessionStatus::Running);
                callback_started_tx.send(()).unwrap();

                let (transition_is_err, transition_debug) = transition_result_rx.recv().unwrap();
                assert!(
                    transition_is_err,
                    "transition unexpectedly completed while execution held the lock: {transition_debug}"
                );
                assert!(
                    transition_debug.contains("LockTimeout"),
                    "transition failed for the wrong reason while execution held the lock: {transition_debug}"
                );

                Ok((receipt_input("act-execute-lock"), "callback-finished"))
            },
        )
    });

    callback_started_rx.recv().unwrap();
    let transition_store = Arc::clone(&store);
    let transition = thread::spawn(move || {
        let result =
            transition_store.transition_session(session_id, SessionTransition::Pause, Utc::now());
        transition_result_tx
            .send((result.is_err(), format!("{result:?}")))
            .unwrap();
        result
    });

    let transition_result = transition.join().unwrap();
    assert!(
        matches!(transition_result, Err(DaemonError::LockTimeout(_))),
        "{transition_result:?}"
    );

    let (receipt, outcome) = execute.join().unwrap().unwrap();
    assert_eq!(outcome, "callback-finished");
    assert_eq!(receipt.action_id, "act-execute-lock");
    assert_eq!(
        store.project(session_id).unwrap().session.status,
        SessionStatus::Running
    );

    store
        .transition_session(session_id, SessionTransition::Pause, Utc::now())
        .unwrap();
    let journal = store.load_journal(session_id).unwrap();
    let receipt_pos = journal
        .records()
        .iter()
        .position(|record| {
            matches!(
                &record.event,
                JournalEvent::ReceiptAppended { receipt }
                    if receipt.action_id == "act-execute-lock"
            )
        })
        .unwrap();
    let pause_pos = journal
        .records()
        .iter()
        .position(|record| {
            matches!(
                &record.event,
                JournalEvent::SessionStatusChanged { to, .. }
                    if *to == SessionStatus::Paused
            )
        })
        .unwrap();
    assert!(receipt_pos < pause_pos);
}

#[test]
fn readmission_after_denial_can_use_new_grant_evidence() {
    let (_root, store) = create_store_with_session("admit-readmit", "sess_readmit");
    let session_id = "sess_readmit";

    let first = store
        .admit_action(session_id, manifest(session_id, "act-readmit"))
        .unwrap();
    assert_eq!(first.decision.result, DecisionResult::Denied);

    append_grant(&store, session_id, grant(session_id));
    let second = store
        .admit_action(session_id, manifest(session_id, "act-readmit"))
        .unwrap();

    assert_eq!(second.proposal_record.seq, first.proposal_record.seq);
    assert_eq!(second.decision.result, DecisionResult::Allowed);
    let journal = store.load_journal(session_id).unwrap();
    journal.verify_chain().unwrap();
    let decisions = journal
        .records()
        .iter()
        .filter(|record| matches!(record.event, JournalEvent::PolicyDecided { .. }))
        .count();
    assert_eq!(decisions, 2);
}

#[test]
fn missing_grant_denies_without_execution_authority() {
    let (_root, store) = create_store_with_session("admit-missing", "sess_missing");
    let session_id = "sess_missing";

    let outcome = store
        .admit_action(session_id, manifest(session_id, "act-missing"))
        .unwrap();

    assert_eq!(outcome.decision.result, DecisionResult::Denied);
    assert!(
        outcome
            .decision
            .explanation
            .contains("required grants are missing")
    );
    store
        .load_journal(session_id)
        .unwrap()
        .verify_chain()
        .unwrap();
}

#[test]
fn issued_payment_mandate_is_projected_into_admission() {
    let (_root, store) =
        create_store_with_initial("payment-mandate-admission", "sess_payment", ["grant-spend"]);
    let session_id = "sess_payment";
    store
        .issue_grant(session_id, payment_grant(session_id), Utc::now())
        .unwrap();
    store
        .issue_payment_mandate(session_id, payment_mandate(session_id), Utc::now())
        .unwrap();

    let projection = store.project(session_id).unwrap();
    assert_eq!(projection.mandates.len(), 1);

    let outcome = store
        .admit_action(session_id, payment_manifest(session_id, "act-pay"))
        .unwrap();
    assert_ne!(outcome.decision.result, DecisionResult::Denied);
    assert!(
        outcome
            .decision
            .matched_rules
            .contains(&"payment_authorized_by_mandate".to_string())
    );
}

#[test]
fn unregistered_tool_denies_through_daemon_admission() {
    let (_root, store) = create_store_with_session("admit-unregistered", "sess_unregistered");
    let session_id = "sess_unregistered";
    append_grant(&store, session_id, grant(session_id));

    let mut manifest = manifest(session_id, "act-unregistered");
    manifest.tool_id = "tool:unknown".to_string();

    let outcome = store.admit_action(session_id, manifest).unwrap();

    assert_eq!(outcome.decision.result, DecisionResult::Denied);
    assert!(
        outcome.decision.explanation.contains("not registered"),
        "{}",
        outcome.decision.explanation
    );
}

#[test]
fn registered_deployment_tool_cannot_be_laundered_as_execute_through_daemon() {
    let root = TempDir::new("admit-deploy-launder");
    let store = Store::open_with_options(
        &root.path,
        StoreOptions {
            tool_registry: BTreeMap::from([(
                "tool:deploy".to_string(),
                registered_tool(
                    "tool:deploy",
                    RiskClass::High,
                    [SideEffectClass::Deployment],
                ),
            )]),
            ..StoreOptions::default()
        },
    )
    .unwrap();
    let session_id = "sess_deploy_launder";
    store.create_session(&session(&root, session_id)).unwrap();
    append_grant(&store, session_id, grant(session_id));

    let mut manifest = manifest(session_id, "act-deploy-launder");
    manifest.tool_id = "tool:deploy".to_string();
    manifest.action_kind = ActionKind::Execute;
    manifest.expected_side_effects = BTreeSet::from([SideEffectClass::Deployment]);
    manifest.idempotency_key = Some("deploy-idem".to_string());

    let outcome = store.admit_action(session_id, manifest).unwrap();

    assert_eq!(outcome.decision.result, DecisionResult::Denied);
    assert!(
        outcome.decision.explanation.contains("deploy action kind"),
        "{}",
        outcome.decision.explanation
    );
}

#[test]
fn durable_approval_and_simulation_evidence_allows_high_risk_action() {
    let (_root, store) =
        create_store_with_initial("approval-simulation-loop", "sess_review", ["grant-deploy"]);
    let session_id = "sess_review";
    append_grant(&store, session_id, deploy_grant(session_id));
    let manifest = deploy_manifest(session_id, "act-deploy");

    let first = store.admit_action(session_id, manifest.clone()).unwrap();
    assert_eq!(first.decision.result, DecisionResult::NeedsApproval);

    store
        .record_approval(session_id, approval_for(&manifest), Utc::now())
        .unwrap();
    let second = store.admit_action(session_id, manifest.clone()).unwrap();
    assert_eq!(second.decision.result, DecisionResult::NeedsSimulation);

    store
        .record_simulation(session_id, simulation_for(&manifest), Utc::now())
        .unwrap();
    let third = store.admit_action(session_id, manifest).unwrap();
    assert_eq!(third.decision.result, DecisionResult::Allowed);
}

#[test]
fn simulation_evidence_must_match_latest_required_simulation() {
    let (_root, store) = create_store_with_initial(
        "simulation-requirement",
        "sess_sim_requirement",
        ["grant-deploy"],
    );
    let session_id = "sess_sim_requirement";
    append_grant(&store, session_id, deploy_grant(session_id));
    let manifest = deploy_manifest(session_id, "act-sim-requirement");
    store.admit_action(session_id, manifest.clone()).unwrap();
    store
        .record_approval(session_id, approval_for(&manifest), Utc::now())
        .unwrap();
    let decision = store.admit_action(session_id, manifest.clone()).unwrap();
    assert_eq!(decision.decision.result, DecisionResult::NeedsSimulation);
    let mut simulation = simulation_for(&manifest);
    simulation.scenario_id = "scenario:wrong".to_string();
    let before = store.load_journal(session_id).unwrap().records().len();

    let result = store.record_simulation(session_id, simulation, Utc::now());

    assert!(
        matches!(result, Err(DaemonError::Refused(ref message)) if message.contains("does not match required simulation")),
        "{result:?}"
    );
    let after = store.load_journal(session_id).unwrap().records().len();
    assert_eq!(after, before);
}

#[test]
fn forged_evidence_is_rejected_without_append() {
    let (_root, store) =
        create_store_with_initial("approval-forgery", "sess_forgery", ["grant-deploy"]);
    let session_id = "sess_forgery";
    append_grant(&store, session_id, deploy_grant(session_id));
    let manifest = deploy_manifest(session_id, "act-forgery");
    store.admit_action(session_id, manifest.clone()).unwrap();
    let before = store.load_journal(session_id).unwrap().records().len();

    let mut approval = approval_for(&manifest);
    approval.manifest_hash = "bad-hash".to_string();
    let result = store.record_approval(session_id, approval, Utc::now());

    assert!(
        matches!(result, Err(DaemonError::Refused(message)) if message.contains("manifest hash"))
    );
    let after = store.load_journal(session_id).unwrap().records().len();
    assert_eq!(after, before);
}

#[test]
fn expired_grant_denies_without_execution_authority() {
    let (_root, store) = create_store_with_session("admit-expired", "sess_expired");
    let session_id = "sess_expired";
    let mut expired = grant(session_id);
    expired.expires_at = Utc::now() - TimeDelta::minutes(1);
    append_grant(&store, session_id, expired);

    let outcome = store
        .admit_action(session_id, manifest(session_id, "act-expired"))
        .unwrap();

    assert_eq!(outcome.decision.result, DecisionResult::Denied);
    assert!(
        outcome
            .decision
            .explanation
            .contains("delegation ancestors is revoked, expired, or missing")
    );
}

#[test]
fn revoked_grant_denies_without_execution_authority() {
    let (_root, store) = create_store_with_session("admit-revoked", "sess_revoked");
    let session_id = "sess_revoked";
    let mut revoked = grant(session_id);
    revoked.revoked = true;
    append_grant(&store, session_id, revoked);

    let outcome = store
        .admit_action(session_id, manifest(session_id, "act-revoked"))
        .unwrap();

    assert_eq!(outcome.decision.result, DecisionResult::Denied);
    assert!(
        outcome
            .decision
            .explanation
            .contains("delegation ancestors is revoked, expired, or missing")
    );
}

#[test]
fn manifest_for_other_session_is_refused_without_append() {
    let (_root, store) = create_store_with_session("admit-session-mismatch", "sess_real");
    let session_id = "sess_real";
    append_grant(&store, session_id, grant(session_id));
    let before = store.load_journal(session_id).unwrap().records().len();

    let result = store.admit_action(session_id, manifest("sess_other", "act-other"));

    assert!(
        matches!(result, Err(DaemonError::Refused(message)) if message.contains("does not match"))
    );
    let after = store.load_journal(session_id).unwrap().records().len();
    assert_eq!(after, before);
}

#[test]
fn raw_policy_decisions_are_refused_by_public_append_event() {
    let (_root, store) = create_store_with_session("raw-policy", "sess_raw_policy");
    let session_id = "sess_raw_policy";
    let manifest = manifest(session_id, "act-forged");
    let decision = fake_decision(&manifest);

    let result = store.append_event(
        session_id,
        JournalEvent::PolicyDecided { decision },
        Utc::now(),
    );

    assert!(
        matches!(result, Err(DaemonError::Refused(message)) if message.contains("admit_action"))
    );
    assert_eq!(store.load_journal(session_id).unwrap().records().len(), 1);
}

#[test]
fn raw_grants_and_proposals_are_refused_by_public_append_event() {
    let (_root, store) = create_store_with_session("raw-authority", "sess_raw_authority");
    let session_id = "sess_raw_authority";

    let raw_session = store.append_event(
        session_id,
        JournalEvent::SessionStatusChanged {
            transition_id: "raw-transition".to_string(),
            session_id: session_id.to_string(),
            from: SessionStatus::Running,
            to: SessionStatus::Paused,
        },
        Utc::now(),
    );
    assert!(
        matches!(raw_session, Err(DaemonError::Refused(message)) if message.contains("transition_session"))
    );

    let raw_grant = store.append_event(
        session_id,
        JournalEvent::CapabilityGranted {
            grant: grant(session_id),
        },
        Utc::now(),
    );
    assert!(
        matches!(raw_grant, Err(DaemonError::Refused(message)) if message.contains("issue_grant"))
    );

    let raw_proposal = store.append_event(
        session_id,
        JournalEvent::ActionProposed {
            manifest: Box::new(manifest(session_id, "act-raw")),
        },
        Utc::now(),
    );
    assert!(
        matches!(raw_proposal, Err(DaemonError::Refused(message)) if message.contains("admit_action"))
    );

    let raw_mandate = store.append_event(
        session_id,
        JournalEvent::PaymentMandateIssued {
            mandate: payment_mandate(session_id),
        },
        Utc::now(),
    );
    assert!(
        matches!(raw_mandate, Err(DaemonError::Refused(message)) if message.contains("issue_payment_mandate"))
    );
    assert_eq!(store.load_journal(session_id).unwrap().records().len(), 1);
}

#[test]
fn tampered_lifecycle_event_cannot_skip_legal_transition() {
    let (root, store) = create_store_with_initial("session-snapshot-tamper", "sess_tamper", []);
    let session_id = "sess_tamper";
    let mut journal = store.load_journal(session_id).unwrap();
    let record = journal
        .append(
            JournalEvent::SessionStatusChanged {
                transition_id: "transition-tampered".to_string(),
                session_id: session_id.to_string(),
                from: SessionStatus::Running,
                to: SessionStatus::Completed,
            },
            Utc::now(),
        )
        .unwrap();
    let journal_path = root
        .path
        .join("sessions")
        .join(session_id)
        .join("journal.jsonl");
    let mut file = OpenOptions::new().append(true).open(journal_path).unwrap();
    writeln!(file, "{}", serde_json::to_string(&record).unwrap()).unwrap();

    let result = store.load_journal(session_id);

    assert!(
        matches!(result, Err(DaemonError::Core(beater_os_core::BeaterOsError::JournalCausality { reason, .. })) if reason.contains("illegal session transition"))
    );
}

#[test]
fn tampered_journal_cannot_project_second_session_created_event() {
    let (root, store) = create_store_with_initial("session-extra-genesis-tamper", "sess_real", []);
    let session_id = "sess_real";
    let mut journal = store.load_journal(session_id).unwrap();
    let other_session = session(&root, "sess_other");
    let record = journal
        .append(
            JournalEvent::SessionCreated {
                session: other_session,
            },
            Utc::now(),
        )
        .unwrap();
    journal.verify_chain().unwrap();
    let journal_path = root
        .path
        .join("sessions")
        .join(session_id)
        .join("journal.jsonl");
    let mut file = OpenOptions::new().append(true).open(journal_path).unwrap();
    writeln!(file, "{}", serde_json::to_string(&record).unwrap()).unwrap();

    let result = store.project(session_id);

    assert!(
        matches!(result, Err(DaemonError::Refused(ref message)) if message.contains("SessionCreated for sess_other")),
        "{result:?}"
    );
}

#[test]
fn undeclared_root_grant_is_refused_without_append() {
    let (_root, store) = create_store_with_initial("grant-undeclared", "sess_undeclared", []);
    let session_id = "sess_undeclared";
    let result = store.issue_grant(session_id, grant(session_id), Utc::now());

    assert!(
        matches!(result, Err(DaemonError::Refused(message)) if message.contains("not declared"))
    );
    assert_eq!(store.load_journal(session_id).unwrap().records().len(), 1);
}

#[test]
fn paused_session_refuses_new_grants_and_action_admission() {
    let (_root, store) = create_store_with_session("session-paused-authority", "sess_paused");
    let session_id = "sess_paused";
    store
        .transition_session(session_id, SessionTransition::Pause, Utc::now())
        .unwrap();

    let grant_result = store.issue_grant(session_id, grant(session_id), Utc::now());
    assert!(
        matches!(grant_result, Err(DaemonError::Refused(message)) if message.contains("not running"))
    );

    let admission_result = store.admit_action(session_id, manifest(session_id, "act-paused"));
    assert!(
        matches!(admission_result, Err(DaemonError::Refused(message)) if message.contains("not running"))
    );
    assert_eq!(
        store.project(session_id).unwrap().session.status,
        SessionStatus::Paused
    );
}

#[test]
fn resumed_session_can_receive_authority_again() {
    let (_root, store) = create_store_with_session("session-resumed-authority", "sess_resumed");
    let session_id = "sess_resumed";
    store
        .transition_session(session_id, SessionTransition::Pause, Utc::now())
        .unwrap();
    store
        .transition_session(session_id, SessionTransition::Resume, Utc::now())
        .unwrap();

    append_grant(&store, session_id, grant(session_id));
    let outcome = store
        .admit_action(session_id, manifest(session_id, "act-resumed"))
        .unwrap();

    assert_eq!(outcome.decision.result, DecisionResult::Allowed);
    assert_eq!(
        store.project(session_id).unwrap().session.status,
        SessionStatus::Running
    );
}

#[test]
fn canceled_session_is_terminal_for_lifecycle_and_admission() {
    let (_root, store) = create_store_with_session("session-canceled-authority", "sess_canceled");
    let session_id = "sess_canceled";
    store
        .transition_session(session_id, SessionTransition::Cancel, Utc::now())
        .unwrap();

    let resume = store.transition_session(session_id, SessionTransition::Resume, Utc::now());
    assert!(
        matches!(resume, Err(DaemonError::Refused(message)) if message.contains("illegal session transition"))
    );
    let admission = store.admit_action(session_id, manifest(session_id, "act-canceled"));
    assert!(
        matches!(admission, Err(DaemonError::Refused(message)) if message.contains("not running"))
    );
    assert_eq!(
        store.project(session_id).unwrap().session.status,
        SessionStatus::Canceled
    );
}

#[test]
fn delegated_grant_cannot_broaden_parent() {
    let (_root, store) = create_store_with_initial(
        "grant-delegated-broaden",
        "sess_delegated",
        ["grant-parent"],
    );
    let session_id = "sess_delegated";
    append_grant(&store, session_id, parent_grant(session_id));
    let mut child = delegated_grant(session_id);
    child.scope.actions.insert(ActionKind::Execute);

    let result = store.issue_grant(session_id, child, Utc::now());

    assert!(
        matches!(result, Err(DaemonError::Refused(message)) if message.contains("broadens parent"))
    );
    assert_eq!(store.project(session_id).unwrap().grants.len(), 1);
}

#[test]
fn missing_path_prefix_grant_is_refused_before_append() {
    let (root, store) = create_store_with_session("grant-missing-prefix", "sess_missing_prefix");
    let session_id = "sess_missing_prefix";
    let mut file_grant = grant(session_id);
    file_grant.scope.selector.resource_id = "*".to_string();
    file_grant
        .constraints
        .path_prefixes
        .insert(root.path.join("missing-prefix").display().to_string());

    let result = store.issue_grant(session_id, file_grant, Utc::now());

    assert!(
        matches!(result, Err(DaemonError::Refused(message)) if message.contains("cannot be canonicalized"))
    );
    assert_eq!(store.project(session_id).unwrap().grants.len(), 0);
}

#[test]
fn proposal_only_recovery_completes_matching_action() {
    let (root, store) = create_store_with_session("admit-recover", "sess_recover");
    let session_id = "sess_recover";
    append_grant(&store, session_id, grant(session_id));
    let manifest = manifest(session_id, "act-recover");
    let manifest_hash = manifest.digest().unwrap();

    let mut journal = store.load_journal(session_id).unwrap();
    let proposal_record = journal
        .append(
            JournalEvent::ActionProposed {
                manifest: Box::new(manifest.clone()),
            },
            Utc::now(),
        )
        .unwrap();
    journal.verify_chain().unwrap();
    let journal_path = root
        .path
        .join("sessions")
        .join(session_id)
        .join("journal.jsonl");
    let mut file = OpenOptions::new().append(true).open(journal_path).unwrap();
    writeln!(file, "{}", serde_json::to_string(&proposal_record).unwrap()).unwrap();

    let outcome = store.admit_action(session_id, manifest).unwrap();

    assert_eq!(outcome.proposal_record.seq, proposal_record.seq);
    assert_eq!(outcome.decision.result, DecisionResult::Allowed);
    assert_eq!(outcome.decision.manifest_hash, manifest_hash);
    let recovered = store.load_journal(session_id).unwrap();
    recovered.verify_chain().unwrap();
    assert_eq!(recovered.records().len(), 4);
    assert!(matches!(
        &recovered.records()[3].event,
        JournalEvent::PolicyDecided { decision }
            if decision.action_id == "act-recover" && decision.result == DecisionResult::Allowed
    ));
}

#[test]
fn proposal_only_recovery_refuses_changed_manifest_without_append() {
    let (root, store) = create_store_with_session("admit-recover-mismatch", "sess_recover_bad");
    let session_id = "sess_recover_bad";
    append_grant(&store, session_id, grant(session_id));
    let original = manifest(session_id, "act-recover-bad");
    let mut changed = original.clone();
    changed.inputs_digest = "different-input-digest".to_string();

    let mut journal = store.load_journal(session_id).unwrap();
    let proposal_record = journal
        .append(
            JournalEvent::ActionProposed {
                manifest: Box::new(original),
            },
            Utc::now(),
        )
        .unwrap();
    journal.verify_chain().unwrap();
    let journal_path = root
        .path
        .join("sessions")
        .join(session_id)
        .join("journal.jsonl");
    let mut file = OpenOptions::new().append(true).open(journal_path).unwrap();
    writeln!(file, "{}", serde_json::to_string(&proposal_record).unwrap()).unwrap();
    let before = store.load_journal(session_id).unwrap().records().len();

    let result = store.admit_action(session_id, changed);

    assert!(
        matches!(result, Err(DaemonError::Refused(message)) if message.contains("different manifest"))
    );
    let after = store.load_journal(session_id).unwrap().records().len();
    assert_eq!(after, before);
}

#[test]
fn concurrent_admissions_serialize_without_forking_the_chain() {
    let root = TempDir::new("admit-concurrent");
    let store = Arc::new(Store::open(&root.path).unwrap());
    let session_id = "sess_concurrent_admit";
    store.create_session(&session(&root, session_id)).unwrap();
    append_grant(&store, session_id, grant(session_id));

    let writers = 12;
    let barrier = Arc::new(Barrier::new(writers));
    let mut handles = Vec::new();
    for index in 0..writers {
        let store = Arc::clone(&store);
        let barrier = Arc::clone(&barrier);
        handles.push(thread::spawn(move || {
            barrier.wait();
            store.admit_action(
                session_id,
                manifest(session_id, &format!("act-concurrent-{index}")),
            )
        }));
    }

    for handle in handles {
        let outcome = handle
            .join()
            .expect("writer thread should not panic")
            .unwrap();
        assert_eq!(outcome.decision.result, DecisionResult::Allowed);
    }

    let journal = store.load_journal(session_id).unwrap();
    let report = journal.verify_chain().unwrap();
    assert_eq!(report.records, 1 + 1 + writers * 2);

    let decisions = journal
        .records()
        .iter()
        .filter(|record| matches!(record.event, JournalEvent::PolicyDecided { .. }))
        .count();
    assert_eq!(decisions, writers);
}
