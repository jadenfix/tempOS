//! Typed agent runtime loop over the daemon store.
//!
//! This crate is deliberately not a model adapter and not an execution sandbox.
//! It is the runtime orchestration layer that turns agent-loop intents into the
//! daemon-owned authority sequence:
//!
//! 1. create a durable session,
//! 2. issue bounded grants,
//! 3. propose an action manifest through `beater-osd`,
//! 4. stop unless policy returns `Allowed`,
//! 5. optionally append a no-side-effect observation receipt.
//!
//! Real tool execution still belongs behind `beater-osd`/gateway/sandbox
//! mediation. This crate exists so the CLI, daemon HTTP surface, and future
//! agent workers can share one small runtime contract instead of rebuilding
//! admission logic.

use std::collections::BTreeSet;
use std::path::PathBuf;

use beater_os_core::{
    ActionKind, ActionManifest, AgentSession, BeaterOsError, Budget, CapabilityGrant,
    CapabilityReceipt, CapabilityReceiptInput, CapabilityScope, CapabilitySelector, DataClass,
    DecisionResult, DelegationMode, GrantConstraints, HashValue, ModelPolicy, ResourceKind,
    RiskClass, SessionStatus, SideEffectClass, TaintLabel, hash_json,
};
use beater_osd::{AdmissionOutcome, DAEMON_POLICY_VERSION, DaemonError, SessionProjection, Store};
use chrono::{DateTime, TimeDelta, Utc};
use thiserror::Error;
use uuid::Uuid;

const DEFAULT_GRANT_TTL_SECS: u64 = 3600;
const DEFAULT_RUNTIME_TOOL_ID: &str = "tool:beater-os-runtime";

pub type RuntimeResult<T> = Result<T, RuntimeError>;

#[derive(Debug, Error)]
pub enum RuntimeError {
    #[error(transparent)]
    Daemon(#[from] DaemonError),
    #[error(transparent)]
    Core(#[from] BeaterOsError),
    #[error("runtime refused request: {0}")]
    Refused(String),
    #[error("invalid ttl seconds: {0}")]
    InvalidTtl(u64),
}

/// Runtime facade around the daemon store.
#[derive(Clone, Debug)]
pub struct AgentRuntime {
    store: Store,
}

impl AgentRuntime {
    pub fn open(root: impl Into<PathBuf>) -> RuntimeResult<Self> {
        Ok(Self {
            store: Store::open(root)?,
        })
    }

    pub fn from_store(store: Store) -> Self {
        Self { store }
    }

    pub fn store(&self) -> &Store {
        &self.store
    }

    pub fn create_session(&self, start: SessionStart) -> RuntimeResult<AgentSession> {
        let now = Utc::now();
        let session_id = start
            .session_id
            .unwrap_or_else(|| Uuid::new_v4().to_string());
        let created_by = start.created_by.unwrap_or_else(|| start.agent_id.clone());
        let initial_capability_ids = if start.initial_capability_ids.is_empty() {
            BTreeSet::from([default_root_grant_id(&session_id)])
        } else {
            start.initial_capability_ids
        };
        let session = AgentSession {
            session_id,
            created_at: now,
            created_by,
            agent_id: start.agent_id,
            workspace_id: start.workspace_id,
            goal: start.goal,
            constraints: start.constraints,
            policy_profile: start.policy_profile,
            initial_capability_ids,
            budget: start.budget,
            model_policy: start.model_policy,
            memory_scope: start.memory_scope,
            journal_root: self.store.root().display().to_string(),
            status: SessionStatus::Running,
        };
        self.store.create_session(&session)?;
        Ok(session)
    }

    pub fn issue_grant(
        &self,
        session_id: &str,
        request: GrantRequest,
    ) -> RuntimeResult<CapabilityGrant> {
        if request.actions.is_empty() {
            return Err(RuntimeError::Refused(
                "grant must allow at least one action".to_string(),
            ));
        }
        let now = Utc::now();
        let projection = self.store.project(session_id)?;
        let expires_at = expires_at(now, request.expires_in_secs)?;
        let parent_grant_id = request.parent_grant_id;
        let grant_id = request.grant_id.unwrap_or_else(|| {
            if parent_grant_id.is_some() {
                Uuid::new_v4().to_string()
            } else {
                default_root_grant_id(session_id)
            }
        });
        let grant = CapabilityGrant {
            grant_id,
            issuer: request
                .issuer
                .unwrap_or_else(|| projection.session.created_by.clone()),
            holder: request
                .holder
                .unwrap_or_else(|| projection.session.agent_id.clone()),
            session_id: session_id.to_string(),
            parent_grant_id,
            scope: CapabilityScope {
                selector: CapabilitySelector {
                    resource_kind: request.resource_kind,
                    resource_id: request.resource_id,
                },
                actions: request.actions,
            },
            denied_actions: request.denied_actions,
            constraints: request.constraints,
            expires_at,
            delegation: request.delegation,
            approval: request.approval,
            revocation_handle: request
                .revocation_handle
                .unwrap_or_else(|| Uuid::new_v4().to_string()),
            policy_version: DAEMON_POLICY_VERSION.to_string(),
            reason: request
                .reason
                .unwrap_or_else(|| "issued via beater-os-runtime".to_string()),
            revoked: false,
        };
        self.store.issue_grant(session_id, grant.clone(), now)?;
        Ok(grant)
    }

    /// Admit steps sequentially and stop after the first non-allowed decision.
    pub fn run_steps(
        &self,
        steps: impl IntoIterator<Item = RuntimeStep>,
    ) -> RuntimeResult<Vec<RuntimeStepOutcome>> {
        let mut outcomes = Vec::new();
        for step in steps {
            let outcome = self.admit_step(step)?;
            let allowed = outcome.admission.decision.result == DecisionResult::Allowed;
            outcomes.push(outcome);
            if !allowed {
                break;
            }
        }
        Ok(outcomes)
    }

    /// Propose one runtime step through the daemon-owned admission path.
    pub fn admit_step(&self, step: RuntimeStep) -> RuntimeResult<RuntimeStepOutcome> {
        let observation = step.observation.clone();
        let external_revoked_handles = step.external_revoked_handles.clone();
        let manifest = step.into_manifest()?;
        if observation.is_some() && !allows_observation_receipt(&manifest) {
            return Err(RuntimeError::Refused(
                "runtime observation receipts may only claim no side effects".to_string(),
            ));
        }
        let session_id = manifest.session_id.clone();
        let admission = self.store.admit_action_with_revoked_handles(
            &session_id,
            manifest.clone(),
            external_revoked_handles,
        )?;
        let receipt_outcome = if admission.decision.result == DecisionResult::Allowed {
            observation
                .map(|observation| self.append_observation_receipt(&manifest, observation))
                .transpose()?
        } else {
            None
        };
        let receipt_root_hash = match &receipt_outcome {
            Some(outcome) => outcome.receipt.receipt_hash.clone(),
            None => admission.receipt_root_hash.clone(),
        };
        let evidence = StepReplayEvidence::from_parts(
            &manifest,
            &admission,
            receipt_outcome.as_ref(),
            receipt_root_hash,
        );
        let receipt = receipt_outcome.map(|outcome| outcome.receipt);
        let projection = self.store.project(&session_id)?;
        Ok(RuntimeStepOutcome {
            admission,
            receipt,
            evidence,
            projection,
        })
    }

    fn append_observation_receipt(
        &self,
        manifest: &ActionManifest,
        observation: RuntimeObservation,
    ) -> RuntimeResult<beater_osd::ReceiptAppendOutcome> {
        let started_at = observation.started_at.unwrap_or_else(Utc::now);
        let finished_at = observation.finished_at.unwrap_or_else(Utc::now);
        let output_digest = hash_json(&observation.output_summary)?;
        let receipt = self.store.append_receipt_with_record(
            &manifest.session_id,
            CapabilityReceiptInput {
                receipt_id: observation.receipt_id,
                action_id: manifest.action_id.clone(),
                tool_id: manifest.tool_id.clone(),
                target: manifest
                    .resolved_target
                    .as_ref()
                    .unwrap_or(&manifest.target)
                    .clone(),
                started_at,
                finished_at,
                status: observation.status,
                input_digest: manifest.inputs_digest.clone(),
                output_digest,
                side_effect_summary: observation.side_effect_summary,
                side_effects: Vec::new(),
                external_ids: observation.external_ids,
                artifact_refs: observation.artifact_refs,
            },
            finished_at,
        )?;
        Ok(receipt)
    }
}

/// Start state for a durable agent session.
#[derive(Clone, Debug)]
pub struct SessionStart {
    pub session_id: Option<String>,
    pub created_by: Option<String>,
    pub agent_id: String,
    pub workspace_id: String,
    pub goal: String,
    pub constraints: Vec<String>,
    pub policy_profile: String,
    pub initial_capability_ids: BTreeSet<String>,
    pub budget: Budget,
    pub model_policy: ModelPolicy,
    pub memory_scope: Option<String>,
}

impl SessionStart {
    pub fn new(
        agent_id: impl Into<String>,
        workspace_id: impl Into<String>,
        goal: impl Into<String>,
    ) -> Self {
        Self {
            session_id: None,
            created_by: None,
            agent_id: agent_id.into(),
            workspace_id: workspace_id.into(),
            goal: goal.into(),
            constraints: Vec::new(),
            policy_profile: "default".to_string(),
            initial_capability_ids: BTreeSet::new(),
            budget: Budget::default(),
            model_policy: ModelPolicy::default(),
            memory_scope: None,
        }
    }
}

/// Bounded authority request issued through the daemon store.
#[derive(Clone, Debug)]
pub struct GrantRequest {
    pub grant_id: Option<String>,
    pub issuer: Option<String>,
    pub holder: Option<String>,
    pub parent_grant_id: Option<String>,
    pub resource_kind: ResourceKind,
    pub resource_id: String,
    pub actions: BTreeSet<ActionKind>,
    pub denied_actions: BTreeSet<ActionKind>,
    pub constraints: GrantConstraints,
    pub expires_in_secs: u64,
    pub delegation: DelegationMode,
    pub approval: beater_os_core::ApprovalRequirement,
    pub revocation_handle: Option<String>,
    pub reason: Option<String>,
}

impl GrantRequest {
    pub fn new(
        resource_kind: ResourceKind,
        resource_id: impl Into<String>,
        actions: impl IntoIterator<Item = ActionKind>,
    ) -> Self {
        Self {
            grant_id: None,
            issuer: None,
            holder: None,
            parent_grant_id: None,
            resource_kind,
            resource_id: resource_id.into(),
            actions: actions.into_iter().collect(),
            denied_actions: BTreeSet::new(),
            constraints: GrantConstraints::default(),
            expires_in_secs: DEFAULT_GRANT_TTL_SECS,
            delegation: DelegationMode::None,
            approval: Default::default(),
            revocation_handle: None,
            reason: None,
        }
    }
}

/// One model-proposed runtime step.
#[derive(Clone, Debug)]
pub struct RuntimeStep {
    pub session_id: String,
    pub action_id: Option<String>,
    pub tool_id: Option<String>,
    pub action_kind: ActionKind,
    pub target: CapabilitySelector,
    pub resolved_target: Option<CapabilitySelector>,
    pub inputs_summary: String,
    pub inputs_digest: Option<String>,
    pub expected_outputs: Vec<String>,
    pub expected_side_effects: BTreeSet<SideEffectClass>,
    pub required_grants: BTreeSet<String>,
    pub requested_budget: Budget,
    pub risk_class: RiskClass,
    pub data_classes: BTreeSet<DataClass>,
    pub taint: BTreeSet<TaintLabel>,
    pub idempotency_key: Option<String>,
    pub compensation_plan: Option<String>,
    pub human_explanation: String,
    pub external_revoked_handles: BTreeSet<String>,
    pub observation: Option<RuntimeObservation>,
}

impl RuntimeStep {
    pub fn new(
        session_id: impl Into<String>,
        action_kind: ActionKind,
        target: CapabilitySelector,
        inputs_summary: impl Into<String>,
    ) -> Self {
        Self {
            session_id: session_id.into(),
            action_id: None,
            tool_id: None,
            action_kind,
            target,
            resolved_target: None,
            inputs_summary: inputs_summary.into(),
            inputs_digest: None,
            expected_outputs: Vec::new(),
            expected_side_effects: BTreeSet::new(),
            required_grants: BTreeSet::new(),
            requested_budget: Budget::default(),
            risk_class: RiskClass::Low,
            data_classes: BTreeSet::new(),
            taint: BTreeSet::new(),
            idempotency_key: None,
            compensation_plan: None,
            human_explanation: "proposed via beater-os-runtime".to_string(),
            external_revoked_handles: BTreeSet::new(),
            observation: None,
        }
    }

    fn into_manifest(self) -> RuntimeResult<ActionManifest> {
        let inputs_digest = match self.inputs_digest {
            Some(digest) => digest,
            None => hash_json(&self.inputs_summary)?,
        };
        Ok(ActionManifest {
            action_id: self.action_id.unwrap_or_else(|| Uuid::new_v4().to_string()),
            session_id: self.session_id,
            tool_id: self
                .tool_id
                .unwrap_or_else(|| DEFAULT_RUNTIME_TOOL_ID.to_string()),
            action_kind: self.action_kind,
            target: self.target,
            resolved_target: self.resolved_target,
            inputs_digest,
            inputs_summary: self.inputs_summary,
            expected_outputs: self.expected_outputs,
            expected_side_effects: self.expected_side_effects,
            required_grants: self.required_grants,
            requested_budget: self.requested_budget,
            risk_class: self.risk_class,
            data_classes: self.data_classes,
            taint: self.taint,
            idempotency_key: self.idempotency_key,
            payment_intent: None,
            compensation_plan: self.compensation_plan,
            human_explanation: self.human_explanation,
        })
    }
}

/// Observation receipt for a runtime-internal step.
///
/// This is intentionally limited to no-side-effect observations. Tool/process
/// side effects must be executed by the daemon gateway and receipt path, not
/// attested by this crate.
#[derive(Clone, Debug)]
pub struct RuntimeObservation {
    pub receipt_id: Option<String>,
    pub started_at: Option<DateTime<Utc>>,
    pub finished_at: Option<DateTime<Utc>>,
    pub status: String,
    pub output_summary: String,
    pub side_effect_summary: String,
    pub external_ids: Vec<String>,
    pub artifact_refs: Vec<String>,
}

impl RuntimeObservation {
    pub fn ok(output_summary: impl Into<String>) -> Self {
        Self {
            receipt_id: None,
            started_at: None,
            finished_at: None,
            status: "ok".to_string(),
            output_summary: output_summary.into(),
            side_effect_summary: "no side effects".to_string(),
            external_ids: Vec::new(),
            artifact_refs: Vec::new(),
        }
    }
}

#[derive(Clone, Debug)]
pub struct RuntimeStepOutcome {
    pub admission: AdmissionOutcome,
    pub receipt: Option<CapabilityReceipt>,
    pub evidence: StepReplayEvidence,
    pub projection: SessionProjection,
}

/// Deterministic replay anchor for one runtime step.
///
/// `PolicyDecision::decision_id` is intentionally nonce-like, so replay should
/// anchor on the manifest hash plus append-only journal/receipt chain roots.
#[derive(Clone, Debug, PartialEq, Eq)]
pub struct StepReplayEvidence {
    pub session_id: String,
    pub action_id: String,
    pub manifest_hash: HashValue,
    pub policy_version: String,
    pub proposal_seq: u64,
    pub proposal_hash: HashValue,
    pub decision_seq: u64,
    pub decision_hash: HashValue,
    pub admission_journal_root_hash: HashValue,
    pub receipt_journal_seq: Option<u64>,
    pub receipt_journal_hash: Option<HashValue>,
    pub receipt_seq: Option<u64>,
    pub receipt_hash: Option<HashValue>,
    pub receipt_root_hash: HashValue,
    pub final_journal_root_hash: HashValue,
}

impl StepReplayEvidence {
    fn from_parts(
        manifest: &ActionManifest,
        admission: &AdmissionOutcome,
        receipt_outcome: Option<&beater_osd::ReceiptAppendOutcome>,
        receipt_root_hash: HashValue,
    ) -> Self {
        let admission_journal_root_hash = admission.decision_record.hash.clone();
        let final_journal_root_hash = receipt_outcome
            .map(|outcome| outcome.receipt_record.hash.clone())
            .unwrap_or_else(|| admission_journal_root_hash.clone());
        Self {
            session_id: manifest.session_id.clone(),
            action_id: manifest.action_id.clone(),
            manifest_hash: admission.decision.manifest_hash.clone(),
            policy_version: admission.decision.policy_version.clone(),
            proposal_seq: admission.proposal_record.seq,
            proposal_hash: admission.proposal_record.hash.clone(),
            decision_seq: admission.decision_record.seq,
            decision_hash: admission.decision_record.hash.clone(),
            admission_journal_root_hash,
            receipt_journal_seq: receipt_outcome.map(|outcome| outcome.receipt_record.seq),
            receipt_journal_hash: receipt_outcome
                .map(|outcome| outcome.receipt_record.hash.clone()),
            receipt_seq: receipt_outcome.map(|outcome| outcome.receipt.seq),
            receipt_hash: receipt_outcome.map(|outcome| outcome.receipt.receipt_hash.clone()),
            receipt_root_hash,
            final_journal_root_hash,
        }
    }
}

pub fn default_root_grant_id(session_id: &str) -> String {
    format!("{session_id}-root-grant")
}

fn expires_at(now: DateTime<Utc>, ttl_secs: u64) -> RuntimeResult<DateTime<Utc>> {
    let ttl_secs = i64::try_from(ttl_secs).map_err(|_| RuntimeError::InvalidTtl(ttl_secs))?;
    let delta =
        TimeDelta::try_seconds(ttl_secs).ok_or(RuntimeError::InvalidTtl(ttl_secs as u64))?;
    now.checked_add_signed(delta)
        .ok_or(RuntimeError::InvalidTtl(ttl_secs as u64))
}

fn allows_observation_receipt(manifest: &ActionManifest) -> bool {
    manifest.expected_side_effects.is_empty()
        || manifest
            .expected_side_effects
            .iter()
            .all(|effect| matches!(effect, SideEffectClass::None))
}
