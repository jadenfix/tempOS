//! Regression tests for the `GrantConstraints` fail-open.
//!
//! A grant serializes to JSON and travels through the journal and storage, so a
//! *partial* `constraints` object must never silently drop the risk/data-class
//! ceilings. These tests pin that: absence keeps the safe caps, and only an
//! explicit `null` opts into an unbounded ceiling.

use std::collections::BTreeSet;

use beater_os_core::{
    ActionKind, ActionManifest, ApprovalRequirement, Budget, CapabilityGrant, CapabilityScope,
    CapabilitySelector, DataClass, DelegationMode, GrantConstraints, ResourceKind, RiskClass,
};
use chrono::{DateTime, Duration, TimeZone, Utc};

fn now() -> DateTime<Utc> {
    Utc.with_ymd_and_hms(2026, 7, 3, 12, 0, 0)
        .single()
        .unwrap_or_else(Utc::now)
}

fn set<T: Ord>(items: impl IntoIterator<Item = T>) -> BTreeSet<T> {
    items.into_iter().collect()
}

fn repo_path() -> CapabilitySelector {
    CapabilitySelector {
        resource_kind: ResourceKind::FilePath,
        resource_id: "/workspace/repo".to_string(),
    }
}

fn grant_with_constraints(constraints: GrantConstraints) -> CapabilityGrant {
    CapabilityGrant {
        grant_id: "grant-1".to_string(),
        issuer: "user:jaden".to_string(),
        holder: "agent:x".to_string(),
        session_id: "s1".to_string(),
        scope: CapabilityScope {
            selector: repo_path(),
            actions: set([ActionKind::Read]),
        },
        denied_actions: BTreeSet::new(),
        constraints,
        expires_at: now() + Duration::hours(1),
        delegation: DelegationMode::None,
        approval: ApprovalRequirement::default(),
        revocation_handle: "revoke:grant-1".to_string(),
        policy_version: "policy-v1".to_string(),
        reason: "read repo".to_string(),
        revoked: false,
    }
}

fn read_manifest(risk: RiskClass, data: BTreeSet<DataClass>) -> ActionManifest {
    ActionManifest {
        action_id: "a1".to_string(),
        session_id: "s1".to_string(),
        tool_id: "tool:fs".to_string(),
        action_kind: ActionKind::Read,
        target: repo_path(),
        resolved_target: Some(repo_path()),
        inputs_digest: "digest".to_string(),
        inputs_summary: "read a file".to_string(),
        expected_outputs: Vec::new(),
        expected_side_effects: BTreeSet::new(),
        required_grants: set(["grant-1".to_string()]),
        requested_budget: Budget::default(),
        risk_class: risk,
        data_classes: data,
        taint: BTreeSet::new(),
        idempotency_key: None,
        compensation_plan: None,
        human_explanation: "read the repo".to_string(),
    }
}

#[test]
fn absent_and_partial_constraints_keep_safe_ceilings() {
    // Whole `constraints` key absent (container default): safe caps.
    let absent: GrantConstraints = serde_json::from_str("{}")
        .unwrap_or_else(|e| panic!("empty constraints should deserialize: {e}"));
    assert_eq!(absent.max_risk, Some(RiskClass::Medium));
    assert_eq!(absent.max_data_class, Some(DataClass::Internal));

    // Present but partial (only path_prefixes set): the ceilings must NOT drop to
    // None. This is the exact fail-open shape.
    let partial: GrantConstraints =
        serde_json::from_str(r#"{"path_prefixes":["/workspace/repo"]}"#)
            .unwrap_or_else(|e| panic!("partial constraints should deserialize: {e}"));
    assert_eq!(partial.max_risk, Some(RiskClass::Medium));
    assert_eq!(partial.max_data_class, Some(DataClass::Internal));
}

#[test]
fn explicit_null_opts_into_unbounded_ceilings() {
    // "No ceiling" must be an explicit, auditable choice, never an omission.
    let c: GrantConstraints = serde_json::from_str(r#"{"max_risk":null,"max_data_class":null}"#)
        .unwrap_or_else(|e| panic!("explicit null should deserialize: {e}"));
    assert_eq!(c.max_risk, None);
    assert_eq!(c.max_data_class, None);
}

#[test]
fn partial_constraints_still_deny_critical_risk() {
    let constraints: GrantConstraints =
        serde_json::from_str(r#"{"path_prefixes":["/workspace/repo"]}"#)
            .unwrap_or_else(|e| panic!("partial constraints should deserialize: {e}"));
    let grant = grant_with_constraints(constraints);
    let manifest = read_manifest(RiskClass::Critical, BTreeSet::new());
    assert!(
        !grant.allows_manifest(&manifest, now(), "agent:x"),
        "a partial-constraints grant must not admit a Critical-risk action"
    );
}

#[test]
fn partial_constraints_still_deny_secret_data() {
    let constraints: GrantConstraints =
        serde_json::from_str(r#"{"path_prefixes":["/workspace/repo"]}"#)
            .unwrap_or_else(|e| panic!("partial constraints should deserialize: {e}"));
    let grant = grant_with_constraints(constraints);
    let manifest = read_manifest(RiskClass::Low, set([DataClass::Secret]));
    assert!(
        !grant.allows_manifest(&manifest, now(), "agent:x"),
        "a partial-constraints grant must not admit Secret data"
    );
}

#[test]
fn in_bounds_action_is_still_admitted() {
    // The fix must not over-block: a Low-risk, Internal-data read within the safe
    // default caps is still allowed.
    let constraints: GrantConstraints = serde_json::from_str("{}")
        .unwrap_or_else(|e| panic!("empty constraints should deserialize: {e}"));
    let grant = grant_with_constraints(constraints);
    let manifest = read_manifest(RiskClass::Low, set([DataClass::Internal]));
    assert!(
        grant.allows_manifest(&manifest, now(), "agent:x"),
        "an in-bounds action should still be admitted under the safe default caps"
    );
}
