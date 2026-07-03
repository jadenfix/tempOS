use chrono::{DateTime, Utc};
use uuid::Uuid;

use crate::contracts::{
    ActionKind, ActionManifest, ApprovalEvidence, ApprovalMode, CapabilityGrant, DecisionResult,
    PolicyDecision, RiskClass, SideEffectClass, SimulationEvidence, TaintLabel,
};

#[derive(Clone, Debug, PartialEq, Eq)]
pub struct AdmissionContext {
    pub now: DateTime<Utc>,
    pub actor_id: String,
    pub session_id: String,
    pub policy_version: String,
    pub grants: Vec<CapabilityGrant>,
    pub approvals: Vec<ApprovalEvidence>,
    pub simulations: Vec<SimulationEvidence>,
}

#[derive(Clone, Debug, Default)]
pub struct PolicyEngine;

impl PolicyEngine {
    pub fn new() -> Self {
        Self
    }

    pub fn admit(&self, manifest: &ActionManifest, ctx: &AdmissionContext) -> PolicyDecision {
        let mut matched_rules = Vec::new();

        if manifest.session_id != ctx.session_id {
            return decision(
                manifest,
                ctx,
                DecisionResult::Denied,
                matched_rules,
                "action manifest session does not match admission context session",
                None,
                None,
            );
        }
        matched_rules.push("manifest_bound_to_context_session".to_string());

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
            .all(|grant| grant.allows_manifest(manifest, ctx.now, &ctx.actor_id));
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
        matched_rules.push("all_required_capabilities_allow_action".to_string());

        if dangerous_untrusted_instruction(manifest)
            && !all_grants_have_action_approval(&matching_grants, manifest, ctx)
        {
            return decision(
                manifest,
                ctx,
                DecisionResult::NeedsApproval,
                matched_rules,
                "untrusted content cannot directly authorize spend, deploy, or delegation actions without action-bound approval",
                Some(format!(
                    "action:{}:untrusted-risk-review",
                    manifest.action_id
                )),
                None,
            );
        }
        matched_rules.push("untrusted_instruction_policy_checked".to_string());

        if matching_grants
            .iter()
            .filter(|grant| {
                grant.approval.mode != ApprovalMode::None
                    && manifest.risk_class >= grant.approval.threshold_risk
            })
            .any(|grant| !has_approval_for_grant(grant, manifest, ctx))
        {
            return decision(
                manifest,
                ctx,
                DecisionResult::NeedsApproval,
                matched_rules,
                "grant policy requires human approval for this risk class",
                Some(format!(
                    "action:{}:grant-threshold-review",
                    manifest.action_id
                )),
                None,
            );
        }
        matched_rules.push("grant_approval_policy_checked".to_string());

        if manifest.risk_class >= RiskClass::High
            && manifest.has_external_side_effect()
            && !has_passed_simulation_for_action(manifest, ctx)
        {
            return decision(
                manifest,
                ctx,
                DecisionResult::NeedsSimulation,
                matched_rules,
                "high-risk external side effects require a passed simulation before execution",
                None,
                Some(format!(
                    "action:{}:high-risk-side-effect-simulation",
                    manifest.action_id
                )),
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

fn dangerous_untrusted_instruction(manifest: &ActionManifest) -> bool {
    manifest.taint.iter().any(|label| {
        matches!(
            label,
            TaintLabel::UntrustedWeb | TaintLabel::UntrustedEmail | TaintLabel::UntrustedDocument
        )
    }) && matches!(
        manifest.action_kind,
        ActionKind::Spend | ActionKind::Deploy | ActionKind::Delegate
    )
}

fn all_grants_have_action_approval(
    grants: &[&CapabilityGrant],
    manifest: &ActionManifest,
    ctx: &AdmissionContext,
) -> bool {
    grants
        .iter()
        .all(|grant| has_approval_for_grant(grant, manifest, ctx))
}

fn has_approval_for_grant(
    grant: &CapabilityGrant,
    manifest: &ActionManifest,
    ctx: &AdmissionContext,
) -> bool {
    ctx.approvals.iter().any(|approval| {
        approval.action_id == manifest.action_id
            && approval.grant_id == grant.grant_id
            && approval.policy_version == ctx.policy_version
            && grant
                .approval
                .reviewer_ids
                .iter()
                .any(|reviewer_id| reviewer_id == &approval.reviewer_id)
    })
}

fn has_passed_simulation_for_action(manifest: &ActionManifest, ctx: &AdmissionContext) -> bool {
    ctx.simulations.iter().any(|simulation| {
        simulation.action_id == manifest.action_id
            && simulation.policy_version == ctx.policy_version
    })
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
