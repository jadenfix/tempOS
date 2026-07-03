use std::collections::BTreeSet;

use beater_os_core::{
    ActionKind, ActionManifest, AdmissionContext, ApprovalMode, ApprovalRequirement, Budget,
    CapabilityGrant, CapabilityReceiptInput, CapabilityScope, CapabilitySelector, DataClass,
    DecisionResult, DelegationMode, GrantConstraints, InMemoryJournal, JournalEvent, PolicyEngine,
    ReceiptLedger, ResourceKind, RiskClass, SideEffectClass, TaintLabel,
};
use chrono::{Duration, TimeZone, Utc};

fn fixed_time() -> chrono::DateTime<Utc> {
    Utc.with_ymd_and_hms(2026, 7, 3, 12, 0, 0)
        .single()
        .unwrap_or_else(Utc::now)
}

fn set<T: Ord>(items: impl IntoIterator<Item = T>) -> BTreeSet<T> {
    items.into_iter().collect()
}

fn grant_for_file(now: chrono::DateTime<Utc>) -> CapabilityGrant {
    CapabilityGrant {
        grant_id: "grant-read-repo".to_string(),
        issuer: "user:jaden".to_string(),
        holder: "agent:beater-os".to_string(),
        session_id: "session-1".to_string(),
        scope: CapabilityScope {
            selector: CapabilitySelector {
                resource_kind: ResourceKind::FilePath,
                resource_id: "/workspace/repo".to_string(),
            },
            actions: set([ActionKind::Read, ActionKind::Write]),
        },
        denied_actions: BTreeSet::new(),
        constraints: GrantConstraints {
            max_risk: Some(RiskClass::Medium),
            max_data_class: Some(DataClass::Internal),
            budget: Budget::default(),
            network_allowlist: BTreeSet::new(),
            path_prefixes: set(["/workspace/repo".to_string()]),
        },
        expires_at: now + Duration::hours(1),
        delegation: DelegationMode::AttenuatedOnly,
        approval: ApprovalRequirement::default(),
        revocation_handle: "revoke:grant-read-repo".to_string(),
        policy_version: "policy-v1".to_string(),
        reason: "read and edit this repo".to_string(),
        revoked: false,
    }
}

fn read_manifest() -> ActionManifest {
    ActionManifest {
        action_id: "action-1".to_string(),
        session_id: "session-1".to_string(),
        tool_id: "tool:repo-reader".to_string(),
        action_kind: ActionKind::Read,
        target: CapabilitySelector {
            resource_kind: ResourceKind::FilePath,
            resource_id: "/workspace/repo".to_string(),
        },
        inputs_digest: "sha256:input".to_string(),
        inputs_summary: "read repo files".to_string(),
        expected_outputs: vec!["file summaries".to_string()],
        expected_side_effects: set([SideEffectClass::None]),
        required_grants: set(["grant-read-repo".to_string()]),
        risk_class: RiskClass::Low,
        data_classes: set([DataClass::Internal]),
        taint: BTreeSet::new(),
        idempotency_key: None,
        compensation_plan: None,
        human_explanation: "Read the scoped repo to plan a change.".to_string(),
    }
}

#[test]
fn policy_allows_action_when_explicit_active_grant_matches() {
    let now = fixed_time();
    let manifest = read_manifest();
    let ctx = AdmissionContext {
        now,
        policy_version: "policy-v1".to_string(),
        grants: vec![grant_for_file(now)],
        approved_review_ids: BTreeSet::new(),
        passed_simulation_ids: BTreeSet::new(),
    };
    let decision = PolicyEngine::new().admit(&manifest, &ctx);
    assert_eq!(decision.result, DecisionResult::Allowed);
    assert!(
        decision
            .matched_rules
            .contains(&"capability_scope_allows_action".to_string())
    );
}

#[test]
fn policy_denies_ambient_authority_when_no_grant_is_named() {
    let now = fixed_time();
    let mut manifest = read_manifest();
    manifest.required_grants.clear();
    let ctx = AdmissionContext {
        now,
        policy_version: "policy-v1".to_string(),
        grants: vec![grant_for_file(now)],
        approved_review_ids: BTreeSet::new(),
        passed_simulation_ids: BTreeSet::new(),
    };
    let decision = PolicyEngine::new().admit(&manifest, &ctx);
    assert_eq!(decision.result, DecisionResult::Denied);
    assert!(decision.explanation.contains("required grant"));
}

#[test]
fn policy_requires_narrowed_grant_for_over_risk_action() {
    let now = fixed_time();
    let mut manifest = read_manifest();
    manifest.risk_class = RiskClass::High;
    let ctx = AdmissionContext {
        now,
        policy_version: "policy-v1".to_string(),
        grants: vec![grant_for_file(now)],
        approved_review_ids: BTreeSet::new(),
        passed_simulation_ids: BTreeSet::new(),
    };
    let decision = PolicyEngine::new().admit(&manifest, &ctx);
    assert_eq!(decision.result, DecisionResult::NeedsNarrowedGrant);
}

#[test]
fn policy_requires_review_for_untrusted_payment_instruction() {
    let now = fixed_time();
    let mut manifest = read_manifest();
    manifest.action_kind = ActionKind::Spend;
    manifest.target = CapabilitySelector {
        resource_kind: ResourceKind::PaymentRail,
        resource_id: "stablecoin:x402".to_string(),
    };
    manifest.expected_side_effects = set([SideEffectClass::Payment]);
    manifest.required_grants = set(["grant-spend".to_string()]);
    manifest.risk_class = RiskClass::Critical;
    manifest.taint = set([TaintLabel::UntrustedWeb]);
    manifest.idempotency_key = Some("pay-once".to_string());
    let mut grant = grant_for_file(now);
    grant.grant_id = "grant-spend".to_string();
    grant.scope.selector.resource_kind = ResourceKind::PaymentRail;
    grant.scope.selector.resource_id = "stablecoin:x402".to_string();
    grant.scope.actions = set([ActionKind::Spend]);
    grant.constraints.max_risk = Some(RiskClass::Critical);
    grant.constraints.max_data_class = Some(DataClass::Financial);
    grant.approval = ApprovalRequirement {
        mode: ApprovalMode::Human,
        threshold_risk: RiskClass::High,
        reviewer_ids: vec!["user:jaden".to_string()],
    };
    let ctx = AdmissionContext {
        now,
        policy_version: "policy-v1".to_string(),
        grants: vec![grant],
        approved_review_ids: BTreeSet::new(),
        passed_simulation_ids: BTreeSet::new(),
    };
    let decision = PolicyEngine::new().admit(&manifest, &ctx);
    assert_eq!(decision.result, DecisionResult::NeedsApproval);
    assert!(decision.explanation.contains("untrusted content"));
}

#[test]
fn policy_requires_simulation_for_high_risk_external_side_effect_after_review() {
    let now = fixed_time();
    let mut manifest = read_manifest();
    manifest.action_kind = ActionKind::Deploy;
    manifest.target = CapabilitySelector {
        resource_kind: ResourceKind::CloudResource,
        resource_id: "staging".to_string(),
    };
    manifest.expected_side_effects = set([SideEffectClass::Deployment]);
    manifest.required_grants = set(["grant-deploy".to_string()]);
    manifest.risk_class = RiskClass::High;
    manifest.idempotency_key = Some("deploy-once".to_string());
    let mut grant = grant_for_file(now);
    grant.grant_id = "grant-deploy".to_string();
    grant.scope.selector.resource_kind = ResourceKind::CloudResource;
    grant.scope.selector.resource_id = "staging".to_string();
    grant.scope.actions = set([ActionKind::Deploy]);
    grant.constraints.max_risk = Some(RiskClass::High);
    grant.approval = ApprovalRequirement {
        mode: ApprovalMode::Human,
        threshold_risk: RiskClass::High,
        reviewer_ids: vec!["user:jaden".to_string()],
    };
    let ctx = AdmissionContext {
        now,
        policy_version: "policy-v1".to_string(),
        grants: vec![grant],
        approved_review_ids: set(["review-1".to_string()]),
        passed_simulation_ids: BTreeSet::new(),
    };
    let decision = PolicyEngine::new().admit(&manifest, &ctx);
    assert_eq!(decision.result, DecisionResult::NeedsSimulation);
}

#[test]
fn journal_detects_event_tampering() -> Result<(), Box<dyn std::error::Error>> {
    let now = fixed_time();
    let manifest = read_manifest();
    let mut journal = InMemoryJournal::new();
    journal.append(
        JournalEvent::ActionProposed {
            manifest: manifest.clone(),
        },
        now,
    )?;
    journal.append(
        JournalEvent::PolicyDecided {
            decision: PolicyEngine::new().admit(
                &manifest,
                &AdmissionContext {
                    now,
                    policy_version: "policy-v1".to_string(),
                    grants: vec![grant_for_file(now)],
                    approved_review_ids: BTreeSet::new(),
                    passed_simulation_ids: BTreeSet::new(),
                },
            ),
        },
        now,
    )?;
    let report = journal.verify_chain()?;
    assert_eq!(report.records, 2);

    let mut records = journal.snapshot().records;
    if let JournalEvent::ActionProposed { manifest } = &mut records[0].event {
        manifest.inputs_summary = "tampered".to_string();
    }
    let tampered = InMemoryJournal::from_records(records);
    assert!(tampered.verify_chain().is_err());
    Ok(())
}

#[test]
fn receipt_ledger_detects_reordered_or_edited_receipts() -> Result<(), Box<dyn std::error::Error>> {
    let now = fixed_time();
    let mut ledger = ReceiptLedger::new();
    ledger.append(CapabilityReceiptInput {
        receipt_id: Some("receipt-1".to_string()),
        action_id: "action-1".to_string(),
        tool_id: "tool:writer".to_string(),
        started_at: now,
        finished_at: now + Duration::milliseconds(10),
        status: "succeeded".to_string(),
        input_digest: "sha256:in".to_string(),
        output_digest: "sha256:out".to_string(),
        side_effect_summary: "wrote a file".to_string(),
        side_effects: vec![SideEffectClass::LocalWrite],
        external_ids: Vec::new(),
        artifact_refs: vec!["diff:1".to_string()],
    })?;
    ledger.append(CapabilityReceiptInput {
        receipt_id: Some("receipt-2".to_string()),
        action_id: "action-2".to_string(),
        tool_id: "tool:test".to_string(),
        started_at: now,
        finished_at: now + Duration::milliseconds(20),
        status: "succeeded".to_string(),
        input_digest: "sha256:in2".to_string(),
        output_digest: "sha256:out2".to_string(),
        side_effect_summary: "ran tests".to_string(),
        side_effects: vec![SideEffectClass::None],
        external_ids: Vec::new(),
        artifact_refs: Vec::new(),
    })?;
    ledger.verify_chain()?;

    let mut receipts = ledger.receipts().to_vec();
    receipts[1].side_effect_summary = "tampered".to_string();
    let tampered = ReceiptLedger::from_receipts(receipts);
    assert!(tampered.verify_chain().is_err());
    Ok(())
}
