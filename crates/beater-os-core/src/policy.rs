use std::collections::BTreeSet;

use chrono::{DateTime, Utc};
use uuid::Uuid;

use crate::contracts::{
    ActionKind, ActionManifest, ApprovalMode, CapabilityGrant, DecisionResult, PolicyDecision,
    RiskClass, SideEffectClass, TaintLabel,
};

#[derive(Clone, Debug, PartialEq, Eq)]
pub struct AdmissionContext {
    pub now: DateTime<Utc>,
    pub policy_version: String,
    pub grants: Vec<CapabilityGrant>,
    pub approved_review_ids: BTreeSet<String>,
    pub passed_simulation_ids: BTreeSet<String>,
}

#[derive(Clone, Debug, Default)]
pub struct PolicyEngine;

impl PolicyEngine {
    pub fn new() -> Self {
        Self
    }

    pub fn admit(&self, manifest: &ActionManifest, ctx: &AdmissionContext) -> PolicyDecision {
        let mut matched_rules = Vec::new();

        if manifest.required_grants.is_empty() {
            return decision(
                manifest,
                ctx,
                DecisionResult::Denied,
                matched_rules,
                "action manifests must name at least one required grant",
                None,
                None,
            );
        }
        matched_rules.push("required_grants_present".to_string());

        if manifest.taint.iter().any(|label| {
            matches!(
                label,
                TaintLabel::UntrustedWeb
                    | TaintLabel::UntrustedEmail
                    | TaintLabel::UntrustedDocument
            )
        }) && matches!(
            manifest.action_kind,
            ActionKind::Spend | ActionKind::Deploy | ActionKind::Delegate
        ) {
            return decision(
                manifest,
                ctx,
                DecisionResult::NeedsApproval,
                matched_rules,
                "untrusted content cannot directly authorize spend, deploy, or delegation actions",
                Some("trusted-human-review".to_string()),
                None,
            );
        }

        if manifest.has_external_side_effect() && manifest.idempotency_key.is_none() {
            return decision(
                manifest,
                ctx,
                DecisionResult::Denied,
                matched_rules,
                "external side effects require an idempotency key before execution",
                None,
                None,
            );
        }
        matched_rules.push("external_side_effect_idempotency".to_string());

        if manifest
            .expected_side_effects
            .contains(&SideEffectClass::Payment)
            && manifest.action_kind != ActionKind::Spend
        {
            return decision(
                manifest,
                ctx,
                DecisionResult::Denied,
                matched_rules,
                "payment side effects must use the spend action kind",
                None,
                None,
            );
        }

        let matching_grants: Vec<&CapabilityGrant> = ctx
            .grants
            .iter()
            .filter(|grant| manifest.required_grants.contains(&grant.grant_id))
            .collect();
        if matching_grants.len() != manifest.required_grants.len() {
            return decision(
                manifest,
                ctx,
                DecisionResult::Denied,
                matched_rules,
                "one or more required grants are missing from the admission context",
                None,
                None,
            );
        }
        matched_rules.push("required_grants_available".to_string());

        let allowed = matching_grants
            .iter()
            .any(|grant| grant.allows_manifest(manifest, ctx.now));
        if !allowed {
            return decision(
                manifest,
                ctx,
                DecisionResult::NeedsNarrowedGrant,
                matched_rules,
                "available grants do not allow this action, target, risk, data class, or time window",
                None,
                None,
            );
        }
        matched_rules.push("capability_scope_allows_action".to_string());

        if matching_grants.iter().any(|grant| {
            grant.approval.mode != ApprovalMode::None
                && manifest.risk_class >= grant.approval.threshold_risk
        }) && ctx.approved_review_ids.is_empty()
        {
            return decision(
                manifest,
                ctx,
                DecisionResult::NeedsApproval,
                matched_rules,
                "grant policy requires human approval for this risk class",
                Some("grant-threshold-review".to_string()),
                None,
            );
        }

        if manifest.risk_class >= RiskClass::High
            && manifest.has_external_side_effect()
            && ctx.passed_simulation_ids.is_empty()
        {
            return decision(
                manifest,
                ctx,
                DecisionResult::NeedsSimulation,
                matched_rules,
                "high-risk external side effects require a passed simulation before execution",
                None,
                Some("high-risk-side-effect-simulation".to_string()),
            );
        }

        matched_rules.push("admitted_by_capability_policy".to_string());
        decision(
            manifest,
            ctx,
            DecisionResult::Allowed,
            matched_rules,
            "action admitted by explicit active capability grant",
            None,
            None,
        )
    }
}

fn decision(
    manifest: &ActionManifest,
    ctx: &AdmissionContext,
    result: DecisionResult,
    matched_rules: Vec<String>,
    explanation: &str,
    required_review: Option<String>,
    required_simulation: Option<String>,
) -> PolicyDecision {
    PolicyDecision {
        decision_id: Uuid::new_v4().to_string(),
        action_id: manifest.action_id.clone(),
        policy_version: ctx.policy_version.clone(),
        result,
        matched_rules,
        explanation: explanation.to_string(),
        required_review,
        required_simulation,
        created_at: ctx.now,
    }
}
