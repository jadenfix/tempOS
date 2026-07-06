use std::collections::{BTreeMap, BTreeSet};

use chrono::{DateTime, Utc};
use uuid::Uuid;

use crate::contracts::{
    ActionKind, ActionManifest, ApprovalEvidence, ApprovalMode, CapabilityGrant, DataClass,
    DecisionResult, PaymentMandate, PolicyDecision, RiskClass, SideEffectClass, SimulationEvidence,
    TaintLabel,
};
use crate::error::BeaterOsResult;
use crate::hash::HashValue;

#[derive(Clone, Debug, PartialEq, Eq)]
pub struct AdmissionContext {
    pub now: DateTime<Utc>,
    pub actor_id: String,
    pub session_id: String,
    pub policy_version: String,
    pub grants: Vec<CapabilityGrant>,
    pub approvals: Vec<ApprovalEvidence>,
    pub simulations: Vec<SimulationEvidence>,
    /// Economic-authority objects available to this action (issue #73;
    /// `final.md` §12.7 "no payment without a mandate"). A payment action is
    /// admitted only if one of these mandates covers it. Grants authorize the
    /// *act* of spending; a mandate authorizes the *money*.
    pub mandates: Vec<PaymentMandate>,
    /// Revocation registry: the set of `revocation_handle`s revoked out of band
    /// (issue #10). This is the monotonic revocation epoch — it only grows, so
    /// admission is deterministic under replay. A grant counts as revoked if its
    /// own `revoked` flag is set *or* its handle is in this set, and a grant is
    /// only exercisable while its whole delegation chain is unrevoked.
    pub revoked_handles: BTreeSet<String>,
}

#[derive(Clone, Debug, Default)]
pub struct PolicyEngine;

impl PolicyEngine {
    pub fn new() -> Self {
        Self
    }

    pub fn admit(
        &self,
        manifest: &ActionManifest,
        ctx: &AdmissionContext,
    ) -> BeaterOsResult<PolicyDecision> {
        let mut matched_rules = Vec::new();
        let manifest_hash = manifest.digest()?;

        // §7.4/§12.3/§26: risk can be raised by policy from kernel-derived fields,
        // never lowered by the agent. The agent-asserted `risk_class` may only
        // RAISE the effective risk above this kernel-derived floor.
        let derived_floor = derived_risk_floor(
            &manifest.action_kind,
            &manifest.expected_side_effects,
            &manifest.data_classes,
        );
        let effective_risk = manifest.risk_class.max(derived_floor);
        matched_rules.push(format!(
            "kernel_derived_risk_floor={derived_floor:?};declared_risk={:?};effective_risk={effective_risk:?}",
            manifest.risk_class
        ));

        if manifest.session_id != ctx.session_id {
            return Ok(decision(
                manifest,
                manifest_hash,
                ctx,
                DecisionResult::Denied,
                matched_rules,
                "action manifest session does not match admission context session",
                DecisionFollowup::none(),
            ));
        }
        matched_rules.push("manifest_bound_to_context_session".to_string());

        if manifest.required_grants.is_empty() {
            return Ok(decision(
                manifest,
                manifest_hash,
                ctx,
                DecisionResult::Denied,
                matched_rules,
                "action manifests must name at least one required grant",
                DecisionFollowup::none(),
            ));
        }
        matched_rules.push("required_grants_present".to_string());

        if manifest.has_external_side_effect() && manifest.idempotency_key.is_none() {
            return Ok(decision(
                manifest,
                manifest_hash,
                ctx,
                DecisionResult::Denied,
                matched_rules,
                "external side effects require an idempotency key before execution",
                DecisionFollowup::none(),
            ));
        }
        matched_rules.push("external_side_effect_idempotency".to_string());

        if manifest
            .expected_side_effects
            .contains(&SideEffectClass::Payment)
            && manifest.action_kind != ActionKind::Spend
        {
            return Ok(decision(
                manifest,
                manifest_hash,
                ctx,
                DecisionResult::Denied,
                matched_rules,
                "payment side effects must use the spend action kind",
                DecisionFollowup::none(),
            ));
        }

        if is_payment_action(manifest)
            && let Err(reason) = payment_authorized_by_mandate(manifest, ctx)
        {
            return Ok(decision(
                manifest,
                manifest_hash,
                ctx,
                DecisionResult::Denied,
                matched_rules,
                reason.as_str(),
                DecisionFollowup::none(),
            ));
        }
        if is_payment_action(manifest) {
            matched_rules.push("payment_authorized_by_mandate".to_string());
        }

        let matching_grants: Vec<&CapabilityGrant> = ctx
            .grants
            .iter()
            .filter(|grant| manifest.required_grants.contains(&grant.grant_id))
            .collect();
        if matching_grants.len() != manifest.required_grants.len() {
            return Ok(decision(
                manifest,
                manifest_hash,
                ctx,
                DecisionResult::Denied,
                matched_rules,
                "one or more required grants are missing from the admission context",
                DecisionFollowup::none(),
            ));
        }
        matched_rules.push("required_grants_available".to_string());

        let grants_by_id: BTreeMap<&str, &CapabilityGrant> = ctx
            .grants
            .iter()
            .map(|grant| (grant.grant_id.as_str(), grant))
            .collect();
        if !matching_grants
            .iter()
            .all(|grant| grant_chain_effectively_active(grant, ctx, &grants_by_id))
        {
            return Ok(decision(
                manifest,
                manifest_hash,
                ctx,
                DecisionResult::Denied,
                matched_rules,
                "a required grant or one of its delegation ancestors is revoked, expired, or missing",
                DecisionFollowup::none(),
            ));
        }
        matched_rules.push("grant_delegation_chain_active".to_string());

        let allowed = matching_grants
            .iter()
            .all(|grant| grant.allows_manifest(manifest, effective_risk, ctx.now, &ctx.actor_id));
        if !allowed {
            return Ok(decision(
                manifest,
                manifest_hash,
                ctx,
                DecisionResult::NeedsNarrowedGrant,
                matched_rules,
                "available grants do not allow this action, target, risk, data class, or time window",
                DecisionFollowup::none(),
            ));
        }
        matched_rules.push("all_required_capabilities_allow_action".to_string());

        if dangerous_untrusted_instruction(manifest)
            && !all_grants_have_explicit_action_approval(
                &matching_grants,
                manifest,
                &manifest_hash,
                ctx,
            )
        {
            return Ok(decision(
                manifest,
                manifest_hash,
                ctx,
                DecisionResult::NeedsApproval,
                matched_rules,
                "untrusted content cannot directly authorize spend, deploy, or delegation actions without action-bound approval",
                DecisionFollowup::review(format!(
                    "action:{}:untrusted-risk-review",
                    manifest.action_id
                )),
            ));
        }
        matched_rules.push("untrusted_instruction_policy_checked".to_string());

        if matching_grants
            .iter()
            .filter(|grant| {
                grant.approval.mode != ApprovalMode::None
                    && effective_risk >= grant.approval.threshold_risk
            })
            .any(|grant| !has_approval_for_grant(grant, manifest, &manifest_hash, ctx))
        {
            return Ok(decision(
                manifest,
                manifest_hash,
                ctx,
                DecisionResult::NeedsApproval,
                matched_rules,
                "grant policy requires human approval for this risk class",
                DecisionFollowup::review(format!(
                    "action:{}:grant-threshold-review",
                    manifest.action_id
                )),
            ));
        }
        matched_rules.push("grant_approval_policy_checked".to_string());

        if effective_risk >= RiskClass::High
            && manifest.has_external_side_effect()
            && !has_passed_simulation_for_action(manifest, &manifest_hash, ctx)
        {
            return Ok(decision(
                manifest,
                manifest_hash,
                ctx,
                DecisionResult::NeedsSimulation,
                matched_rules,
                "high-risk external side effects require a passed simulation before execution",
                DecisionFollowup::simulation(format!(
                    "action:{}:high-risk-side-effect-simulation",
                    manifest.action_id
                )),
            ));
        }

        matched_rules.push("admitted_by_capability_policy".to_string());
        Ok(decision(
            manifest,
            manifest_hash,
            ctx,
            DecisionResult::Allowed,
            matched_rules,
            "action admitted by explicit active capability grant",
            DecisionFollowup::none(),
        ))
    }
}

/// Kernel-derived risk floor (final.md §7.4/§12.3/§26).
///
/// Risk class can be raised by policy but never lowered by the agent, and no
/// policy predicate may condition on an agent-asserted field. This function is a
/// pure function of the kernel-derived fields only (`action_kind`,
/// `expected_side_effects`, `data_classes`); it must never read the
/// agent-asserted `risk_class`. The returned floor is the conservative maximum
/// across the action kind and every present side effect and data class.
pub fn derived_risk_floor(
    action_kind: &ActionKind,
    side_effects: &BTreeSet<SideEffectClass>,
    data_classes: &BTreeSet<DataClass>,
) -> RiskClass {
    let mut floor = RiskClass::Low;

    if matches!(
        action_kind,
        ActionKind::Spend | ActionKind::Deploy | ActionKind::Delegate
    ) {
        floor = floor.max(RiskClass::High);
    }

    for effect in side_effects {
        let contribution = match effect {
            SideEffectClass::Payment
            | SideEffectClass::CloudMutation
            | SideEffectClass::Deployment
            | SideEffectClass::Delegation => RiskClass::High,
            SideEffectClass::NetworkWrite
            | SideEffectClass::BrowserSubmit
            | SideEffectClass::HumanCommunication => RiskClass::Medium,
            // Benign side effects must not be over-gated.
            SideEffectClass::None | SideEffectClass::LocalWrite | SideEffectClass::MemoryWrite => {
                RiskClass::Low
            }
        };
        floor = floor.max(contribution);
    }

    for class in data_classes {
        let contribution = match class {
            DataClass::Secret | DataClass::Financial => RiskClass::High,
            DataClass::Customer | DataClass::Personal => RiskClass::Medium,
            _ => RiskClass::Low,
        };
        floor = floor.max(contribution);
    }

    floor
}

/// An action moves money if it declares a payment side effect or uses the spend
/// verb. Both are treated as payments so a spend cannot dodge mandate review by
/// omitting the `Payment` side-effect label (the same anti-laundering stance as
/// issues #46 and #8).
fn is_payment_action(manifest: &ActionManifest) -> bool {
    manifest
        .expected_side_effects
        .contains(&SideEffectClass::Payment)
        || manifest.action_kind == ActionKind::Spend
}

/// Enforce `final.md` §12.7 "no payment without a mandate" (issue #73). Grants
/// authorize the act of spending; a `PaymentMandate` authorizes the money. A
/// payment is admitted only if a mandate, bound to this session and holder and
/// still active, covers the declared amount. The amount must be declared — a
/// payment that does not state how much it moves cannot be bounded, so it fails
/// closed ("no silent mandate expansion").
///
/// Payment intent is chain-neutral. Aether, Stripe, x402, cards, and fake rails
/// all project into the same normalized fields, while each adapter owns envelope
/// parsing and signature/settlement verification outside the policy TCB.
fn payment_authorized_by_mandate(
    manifest: &ActionManifest,
    ctx: &AdmissionContext,
) -> Result<(), String> {
    let Some(amount) = manifest.requested_budget.max_payment_minor_units else {
        return Err(
            "payment action must declare its amount in requested_budget.max_payment_minor_units"
                .to_string(),
        );
    };

    let Some(intent) = &manifest.payment_intent else {
        return Err("payment actions require a payment_intent".to_string());
    };

    if manifest.target.resource_kind != crate::contracts::ResourceKind::PaymentRail {
        return Err("payment intent target must be a payment_rail".to_string());
    }
    if manifest.target.resource_id != intent.rail {
        return Err("payment intent rail must match the manifest payment_rail target".to_string());
    }
    if intent.amount_minor_units == 0 {
        return Err("payment intent amount must be non-zero".to_string());
    }
    if intent.amount_minor_units != amount {
        return Err(
            "payment intent amount must match requested_budget.max_payment_minor_units".to_string(),
        );
    }
    if manifest.idempotency_key.as_deref() != Some(intent.payment_idempotency_key.as_str()) {
        return Err(
            "payment intent idempotency key must match the manifest idempotency key".to_string(),
        );
    }
    if intent.mandate_id.is_empty()
        || intent.rail.is_empty()
        || intent.adapter_id.is_empty()
        || intent.asset.is_empty()
        || intent.counterparty_ref.is_empty()
        || intent.purpose.is_empty()
        || intent.payment_idempotency_key.is_empty()
        || intent.envelope_format.is_empty()
    {
        return Err("payment intent fields must be non-empty".to_string());
    }
    if !is_hex_64(&intent.counterparty_binding_hash) || !is_hex_64(&intent.envelope_hash) {
        return Err("payment intent hashes must be lowercase 32-byte hex".to_string());
    }
    if intent
        .envelope_expires_at
        .as_ref()
        .is_some_and(|expires_at| expires_at <= &ctx.now)
    {
        return Err("payment intent envelope is expired".to_string());
    }

    let mut matching = ctx.mandates.iter().filter(|mandate| {
        mandate.mandate_id == intent.mandate_id
            && mandate.session_id == ctx.session_id
            && mandate.holder == ctx.actor_id
    });
    let Some(mandate) = matching.next() else {
        return Err(
            "payment requires an active PaymentMandate covering the amount for this session and holder"
                .to_string(),
        );
    };
    if matching.next().is_some() {
        return Err("payment intent mandate_id must select exactly one mandate".to_string());
    }

    if mandate.expires_at <= ctx.now {
        return Err("payment mandate is expired".to_string());
    }
    if intent.rail != mandate.rail {
        return Err("payment intent rail does not match mandate rail".to_string());
    }
    if intent.asset != mandate.asset {
        return Err("payment intent asset does not match mandate asset".to_string());
    }
    if intent.amount_minor_units > mandate.max_minor_units {
        return Err("payment intent amount exceeds mandate ceiling".to_string());
    }
    if intent.purpose != mandate.purpose {
        return Err("payment intent purpose does not match mandate purpose".to_string());
    }
    if intent.payment_idempotency_key != mandate.idempotency_key {
        return Err("payment intent idempotency key does not match mandate".to_string());
    }
    if !mandate.allowed_adapter_ids.is_empty()
        && !mandate.allowed_adapter_ids.contains(&intent.adapter_id)
    {
        return Err("payment intent adapter is not allowed by mandate".to_string());
    }
    if !mandate.allowed_envelope_formats.is_empty()
        && !mandate
            .allowed_envelope_formats
            .contains(&intent.envelope_format)
    {
        return Err("payment intent envelope format is not allowed by mandate".to_string());
    }

    Ok(())
}

fn is_hex_64(value: &str) -> bool {
    value.len() == 64
        && value
            .bytes()
            .all(|byte| byte.is_ascii_digit() || (b'a'..=b'f').contains(&byte))
}

/// Walk a grant's delegation chain and return whether the whole chain is live
/// (issue #10). A delegated grant is authority indirected through its parent, so
/// it is exercisable only while it and every ancestor are unexpired and
/// unrevoked — by the grant's own `revoked` flag or by the revocation registry
/// (`ctx.revoked_handles`). Fails closed on a missing named ancestor (the parent
/// was dropped, so its liveness is unknown) and on a cycle (bounded by a visited
/// set), so a malformed chain can never admit an action.
fn grant_chain_effectively_active(
    grant: &CapabilityGrant,
    ctx: &AdmissionContext,
    grants_by_id: &BTreeMap<&str, &CapabilityGrant>,
) -> bool {
    let mut current = grant;
    let mut visited: BTreeSet<&str> = BTreeSet::new();
    loop {
        if !visited.insert(current.grant_id.as_str()) {
            return false;
        }
        let registry_revoked = ctx.revoked_handles.contains(&current.revocation_handle);
        if !current.is_active_at(ctx.now) || registry_revoked {
            return false;
        }
        let Some(parent_id) = &current.parent_grant_id else {
            return true;
        };
        let Some(parent) = grants_by_id.get(parent_id.as_str()) else {
            return false;
        };
        current = parent;
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

fn all_grants_have_explicit_action_approval(
    grants: &[&CapabilityGrant],
    manifest: &ActionManifest,
    manifest_hash: &HashValue,
    ctx: &AdmissionContext,
) -> bool {
    grants.iter().all(|grant| match grant.approval.mode {
        ApprovalMode::None => false,
        ApprovalMode::Human | ApprovalMode::MultiParty => {
            has_approval_for_grant(grant, manifest, manifest_hash, ctx)
        }
    })
}

fn has_approval_for_grant(
    grant: &CapabilityGrant,
    manifest: &ActionManifest,
    manifest_hash: &HashValue,
    ctx: &AdmissionContext,
) -> bool {
    match grant.approval.mode {
        ApprovalMode::None => true,
        ApprovalMode::Human => grant.approval.reviewer_ids.iter().any(|reviewer_id| {
            has_approval_from_reviewer(grant, manifest, manifest_hash, ctx, reviewer_id)
        }),
        ApprovalMode::MultiParty => {
            !grant.approval.reviewer_ids.is_empty()
                && grant.approval.reviewer_ids.iter().all(|reviewer_id| {
                    has_approval_from_reviewer(grant, manifest, manifest_hash, ctx, reviewer_id)
                })
        }
    }
}

fn has_approval_from_reviewer(
    grant: &CapabilityGrant,
    manifest: &ActionManifest,
    manifest_hash: &HashValue,
    ctx: &AdmissionContext,
    reviewer_id: &str,
) -> bool {
    ctx.approvals.iter().any(|approval| {
        approval.approved_at <= ctx.now
            && approval.action_id == manifest.action_id
            && approval.manifest_hash == *manifest_hash
            && approval.grant_id == grant.grant_id
            && approval.policy_version == ctx.policy_version
            && approval.reviewer_id == reviewer_id
    })
}

fn has_passed_simulation_for_action(
    manifest: &ActionManifest,
    manifest_hash: &HashValue,
    ctx: &AdmissionContext,
) -> bool {
    ctx.simulations.iter().any(|simulation| {
        simulation.passed_at <= ctx.now
            && simulation.action_id == manifest.action_id
            && simulation.manifest_hash == *manifest_hash
            && simulation.policy_version == ctx.policy_version
    })
}

fn decision(
    manifest: &ActionManifest,
    manifest_hash: HashValue,
    ctx: &AdmissionContext,
    result: DecisionResult,
    matched_rules: Vec<String>,
    explanation: &str,
    followup: DecisionFollowup,
) -> PolicyDecision {
    PolicyDecision {
        decision_id: Uuid::new_v4().to_string(),
        action_id: manifest.action_id.clone(),
        manifest_hash,
        policy_version: ctx.policy_version.clone(),
        result,
        matched_rules,
        explanation: explanation.to_string(),
        required_review: followup.required_review,
        required_simulation: followup.required_simulation,
        created_at: ctx.now,
    }
}

#[derive(Clone, Debug, Default)]
struct DecisionFollowup {
    required_review: Option<String>,
    required_simulation: Option<String>,
}

impl DecisionFollowup {
    fn none() -> Self {
        Self::default()
    }

    fn review(review: String) -> Self {
        Self {
            required_review: Some(review),
            required_simulation: None,
        }
    }

    fn simulation(simulation: String) -> Self {
        Self {
            required_review: None,
            required_simulation: Some(simulation),
        }
    }
}
