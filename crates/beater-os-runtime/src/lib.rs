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

use std::collections::{BTreeMap, BTreeSet};
use std::path::PathBuf;
use std::time::Duration as StdDuration;

use beater_os_core::{
    ActionKind, ActionManifest, AgentSession, BeaterOsError, Budget, CapabilityGrant,
    CapabilityReceipt, CapabilityReceiptInput, CapabilityScope, CapabilitySelector, DataClass,
    DecisionResult, DelegationMode, ExecutionLeaseReconciliation, ExecutionLeaseResolution,
    GrantConstraints, HashValue, ModelPolicy, ResourceKind, RiskClass, SessionStatus,
    SideEffectClass, TaintLabel, hash_json,
};
use beater_os_sandbox::{SandboxLimits, safe_path_environment};
use beater_os_tool_gateway::{
    ClaimedLocalToolInvocation, GatewayError, execute_claimed_local_tool,
    local_shell_tool_digest_with_environment,
};
use beater_osd::{
    AdmissionOutcome, ClaimableExecutionAction, DAEMON_POLICY_VERSION, DaemonError,
    ExecutionLeaseClaimRequest, LocalShellToolRegistration, SchedulerExecutionLeaseStatus,
    SchedulerOpenExecutionLease, SessionProjection, Store,
};
use chrono::{DateTime, TimeDelta, Utc};
use serde::{Deserialize, Serialize};
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
    #[error(transparent)]
    Gateway(#[from] GatewayError),
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

    /// Run one durable agent-runtime bundle through the daemon-owned authority
    /// path.
    ///
    /// A bundle is the hosted agent-kernel handoff format for future model
    /// workers and service adapters: it can create a session, issue scoped
    /// grants, admit ordered action manifests, and return deterministic replay
    /// evidence without granting the caller a direct store API.
    pub fn run_bundle(&self, bundle: RuntimeBundle) -> RuntimeResult<RuntimeBundleOutcome> {
        let RuntimeBundle {
            session_id: bundle_session_id,
            session,
            grants,
            steps,
        } = bundle;
        let declared_session_id = bundle_session_id
            .or_else(|| session.as_ref().and_then(|start| start.session_id.clone()))
            .or_else(|| steps.first().map(|step| step.session_id.clone()));
        let session_id = declared_session_id.ok_or_else(|| {
            RuntimeError::Refused(
                "runtime bundle must declare a session or at least one step".to_string(),
            )
        })?;
        for step in &steps {
            if step.session_id != session_id {
                return Err(RuntimeError::Refused(format!(
                    "bundle step session_id {} does not match bundle session {session_id}",
                    step.session_id
                )));
            }
            validate_runtime_step_request(step)?;
        }
        let (session_id, created_session) = match session {
            Some(mut start) => {
                if let Some(start_session_id) = &start.session_id
                    && start_session_id != &session_id
                {
                    return Err(RuntimeError::Refused(format!(
                        "bundle session_id {session_id} does not match session genesis {start_session_id}",
                    )));
                }
                if start.session_id.is_none() {
                    start.session_id = Some(session_id.clone());
                }
                let session = self.create_session(start)?;
                (session.session_id, true)
            }
            None => (session_id, false),
        };

        let mut issued_grants = Vec::new();
        for request in grants {
            let grant = self.issue_grant(&session_id, request)?;
            issued_grants.push(grant.grant_id);
        }

        let mut step_reports = Vec::new();
        for step in steps {
            let outcome = self.admit_step(step)?;
            let report = RuntimeBundleStepReport::from_outcome(&outcome);
            let allowed = outcome.admission.decision.result == DecisionResult::Allowed;
            step_reports.push(report);
            if !allowed {
                break;
            }
        }

        let projection = self.store.project(&session_id)?;
        Ok(RuntimeBundleOutcome {
            session_id,
            created_session,
            issued_grants,
            steps: step_reports,
            projection: RuntimeBundleProjectionSummary::from_projection(&projection),
        })
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
        validate_runtime_step_request(&step)?;
        let observation = step.observation.clone();
        let external_revoked_handles = step.external_revoked_handles.clone();
        let manifest = step.into_manifest()?;
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

    /// Run one local-shell worker dispatch for an already-admitted execute
    /// action.
    ///
    /// This is the typed in-process worker loop for the agent runtime. It does
    /// not treat `Allowed` as execution authority. Instead it registers the
    /// exact local command digest, asks the daemon store for claimable actions,
    /// claims one action with manifest/decision/tool compare-and-set fields,
    /// executes through the gateway, and completes the exact open lease.
    pub fn run_local_shell_worker_once(
        &self,
        request: RuntimeLocalShellWorkerRequest,
    ) -> RuntimeResult<Option<RuntimeLocalShellWorkerOutcome>> {
        let projection = self.store.project(&request.session_id)?;
        let prepared = prepare_local_shell_worker(&request)?;
        let registry = self
            .store
            .register_local_shell_tool(LocalShellToolRegistration {
                workspace_id: projection.session.workspace_id.clone(),
                tool_id: prepared.tool_id.clone(),
                version: prepared.tool_version.clone(),
                content_digest: prepared.expected_tool_digest.clone(),
                side_effects: prepared.registered_side_effects.clone(),
                risk_class: prepared.risk,
            })?;
        let claimable = self
            .store
            .claimable_execution_actions(&request.session_id)?;
        let Some(action) = select_worker_action(
            claimable,
            prepared.action_filter.as_deref(),
            &prepared.tool_id,
            &prepared.tool_version,
            &prepared.expected_tool_digest,
            &prepared.computed_digest,
        )?
        else {
            return Ok(None);
        };
        let lease_id = request
            .lease_id
            .unwrap_or_else(|| format!("lease-{}", action.decision_id));
        let lease = self.store.claim_execution_lease_from_admission(
            &request.session_id,
            ExecutionLeaseClaimRequest {
                lease_id,
                action_id: action.action_id.clone(),
                expected_manifest_hash: action.manifest_hash.clone(),
                expected_decision_id: action.decision_id.clone(),
                expected_tool_version: action.expected_tool_version.clone(),
                expected_tool_digest: action.expected_tool_digest.clone(),
                initial_lease_ms: None,
            },
            Utc::now(),
        )?;
        let gateway = execute_claimed_local_tool(
            &self.store,
            &registry,
            &request.session_id,
            &lease.lease.lease_id,
            ClaimedLocalToolInvocation {
                command: prepared.command,
                args: prepared.args,
                cwd: prepared.cwd,
                environment: prepared.environment,
                receipt_id: request.receipt_id,
                limits: prepared.limits,
            },
        )?;
        let projection = self.store.project(&request.session_id)?;
        Ok(Some(RuntimeLocalShellWorkerOutcome {
            session_id: request.session_id,
            action_id: action.action_id,
            lease_id: lease.lease.lease_id,
            manifest_hash: action.manifest_hash,
            decision_id: action.decision_id,
            tool_ref: lease.lease.tool_ref,
            target: lease.lease.target,
            execution: RuntimeWorkerExecutionReport {
                status: gateway.execution.status_str().to_string(),
                exit_code: gateway.execution.exit_code,
                stdout_digest: gateway.execution.stdout_digest(),
                stdout_truncated: gateway.execution.stdout_truncated,
                stderr_truncated: gateway.execution.stderr_truncated,
                created: gateway.execution.diff.created,
                modified: gateway.execution.diff.modified,
                deleted: gateway.execution.diff.deleted,
            },
            receipt: gateway.receipt,
            projection: RuntimeBundleProjectionSummary::from_projection(&projection),
        }))
    }

    /// Plan the next local-shell worker action without claiming, recovering, or
    /// executing anything.
    ///
    /// The plan uses the same local command digest and claimable-action matching
    /// rules as `run_local_shell_worker_once`, but returns a scheduler
    /// recommendation derived from daemon-owned projection state. This gives
    /// external workers a safe preflight before choosing to wait, ask for
    /// explicit recovery, dispatch, or idle.
    pub fn plan_local_shell_worker(
        &self,
        request: RuntimeLocalShellWorkerPlanRequest,
    ) -> RuntimeResult<RuntimeLocalShellWorkerPlanOutcome> {
        if request.max_actions == 0 {
            return Err(RuntimeError::Refused(
                "max_actions must be greater than zero".to_string(),
            ));
        }
        if request.worker.lease_id.is_some() {
            return Err(RuntimeError::Refused(
                "worker plan must not pin a lease_id".to_string(),
            ));
        }
        if request.worker.receipt_id.is_some() {
            return Err(RuntimeError::Refused(
                "worker plan must not pin a receipt_id".to_string(),
            ));
        }
        let prepared = prepare_local_shell_worker(&request.worker)?;
        let projection = self.store.project(&request.worker.session_id)?;
        let observed_at = Utc::now();
        let summary = RuntimeBundleProjectionSummary::from_projection(&projection);
        let mut lease = None;
        let mut dispatch = None;
        let wait_until;
        let reason;
        let action = if summary.live_open_execution_leases > 0 {
            lease = summary
                .open_execution_lease_statuses
                .iter()
                .find(|lease| lease.status == SchedulerExecutionLeaseStatus::LiveOpen)
                .map(RuntimeWorkerLeasePlan::from_scheduler_status);
            wait_until = lease.as_ref().map(|lease| lease.expires_at);
            reason =
                "a live open execution lease blocks dispatch and recovery until it completes or expires"
                    .to_string();
            RuntimeWorkerPreflightAction::WaitLiveLease
        } else if summary.expired_recoverable_execution_leases > 0 {
            lease = summary
                .open_execution_lease_statuses
                .iter()
                .find(|lease| lease.status == SchedulerExecutionLeaseStatus::ExpiredRecoverable)
                .map(RuntimeWorkerLeasePlan::from_scheduler_status);
            wait_until = None;
            reason =
                "an expired open execution lease must be reconciled as outcome_unknown before dispatch"
                    .to_string();
            RuntimeWorkerPreflightAction::RecoverExpiredLease
        } else if summary.runnable_pending_actions == 0 {
            wait_until = None;
            reason = "no pending allowed actions are runnable for this session".to_string();
            RuntimeWorkerPreflightAction::Idle
        } else {
            let claimable = self
                .store
                .claimable_execution_actions(&request.worker.session_id)?;
            match select_worker_action(
                claimable,
                prepared.action_filter.as_deref(),
                &prepared.tool_id,
                &prepared.tool_version,
                &prepared.expected_tool_digest,
                &prepared.computed_digest,
            )? {
                Some(action) => {
                    dispatch = Some(RuntimeWorkerDispatchPlan {
                        action_id: action.action_id,
                        manifest_hash: action.manifest_hash,
                        decision_id: action.decision_id,
                        tool_id: action.tool_id,
                        expected_tool_version: action.expected_tool_version,
                        expected_tool_digest: action.expected_tool_digest,
                        target: action.target,
                        required_grants: action.required_grants,
                        requested_budget: action.requested_budget,
                    });
                    wait_until = None;
                    reason =
                        "a claimable action matches this worker tool and input digest".to_string();
                    RuntimeWorkerPreflightAction::DispatchRunnableAction
                }
                None => {
                    wait_until = None;
                    reason =
                        "runnable actions exist, but none match this worker tool and input digest"
                            .to_string();
                    RuntimeWorkerPreflightAction::NoMatchingAction
                }
            }
        };
        Ok(RuntimeLocalShellWorkerPlanOutcome {
            session_id: request.worker.session_id,
            worker_lane: "local_shell".to_string(),
            observed_at,
            max_actions: request.max_actions,
            action,
            reason,
            wait_until,
            lease,
            tool_id: prepared.tool_id,
            tool_version: prepared.tool_version,
            tool_digest: prepared.expected_tool_digest,
            input_digest: prepared.computed_digest,
            action_filter: prepared.action_filter,
            dispatch,
            projection: summary,
        })
    }

    /// Run a bounded local-shell worker loop over daemon-claimable work.
    ///
    /// The loop is intentionally just repeated one-shot dispatch: every
    /// iteration re-projects daemon state, claims a fresh lease, executes
    /// through the gateway, and completes the exact receipt before continuing.
    /// A live or unreconciled lease stops the loop rather than causing blind
    /// replay.
    pub fn run_local_shell_worker_loop(
        &self,
        request: RuntimeLocalShellWorkerLoopRequest,
    ) -> RuntimeResult<RuntimeLocalShellWorkerLoopOutcome> {
        if request.max_actions == 0 {
            return Err(RuntimeError::Refused(
                "max_actions must be greater than zero".to_string(),
            ));
        }
        if request.worker.lease_id.is_some() {
            return Err(RuntimeError::Refused(
                "worker loop must not pin a lease_id across iterations".to_string(),
            ));
        }
        if request.worker.receipt_id.is_some() {
            return Err(RuntimeError::Refused(
                "worker loop must not pin a receipt_id across iterations".to_string(),
            ));
        }
        let session_id = request.worker.session_id.clone();
        let mut executions = Vec::new();
        for _ in 0..request.max_actions {
            match self.run_local_shell_worker_once(request.worker.clone())? {
                Some(outcome) => executions.push(outcome),
                None => {
                    let projection = self.store.project(&session_id)?;
                    let summary = RuntimeBundleProjectionSummary::from_projection(&projection);
                    let stop_reason = if summary.recovery_blocked {
                        RuntimeLocalShellWorkerLoopStopReason::RecoveryBlocked
                    } else if summary.runnable_pending_actions == 0 {
                        RuntimeLocalShellWorkerLoopStopReason::NoRunnableAction
                    } else {
                        RuntimeLocalShellWorkerLoopStopReason::NoMatchingRunnableAction
                    };
                    return Ok(RuntimeLocalShellWorkerLoopOutcome {
                        session_id,
                        stop_reason,
                        executions,
                        projection: summary,
                    });
                }
            }
        }
        let projection = self.store.project(&session_id)?;
        Ok(RuntimeLocalShellWorkerLoopOutcome {
            session_id,
            stop_reason: RuntimeLocalShellWorkerLoopStopReason::MaxActions,
            executions,
            projection: RuntimeBundleProjectionSummary::from_projection(&projection),
        })
    }

    /// Run one supervised worker cycle: recover expired lost leases, then drain
    /// claimable local-shell work through the bounded worker loop.
    ///
    /// This is the minimal scheduler supervision primitive. It treats an
    /// expired open lease as a recovery event (`outcome_unknown`), never as a
    /// retryable action. Live leases still stop the cycle through the worker
    /// loop's `recovery_blocked` stop state.
    pub fn run_supervised_local_shell_worker_cycle(
        &self,
        request: RuntimeSupervisedLocalShellWorkerCycleRequest,
    ) -> RuntimeResult<RuntimeSupervisedLocalShellWorkerCycleOutcome> {
        let session_id = request.worker_loop.worker.session_id.clone();
        let mut recoveries = Vec::new();
        for _ in 0..request.max_recoveries {
            let Some(recovery_target) = self.next_expired_open_execution_lease(&session_id)? else {
                break;
            };
            let recovery = self
                .recover_expired_execution_lease_once(RuntimeExecutionLeaseRecoveryRequest {
                    session_id: session_id.clone(),
                    action_id: Some(recovery_target.action_id.clone()),
                    lease_id: Some(recovery_target.lease_id.clone()),
                    reconciliation_id: Some(format!(
                        "supervised-reconcile-{}",
                        recovery_target.lease_id
                    )),
                    reconciled_by: request.reconciled_by.clone(),
                    reason: Some(request.recovery_reason.clone()),
                    evidence_refs: request.recovery_evidence_refs.clone(),
                })?
                .ok_or_else(|| {
                    RuntimeError::Refused(format!(
                        "expired execution lease {} disappeared before recovery",
                        recovery_target.lease_id
                    ))
                })?;
            recoveries.push(recovery);
        }
        let worker_loop = self.run_local_shell_worker_loop(request.worker_loop)?;
        let projection = worker_loop.projection.clone();
        Ok(RuntimeSupervisedLocalShellWorkerCycleOutcome {
            session_id,
            recoveries,
            worker_loop,
            projection,
        })
    }

    /// Reconcile one expired open execution lease as `outcome_unknown`.
    ///
    /// This is the runtime recovery path for a worker that claimed authority but
    /// lost its process, transport, or local state before it could durably
    /// complete a receipt. It never fabricates success or retries the action:
    /// the daemon writes an explicit reconciliation record, closes the recovery
    /// blocker, and leaves the side-effect outcome unknown for review/replay.
    pub fn recover_expired_execution_lease_once(
        &self,
        request: RuntimeExecutionLeaseRecoveryRequest,
    ) -> RuntimeResult<Option<RuntimeExecutionLeaseRecoveryOutcome>> {
        let projection = self.store.project(&request.session_id)?;
        let closed_actions = closed_execution_actions(&projection);
        let mut matching_open_leases = projection
            .execution_leases
            .iter()
            .filter(|lease| !closed_actions.contains(lease.action_id.as_str()))
            .filter(|lease| {
                request
                    .action_id
                    .as_ref()
                    .is_none_or(|action_id| action_id == &lease.action_id)
            })
            .filter(|lease| {
                request
                    .lease_id
                    .as_ref()
                    .is_none_or(|lease_id| lease_id == &lease.lease_id)
            });
        let Some(open_lease) = matching_open_leases.next() else {
            return Ok(None);
        };
        if let Some(second) = matching_open_leases.next() {
            return Err(RuntimeError::Refused(format!(
                "multiple open execution leases matched recovery request: {} and {}",
                open_lease.lease_id, second.lease_id
            )));
        }
        let open_lease = open_lease.clone();
        let now = Utc::now();
        if now < open_lease.expires_at {
            return Err(RuntimeError::Refused(format!(
                "execution lease {} is still live until {}",
                open_lease.lease_id, open_lease.expires_at
            )));
        }
        let reason = request.reason.unwrap_or_else(|| {
            "runtime worker lease expired before a receipt was completed".to_string()
        });
        if reason.trim().is_empty() {
            return Err(RuntimeError::Refused(
                "recovery reason must not be empty".to_string(),
            ));
        }
        if request
            .evidence_refs
            .iter()
            .any(|reference| reference.trim().is_empty())
        {
            return Err(RuntimeError::Refused(
                "recovery evidence_refs must not contain empty references".to_string(),
            ));
        }
        let reconciliation_id = request
            .reconciliation_id
            .unwrap_or_else(|| format!("reconcile-{}", open_lease.lease_id));
        let reconciled_by = request
            .reconciled_by
            .unwrap_or_else(|| projection.session.created_by.clone());
        let reconciliation = ExecutionLeaseReconciliation {
            reconciliation_id: reconciliation_id.clone(),
            lease_id: open_lease.lease_id.clone(),
            session_id: request.session_id.clone(),
            action_id: open_lease.action_id.clone(),
            manifest_hash: open_lease.manifest_hash.clone(),
            decision_id: open_lease.decision_id.clone(),
            resolution: ExecutionLeaseResolution::OutcomeUnknown,
            reconciled_by,
            reason,
            evidence_refs: request.evidence_refs,
            reconciled_at: now,
        };
        let record =
            self.store
                .reconcile_execution_lease(&request.session_id, reconciliation, now)?;
        let projection = self.store.project(&request.session_id)?;
        Ok(Some(RuntimeExecutionLeaseRecoveryOutcome {
            session_id: request.session_id,
            action_id: open_lease.action_id.clone(),
            lease_id: open_lease.lease_id.clone(),
            reconciliation_id,
            resolution: ExecutionLeaseResolution::OutcomeUnknown,
            reconciliation_seq: record.seq,
            reconciliation_hash: record.hash.clone(),
            final_journal_root_hash: record.hash,
            projection: RuntimeBundleProjectionSummary::from_projection(&projection),
        }))
    }

    fn next_expired_open_execution_lease(
        &self,
        session_id: &str,
    ) -> RuntimeResult<Option<RuntimeExpiredOpenExecutionLease>> {
        let projection = self.store.project(session_id)?;
        let closed_actions = closed_execution_actions(&projection);
        let now = Utc::now();
        Ok(projection
            .execution_leases
            .iter()
            .filter(|lease| !closed_actions.contains(lease.action_id.as_str()))
            .filter(|lease| now >= lease.expires_at)
            .map(|lease| RuntimeExpiredOpenExecutionLease {
                action_id: lease.action_id.clone(),
                lease_id: lease.lease_id.clone(),
            })
            .next())
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
                payment_receipt: None,
            },
            finished_at,
        )?;
        Ok(receipt)
    }
}

/// Start state for a durable agent session.
#[derive(Clone, Debug, Serialize, Deserialize)]
pub struct SessionStart {
    #[serde(default)]
    pub session_id: Option<String>,
    #[serde(default)]
    pub created_by: Option<String>,
    pub agent_id: String,
    pub workspace_id: String,
    pub goal: String,
    #[serde(default)]
    pub constraints: Vec<String>,
    #[serde(default = "default_policy_profile")]
    pub policy_profile: String,
    #[serde(default)]
    pub initial_capability_ids: BTreeSet<String>,
    #[serde(default)]
    pub budget: Budget,
    #[serde(default)]
    pub model_policy: ModelPolicy,
    #[serde(default)]
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
#[derive(Clone, Debug, Serialize, Deserialize)]
pub struct GrantRequest {
    #[serde(default)]
    pub grant_id: Option<String>,
    #[serde(default)]
    pub issuer: Option<String>,
    #[serde(default)]
    pub holder: Option<String>,
    #[serde(default)]
    pub parent_grant_id: Option<String>,
    pub resource_kind: ResourceKind,
    pub resource_id: String,
    pub actions: BTreeSet<ActionKind>,
    #[serde(default)]
    pub denied_actions: BTreeSet<ActionKind>,
    #[serde(default)]
    pub constraints: GrantConstraints,
    #[serde(default = "default_grant_ttl_secs")]
    pub expires_in_secs: u64,
    #[serde(default = "default_delegation_none")]
    pub delegation: DelegationMode,
    #[serde(default)]
    pub approval: beater_os_core::ApprovalRequirement,
    #[serde(default)]
    pub revocation_handle: Option<String>,
    #[serde(default)]
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
#[derive(Clone, Debug, Serialize, Deserialize)]
pub struct RuntimeStep {
    pub session_id: String,
    #[serde(default)]
    pub action_id: Option<String>,
    #[serde(default)]
    pub tool_id: Option<String>,
    pub action_kind: ActionKind,
    pub target: CapabilitySelector,
    #[serde(default)]
    pub resolved_target: Option<CapabilitySelector>,
    pub inputs_summary: String,
    #[serde(default)]
    pub inputs_digest: Option<String>,
    #[serde(default)]
    pub expected_outputs: Vec<String>,
    #[serde(default)]
    pub expected_side_effects: BTreeSet<SideEffectClass>,
    #[serde(default)]
    pub required_grants: BTreeSet<String>,
    #[serde(default)]
    pub requested_budget: Budget,
    pub risk_class: RiskClass,
    #[serde(default)]
    pub data_classes: BTreeSet<DataClass>,
    #[serde(default)]
    pub taint: BTreeSet<TaintLabel>,
    #[serde(default)]
    pub idempotency_key: Option<String>,
    #[serde(default)]
    pub compensation_plan: Option<String>,
    pub human_explanation: String,
    #[serde(default)]
    pub external_revoked_handles: BTreeSet<String>,
    #[serde(default)]
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
#[derive(Clone, Debug, Serialize, Deserialize)]
pub struct RuntimeObservation {
    #[serde(default)]
    pub receipt_id: Option<String>,
    #[serde(default)]
    pub started_at: Option<DateTime<Utc>>,
    #[serde(default)]
    pub finished_at: Option<DateTime<Utc>>,
    pub status: String,
    pub output_summary: String,
    pub side_effect_summary: String,
    #[serde(default)]
    pub external_ids: Vec<String>,
    #[serde(default)]
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

/// One local-shell worker attempt over already-admitted runtime work.
#[derive(Clone, Debug, Serialize, Deserialize)]
pub struct RuntimeLocalShellWorkerRequest {
    pub session_id: String,
    #[serde(default)]
    pub action_id: Option<String>,
    #[serde(default)]
    pub lease_id: Option<String>,
    #[serde(default)]
    pub tool: Option<String>,
    #[serde(default)]
    pub tool_version: Option<String>,
    #[serde(default)]
    pub tool_digest: Option<String>,
    pub command: String,
    #[serde(default)]
    pub args: Vec<String>,
    pub cwd: String,
    #[serde(default)]
    pub env: BTreeMap<String, String>,
    #[serde(default)]
    pub side_effects: BTreeSet<SideEffectClass>,
    #[serde(default)]
    pub risk: Option<RiskClass>,
    #[serde(default)]
    pub receipt_id: Option<String>,
    #[serde(default)]
    pub timeout_secs: Option<u64>,
    #[serde(default)]
    pub max_output_bytes: Option<usize>,
}

/// Result of one lease-bound local-shell worker dispatch.
#[derive(Clone, Debug, Serialize, Deserialize)]
pub struct RuntimeLocalShellWorkerOutcome {
    pub session_id: String,
    pub action_id: String,
    pub lease_id: String,
    pub manifest_hash: HashValue,
    pub decision_id: String,
    pub tool_ref: String,
    pub target: CapabilitySelector,
    pub execution: RuntimeWorkerExecutionReport,
    pub receipt: CapabilityReceipt,
    pub projection: RuntimeBundleProjectionSummary,
}

#[derive(Clone, Debug, Serialize, Deserialize)]
pub struct RuntimeWorkerExecutionReport {
    pub status: String,
    #[serde(default)]
    pub exit_code: Option<i32>,
    pub stdout_digest: String,
    pub stdout_truncated: bool,
    pub stderr_truncated: bool,
    pub created: Vec<String>,
    pub modified: Vec<String>,
    pub deleted: Vec<String>,
}

/// Bounded worker-loop request over already-admitted local-shell work.
#[derive(Clone, Debug, Serialize, Deserialize)]
pub struct RuntimeLocalShellWorkerLoopRequest {
    pub worker: RuntimeLocalShellWorkerRequest,
    pub max_actions: usize,
}

/// Side-effect-free local-shell worker planning request.
#[derive(Clone, Debug, Serialize, Deserialize)]
pub struct RuntimeLocalShellWorkerPlanRequest {
    pub worker: RuntimeLocalShellWorkerRequest,
    pub max_actions: usize,
}

/// Result of a side-effect-free local-shell worker preflight.
#[derive(Clone, Debug, Serialize, Deserialize)]
pub struct RuntimeLocalShellWorkerPlanOutcome {
    pub session_id: String,
    pub worker_lane: String,
    pub observed_at: DateTime<Utc>,
    pub max_actions: usize,
    pub action: RuntimeWorkerPreflightAction,
    pub reason: String,
    #[serde(default)]
    pub wait_until: Option<DateTime<Utc>>,
    #[serde(default)]
    pub lease: Option<RuntimeWorkerLeasePlan>,
    pub tool_id: String,
    pub tool_version: String,
    pub tool_digest: String,
    pub input_digest: String,
    #[serde(default)]
    pub action_filter: Option<String>,
    #[serde(default)]
    pub dispatch: Option<RuntimeWorkerDispatchPlan>,
    pub projection: RuntimeBundleProjectionSummary,
}

/// Scheduler recommendation for a worker that has not yet claimed authority.
#[derive(Clone, Debug, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum RuntimeWorkerPreflightAction {
    WaitLiveLease,
    RecoverExpiredLease,
    DispatchRunnableAction,
    Idle,
    NoMatchingAction,
}

/// Open lease that blocks dispatch in a preflight plan.
#[derive(Clone, Debug, Serialize, Deserialize)]
pub struct RuntimeWorkerLeasePlan {
    pub action_id: String,
    pub lease_id: String,
    pub expires_at: DateTime<Utc>,
    pub status: SchedulerExecutionLeaseStatus,
}

impl RuntimeWorkerLeasePlan {
    fn from_scheduler_status(lease: &SchedulerOpenExecutionLease) -> Self {
        Self {
            action_id: lease.action_id.clone(),
            lease_id: lease.lease_id.clone(),
            expires_at: lease.expires_at,
            status: lease.status.clone(),
        }
    }
}

/// Claimable dispatch target selected by a preflight plan.
#[derive(Clone, Debug, Serialize, Deserialize)]
pub struct RuntimeWorkerDispatchPlan {
    pub action_id: String,
    pub manifest_hash: HashValue,
    pub decision_id: String,
    pub tool_id: String,
    pub expected_tool_version: String,
    pub expected_tool_digest: HashValue,
    pub target: CapabilitySelector,
    pub required_grants: BTreeSet<String>,
    pub requested_budget: Budget,
}

/// Result of a bounded local-shell worker loop.
#[derive(Clone, Debug, Serialize, Deserialize)]
pub struct RuntimeLocalShellWorkerLoopOutcome {
    pub session_id: String,
    pub stop_reason: RuntimeLocalShellWorkerLoopStopReason,
    pub executions: Vec<RuntimeLocalShellWorkerOutcome>,
    pub projection: RuntimeBundleProjectionSummary,
}

/// Why a bounded worker loop stopped without surfacing an execution error.
#[derive(Clone, Debug, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum RuntimeLocalShellWorkerLoopStopReason {
    MaxActions,
    NoRunnableAction,
    NoMatchingRunnableAction,
    RecoveryBlocked,
}

/// Supervised worker-cycle request: recovery budget followed by a worker loop.
#[derive(Clone, Debug, Serialize, Deserialize)]
pub struct RuntimeSupervisedLocalShellWorkerCycleRequest {
    pub worker_loop: RuntimeLocalShellWorkerLoopRequest,
    #[serde(default)]
    pub max_recoveries: usize,
    #[serde(default = "default_supervised_recovery_reason")]
    pub recovery_reason: String,
    #[serde(default)]
    pub reconciled_by: Option<String>,
    #[serde(default)]
    pub recovery_evidence_refs: Vec<String>,
}

/// Result of one supervised worker cycle.
#[derive(Clone, Debug, Serialize, Deserialize)]
pub struct RuntimeSupervisedLocalShellWorkerCycleOutcome {
    pub session_id: String,
    pub recoveries: Vec<RuntimeExecutionLeaseRecoveryOutcome>,
    pub worker_loop: RuntimeLocalShellWorkerLoopOutcome,
    pub projection: RuntimeBundleProjectionSummary,
}

#[derive(Clone, Debug)]
struct RuntimeExpiredOpenExecutionLease {
    action_id: String,
    lease_id: String,
}

/// Request to close a stale worker lease without inventing a receipt.
#[derive(Clone, Debug, Serialize, Deserialize)]
pub struct RuntimeExecutionLeaseRecoveryRequest {
    pub session_id: String,
    #[serde(default)]
    pub action_id: Option<String>,
    #[serde(default)]
    pub lease_id: Option<String>,
    #[serde(default)]
    pub reconciliation_id: Option<String>,
    #[serde(default)]
    pub reconciled_by: Option<String>,
    #[serde(default)]
    pub reason: Option<String>,
    #[serde(default)]
    pub evidence_refs: Vec<String>,
}

/// Result of one runtime worker lease recovery write.
#[derive(Clone, Debug, Serialize, Deserialize)]
pub struct RuntimeExecutionLeaseRecoveryOutcome {
    pub session_id: String,
    pub action_id: String,
    pub lease_id: String,
    pub reconciliation_id: String,
    pub resolution: ExecutionLeaseResolution,
    pub reconciliation_seq: u64,
    pub reconciliation_hash: HashValue,
    pub final_journal_root_hash: HashValue,
    pub projection: RuntimeBundleProjectionSummary,
}

#[derive(Clone, Debug)]
pub struct RuntimeStepOutcome {
    pub admission: AdmissionOutcome,
    pub receipt: Option<CapabilityReceipt>,
    pub evidence: StepReplayEvidence,
    pub projection: SessionProjection,
}

/// Serializable hosted-runtime work bundle.
#[derive(Clone, Debug, Serialize, Deserialize)]
pub struct RuntimeBundle {
    /// Existing session id for grant/step execution, or the id to assign when
    /// `session` omits one.
    #[serde(default)]
    pub session_id: Option<String>,
    /// Optional session genesis. If omitted, the bundle operates on an existing
    /// session.
    #[serde(default)]
    pub session: Option<SessionStart>,
    /// Grants to issue before step admission, in order.
    #[serde(default)]
    pub grants: Vec<GrantRequest>,
    /// Ordered model-proposed steps. Execution stops after the first non-allowed
    /// admission.
    #[serde(default)]
    pub steps: Vec<RuntimeStep>,
}

/// Serializable result of running a hosted-runtime work bundle.
#[derive(Clone, Debug, Serialize, Deserialize)]
pub struct RuntimeBundleOutcome {
    pub session_id: String,
    pub created_session: bool,
    pub issued_grants: Vec<String>,
    pub steps: Vec<RuntimeBundleStepReport>,
    pub projection: RuntimeBundleProjectionSummary,
}

#[derive(Clone, Debug, Serialize, Deserialize)]
pub struct RuntimeBundleStepReport {
    pub action_id: String,
    pub decision: DecisionResult,
    pub explanation: String,
    #[serde(default)]
    pub receipt_id: Option<String>,
    pub evidence: StepReplayEvidence,
}

impl RuntimeBundleStepReport {
    fn from_outcome(outcome: &RuntimeStepOutcome) -> Self {
        Self {
            action_id: outcome.evidence.action_id.clone(),
            decision: outcome.admission.decision.result.clone(),
            explanation: outcome.admission.decision.explanation.clone(),
            receipt_id: outcome
                .receipt
                .as_ref()
                .map(|receipt| receipt.receipt_id.clone()),
            evidence: outcome.evidence.clone(),
        }
    }
}

#[derive(Clone, Debug, Serialize, Deserialize)]
pub struct RuntimeBundleProjectionSummary {
    pub grants: usize,
    pub active_grants: usize,
    pub actions: usize,
    pub decisions: usize,
    #[serde(default)]
    pub pending_allowed_actions: usize,
    #[serde(default)]
    pub pending_allowed_action_ids: Vec<String>,
    #[serde(default)]
    pub runnable_pending_actions: usize,
    #[serde(default)]
    pub runnable_pending_action_ids: Vec<String>,
    #[serde(default)]
    pub execution_leases: usize,
    #[serde(default)]
    pub open_execution_leases: usize,
    #[serde(default)]
    pub open_execution_lease_ids: Vec<String>,
    #[serde(default)]
    pub open_execution_lease_statuses: Vec<SchedulerOpenExecutionLease>,
    #[serde(default)]
    pub live_open_execution_leases: usize,
    #[serde(default)]
    pub live_open_execution_lease_ids: Vec<String>,
    #[serde(default)]
    pub expired_recoverable_execution_leases: usize,
    #[serde(default)]
    pub expired_recoverable_execution_lease_ids: Vec<String>,
    #[serde(default)]
    pub execution_reconciliations: usize,
    #[serde(default)]
    pub recovery_blocked: bool,
    #[serde(default)]
    pub admission_blocked: bool,
    #[serde(default)]
    pub admission_blockers: Vec<String>,
    pub receipts: usize,
}

impl RuntimeBundleProjectionSummary {
    fn from_projection(projection: &SessionProjection) -> Self {
        let scheduler = projection.scheduler_projection(Utc::now());
        Self {
            grants: projection.grants.len(),
            active_grants: projection.active_grants(Utc::now()).len(),
            actions: projection.manifests.len(),
            decisions: projection.decisions.len(),
            pending_allowed_actions: scheduler.pending_allowed_action_ids.len(),
            pending_allowed_action_ids: scheduler.pending_allowed_action_ids,
            runnable_pending_actions: scheduler.runnable_pending_action_ids.len(),
            runnable_pending_action_ids: scheduler.runnable_pending_action_ids,
            execution_leases: projection.execution_leases.len(),
            open_execution_leases: scheduler.open_execution_lease_ids.len(),
            recovery_blocked: scheduler.recovery_blocked,
            open_execution_lease_statuses: scheduler.open_execution_lease_statuses,
            live_open_execution_leases: scheduler.live_open_execution_lease_ids.len(),
            live_open_execution_lease_ids: scheduler.live_open_execution_lease_ids,
            expired_recoverable_execution_leases: scheduler
                .expired_recoverable_execution_lease_ids
                .len(),
            expired_recoverable_execution_lease_ids: scheduler
                .expired_recoverable_execution_lease_ids,
            open_execution_lease_ids: scheduler.open_execution_lease_ids,
            execution_reconciliations: projection.execution_reconciliations.len(),
            admission_blocked: scheduler.admission_blocked,
            admission_blockers: scheduler.admission_blockers,
            receipts: projection.receipts.len(),
        }
    }
}

fn closed_execution_actions(projection: &SessionProjection) -> BTreeSet<&str> {
    let mut closed_actions: BTreeSet<&str> = projection
        .receipts
        .iter()
        .map(|receipt| receipt.action_id.as_str())
        .collect();
    closed_actions.extend(
        projection
            .execution_reconciliations
            .iter()
            .map(|reconciliation| reconciliation.action_id.as_str()),
    );
    closed_actions
}

struct RuntimePreparedLocalShellWorker {
    action_filter: Option<String>,
    command: String,
    args: Vec<String>,
    cwd: String,
    environment: BTreeMap<String, String>,
    limits: SandboxLimits,
    expected_tool_digest: String,
    computed_digest: String,
    tool_id: String,
    tool_version: String,
    registered_side_effects: BTreeSet<SideEffectClass>,
    risk: RiskClass,
}

fn prepare_local_shell_worker(
    request: &RuntimeLocalShellWorkerRequest,
) -> RuntimeResult<RuntimePreparedLocalShellWorker> {
    let command = require_worker_non_empty("command", request.command.clone())?;
    if command.contains('/') {
        return Err(RuntimeError::Refused(
            "local shell worker accepts PATH-resolved command names only".to_string(),
        ));
    }
    let cwd = require_worker_non_empty("cwd", request.cwd.clone())?;
    let mut environment = safe_path_environment();
    for (name, value) in request.env.clone() {
        if name == "PATH" {
            return Err(RuntimeError::Refused(
                "PATH is reserved for the sandbox safe system search path".to_string(),
            ));
        }
        if environment.contains_key(&name) {
            return Err(RuntimeError::Refused(format!(
                "duplicate environment variable {name:?}"
            )));
        }
        environment.insert(name, value);
    }
    let defaults = SandboxLimits::default();
    let timeout_secs = request.timeout_secs.unwrap_or(30);
    if timeout_secs == 0 {
        return Err(RuntimeError::Refused(
            "timeout_secs must be greater than zero".to_string(),
        ));
    }
    let max_output_bytes = request
        .max_output_bytes
        .unwrap_or(defaults.max_output_bytes);
    if max_output_bytes > defaults.max_output_bytes {
        return Err(RuntimeError::Refused(format!(
            "max_output_bytes must be at most {}",
            defaults.max_output_bytes
        )));
    }
    let limits = SandboxLimits {
        timeout: StdDuration::from_secs(timeout_secs),
        max_output_bytes,
        ..defaults
    };
    let args = request.args.clone();
    let computed_digest =
        local_shell_tool_digest_with_environment(&cwd, &command, &args, &environment)?;
    let expected_tool_digest = request
        .tool_digest
        .clone()
        .unwrap_or_else(|| computed_digest.clone());
    if expected_tool_digest != computed_digest {
        return Err(RuntimeError::Gateway(GatewayError::ToolDigestMismatch));
    }
    let tool_id = request.tool.clone().unwrap_or_else(|| "shell".to_string());
    let tool_version = request.tool_version.clone().unwrap_or_else(|| {
        let prefix_len = expected_tool_digest.len().min(16);
        format!("local-{}", &expected_tool_digest[..prefix_len])
    });
    let registered_side_effects = if request.side_effects.is_empty() {
        BTreeSet::from([SideEffectClass::LocalWrite])
    } else {
        request.side_effects.clone()
    };
    Ok(RuntimePreparedLocalShellWorker {
        action_filter: request.action_id.clone(),
        command,
        args,
        cwd,
        environment,
        limits,
        expected_tool_digest,
        computed_digest,
        tool_id,
        tool_version,
        registered_side_effects,
        risk: request.risk.unwrap_or(RiskClass::Low),
    })
}

fn select_worker_action(
    actions: Vec<ClaimableExecutionAction>,
    action_filter: Option<&str>,
    tool_id: &str,
    tool_version: &str,
    tool_digest: &str,
    input_digest: &str,
) -> RuntimeResult<Option<ClaimableExecutionAction>> {
    let mut mismatched_filter = false;
    for action in actions {
        if let Some(expected_action_id) = action_filter
            && action.action_id != expected_action_id
        {
            continue;
        }
        if action.tool_id != tool_id
            || action.expected_tool_version != tool_version
            || action.expected_tool_digest != tool_digest
            || action.manifest.inputs_digest != input_digest
        {
            mismatched_filter = action_filter.is_some();
            continue;
        }
        return Ok(Some(action));
    }
    if mismatched_filter {
        return Err(RuntimeError::Refused(
            "selected action is claimable but does not match the worker tool/input digest"
                .to_string(),
        ));
    }
    Ok(None)
}

/// Deterministic replay anchor for one runtime step.
///
/// `PolicyDecision::decision_id` is intentionally nonce-like, so replay should
/// anchor on the manifest hash plus append-only journal/receipt chain roots.
#[derive(Clone, Debug, PartialEq, Eq, Serialize, Deserialize)]
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

fn require_worker_non_empty(field: &str, value: String) -> RuntimeResult<String> {
    if value.trim().is_empty() {
        return Err(RuntimeError::Refused(format!("{field} must not be empty")));
    }
    Ok(value)
}

fn default_policy_profile() -> String {
    "default".to_string()
}

fn default_grant_ttl_secs() -> u64 {
    DEFAULT_GRANT_TTL_SECS
}

fn default_supervised_recovery_reason() -> String {
    "supervised worker cycle found an expired open execution lease".to_string()
}

fn default_delegation_none() -> DelegationMode {
    DelegationMode::None
}

fn expires_at(now: DateTime<Utc>, ttl_secs: u64) -> RuntimeResult<DateTime<Utc>> {
    let ttl_secs = i64::try_from(ttl_secs).map_err(|_| RuntimeError::InvalidTtl(ttl_secs))?;
    let delta =
        TimeDelta::try_seconds(ttl_secs).ok_or(RuntimeError::InvalidTtl(ttl_secs as u64))?;
    now.checked_add_signed(delta)
        .ok_or(RuntimeError::InvalidTtl(ttl_secs as u64))
}

fn validate_runtime_step_request(step: &RuntimeStep) -> RuntimeResult<()> {
    if step.observation.is_none() {
        return Ok(());
    }
    if let Some(tool_id) = step.tool_id.as_deref()
        && tool_id != DEFAULT_RUNTIME_TOOL_ID
    {
        return Err(RuntimeError::Refused(format!(
            "runtime observation receipts must use {DEFAULT_RUNTIME_TOOL_ID}, not {tool_id}"
        )));
    }
    if !allows_observation_side_effects(&step.expected_side_effects) {
        return Err(RuntimeError::Refused(
            "runtime observation receipts may only claim no side effects".to_string(),
        ));
    }
    Ok(())
}

fn allows_observation_side_effects(effects: &BTreeSet<SideEffectClass>) -> bool {
    effects.is_empty()
        || effects
            .iter()
            .all(|effect| matches!(effect, SideEffectClass::None))
}

#[cfg(test)]
mod tests {
    use std::collections::BTreeSet;
    use std::error::Error;
    use std::fs;

    use super::*;

    #[test]
    fn bundle_creates_session_issues_grant_and_records_observation_evidence()
    -> Result<(), Box<dyn Error>> {
        let root =
            std::env::temp_dir().join(format!("beater-os-runtime-bundle-{}", Uuid::new_v4()));
        let runtime = AgentRuntime::open(&root)?;
        let session_id = "bundle-session".to_string();
        let grant_id = default_root_grant_id(&session_id);
        let target = CapabilitySelector {
            resource_kind: ResourceKind::FilePath,
            resource_id: "/tmp/beater-os-runtime-bundle-observe".to_string(),
        };
        let outcome = runtime.run_bundle(RuntimeBundle {
            session_id: Some(session_id.clone()),
            session: Some(SessionStart::new(
                "agent:bundle",
                "workspace:bundle",
                "prove hosted runtime bundle orchestration",
            )),
            grants: vec![GrantRequest::new(
                ResourceKind::FilePath,
                "*",
                [ActionKind::Read],
            )],
            steps: vec![RuntimeStep {
                session_id: session_id.clone(),
                action_id: Some("bundle-observe-action".to_string()),
                tool_id: Some(DEFAULT_RUNTIME_TOOL_ID.to_string()),
                action_kind: ActionKind::Read,
                target: target.clone(),
                resolved_target: Some(target),
                inputs_summary: "observe runtime bundle state".to_string(),
                inputs_digest: None,
                expected_outputs: Vec::new(),
                expected_side_effects: BTreeSet::from([SideEffectClass::None]),
                required_grants: BTreeSet::from([grant_id.clone()]),
                requested_budget: Budget::default(),
                risk_class: RiskClass::Low,
                data_classes: BTreeSet::from([DataClass::Internal]),
                taint: BTreeSet::new(),
                idempotency_key: Some("bundle-observe-action".to_string()),
                compensation_plan: None,
                human_explanation: "read-only runtime bundle observation".to_string(),
                external_revoked_handles: BTreeSet::new(),
                observation: Some(RuntimeObservation::ok("bundle observed")),
            }],
        })?;

        assert!(outcome.created_session);
        assert_eq!(outcome.session_id, session_id);
        assert_eq!(outcome.issued_grants, vec![grant_id]);
        assert_eq!(outcome.steps.len(), 1);
        assert_eq!(outcome.steps[0].decision, DecisionResult::Allowed);
        assert!(outcome.steps[0].receipt_id.is_some());
        assert_eq!(outcome.projection.grants, 1);
        assert_eq!(outcome.projection.actions, 1);
        assert_eq!(outcome.projection.decisions, 1);
        assert_eq!(outcome.projection.pending_allowed_actions, 0);
        assert_eq!(outcome.projection.runnable_pending_actions, 0);
        assert_eq!(outcome.projection.execution_leases, 0);
        assert_eq!(outcome.projection.open_execution_leases, 0);
        assert!(!outcome.projection.recovery_blocked);
        assert!(!outcome.projection.admission_blocked);
        assert_eq!(outcome.projection.receipts, 1);

        let _ = fs::remove_dir_all(root);
        Ok(())
    }

    #[test]
    fn bundle_rejects_session_mismatch_before_persisting() -> Result<(), Box<dyn Error>> {
        let root = std::env::temp_dir().join(format!(
            "beater-os-runtime-bundle-reject-{}",
            Uuid::new_v4()
        ));
        let runtime = AgentRuntime::open(&root)?;
        let mut start = SessionStart::new(
            "agent:bundle",
            "workspace:bundle",
            "prove mismatched bundle rejection",
        );
        start.session_id = Some("genesis-session".to_string());

        let result = runtime.run_bundle(RuntimeBundle {
            session_id: Some("declared-session".to_string()),
            session: Some(start),
            grants: Vec::new(),
            steps: Vec::new(),
        });

        assert!(matches!(result, Err(RuntimeError::Refused(_))));
        assert!(!runtime.store().session_exists("declared-session")?);
        assert!(!runtime.store().session_exists("genesis-session")?);

        let _ = fs::remove_dir_all(root);
        Ok(())
    }

    #[test]
    fn bundle_rejects_step_mismatch_before_grants_are_issued() -> Result<(), Box<dyn Error>> {
        let root = std::env::temp_dir().join(format!(
            "beater-os-runtime-bundle-step-reject-{}",
            Uuid::new_v4()
        ));
        let runtime = AgentRuntime::open(&root)?;
        let session_id = "bundle-session".to_string();
        let mut start = SessionStart::new(
            "agent:bundle",
            "workspace:bundle",
            "prove step mismatch rejection",
        );
        start.session_id = Some(session_id.clone());

        let result = runtime.run_bundle(RuntimeBundle {
            session_id: Some(session_id.clone()),
            session: Some(start),
            grants: vec![GrantRequest::new(
                ResourceKind::FilePath,
                "*",
                [ActionKind::Read],
            )],
            steps: vec![RuntimeStep::new(
                "other-session",
                ActionKind::Read,
                CapabilitySelector {
                    resource_kind: ResourceKind::FilePath,
                    resource_id: "/tmp/beater-os-runtime-bundle-observe".to_string(),
                },
                "observe runtime bundle state",
            )],
        });

        assert!(matches!(result, Err(RuntimeError::Refused(_))));
        assert!(!runtime.store().session_exists(&session_id)?);

        let _ = fs::remove_dir_all(root);
        Ok(())
    }

    #[test]
    fn bundle_rejects_observation_tool_override_before_persisting() -> Result<(), Box<dyn Error>> {
        let root = std::env::temp_dir().join(format!(
            "beater-os-runtime-bundle-tool-reject-{}",
            Uuid::new_v4()
        ));
        let runtime = AgentRuntime::open(&root)?;
        let session_id = "bundle-session".to_string();
        let mut start = SessionStart::new(
            "agent:bundle",
            "workspace:bundle",
            "prove observation tool binding",
        );
        start.session_id = Some(session_id.clone());

        let result = runtime.run_bundle(RuntimeBundle {
            session_id: Some(session_id.clone()),
            session: Some(start),
            grants: vec![GrantRequest::new(
                ResourceKind::FilePath,
                "*",
                [ActionKind::Read],
            )],
            steps: vec![RuntimeStep {
                session_id: session_id.clone(),
                action_id: Some("bundle-observe-action".to_string()),
                tool_id: Some("shell".to_string()),
                action_kind: ActionKind::Read,
                target: CapabilitySelector {
                    resource_kind: ResourceKind::FilePath,
                    resource_id: "/tmp/beater-os-runtime-bundle-observe".to_string(),
                },
                resolved_target: None,
                inputs_summary: "observe runtime bundle state".to_string(),
                inputs_digest: None,
                expected_outputs: Vec::new(),
                expected_side_effects: BTreeSet::from([SideEffectClass::None]),
                required_grants: BTreeSet::new(),
                requested_budget: Budget::default(),
                risk_class: RiskClass::Low,
                data_classes: BTreeSet::new(),
                taint: BTreeSet::new(),
                idempotency_key: None,
                compensation_plan: None,
                human_explanation: "attempt spoofed shell observation receipt".to_string(),
                external_revoked_handles: BTreeSet::new(),
                observation: Some(RuntimeObservation::ok("spoofed shell observation")),
            }],
        });

        assert!(matches!(result, Err(RuntimeError::Refused(_))));
        assert!(!runtime.store().session_exists(&session_id)?);

        let _ = fs::remove_dir_all(root);
        Ok(())
    }

    #[test]
    fn sparse_bundle_json_deserializes_with_contract_defaults() -> Result<(), Box<dyn Error>> {
        let bundle: RuntimeBundle = serde_json::from_value(serde_json::json!({
            "session_id": "json-bundle-session",
            "session": {
                "agent_id": "agent:json",
                "workspace_id": "workspace:json",
                "goal": "prove sparse bundle defaults"
            },
            "grants": [{
                "resource_kind": "file_path",
                "resource_id": "*",
                "actions": ["read"]
            }],
            "steps": [{
                "session_id": "json-bundle-session",
                "action_kind": "read",
                "target": {
                    "resource_kind": "file_path",
                    "resource_id": "/tmp/beater-os-runtime-json"
                },
                "inputs_summary": "observe sparse json bundle",
                "risk_class": "low",
                "human_explanation": "read-only sparse json runtime bundle"
            }]
        }))?;

        let Some(session) = bundle.session.as_ref() else {
            panic!("sparse json should include session");
        };
        assert_eq!(session.policy_profile, "default");
        assert!(session.initial_capability_ids.is_empty());
        assert_eq!(bundle.grants[0].expires_in_secs, DEFAULT_GRANT_TTL_SECS);
        assert_eq!(bundle.grants[0].delegation, DelegationMode::None);
        assert!(bundle.steps[0].expected_side_effects.is_empty());
        assert!(bundle.steps[0].required_grants.is_empty());
        assert!(bundle.steps[0].observation.is_none());
        Ok(())
    }
}
