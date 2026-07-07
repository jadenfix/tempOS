//! Durable single-writer runtime store for the local beaterOS daemon.
//!
//! This crate is the first `beater-osd` foundation slice: it moves durable
//! journal ownership behind a serialized runtime boundary without changing the
//! core contracts. The hot path is intentionally small and synchronous:
//! validate a session id, acquire one per-session lock, load and verify the
//! current chain, append one record, and release the lock.
//!
//! The lock is an atomic directory create under the store root. That keeps the
//! implementation portable to macOS without `unsafe` or platform-specific
//! syscalls. Lock acquisition is bounded by [`StoreOptions::lock_timeout`];
//! overload fails closed with [`DaemonError::LockTimeout`] rather than forking a
//! journal chain or waiting forever.

mod error;

use std::collections::{BTreeMap, BTreeSet};
use std::fs::{self, File, OpenOptions};
use std::io::Write;
use std::path::{Path, PathBuf};
use std::thread;
use std::time::{Duration, Instant};

use beater_os_core::{
    ActionKind, ActionManifest, AdmissionContext, AgentSession, ApprovalEvidence, Budget,
    CapabilityGrant, CapabilityReceipt, CapabilityReceiptInput, CapabilityScope, DecisionResult,
    DelegationMode, ExecutionLease, ExecutionLeaseHeartbeat, ExecutionLeaseReconciliation,
    HashValue, InMemoryJournal, JournalEvent, JournalRecord, JournalSnapshot, PaymentMandate,
    PolicyDecision, PolicyEngine, ReceiptLedger, ResourceKind, RiskClass, SessionStatus,
    SideEffectClass, SimulationEvidence, ToolManifest,
};
use beater_os_tool_registry::{
    RegisteredTool, RegistryPolicy, ResolveRequest, TestStatus, ToolRegistry, ToolTrust,
};
use chrono::{DateTime, TimeDelta, Utc};
use serde::{Deserialize, Serialize};

pub use crate::error::{DaemonError, DaemonResult};

const SESSIONS_DIR: &str = "sessions";
const JOURNAL_FILE: &str = "journal.jsonl";
const RECEIPTS_FILE: &str = "receipts.jsonl";
const TOOL_REGISTRY_FILE: &str = "tool-registry.json";
const TOOL_REGISTRY_LOCK: &str = "tool-registry.lock";
const LOCK_SUFFIX: &str = ".lock";
const EXECUTION_LEASE_OVERHEAD_GRACE_MS: u64 = 2_000;
const EXECUTION_LEASE_HEARTBEAT_WINDOW_MS: u64 = 5_000;
/// Policy contract version enforced by daemon-owned authority writes.
pub const DAEMON_POLICY_VERSION: &str = "beateros-policy-v0";

/// Runtime-store configuration.
#[derive(Debug, Clone)]
pub struct StoreOptions {
    /// Maximum time spent trying to acquire a per-session writer lock.
    pub lock_timeout: Duration,
    /// Sleep interval between bounded lock-acquire attempts.
    pub lock_poll_interval: Duration,
    /// Kernel-owned local tool registry used to ground daemon policy admission.
    pub tool_registry: BTreeMap<String, ToolManifest>,
    /// Deny actions whose tool is absent from [`Self::tool_registry`].
    pub require_registered_tools: bool,
}

impl Default for StoreOptions {
    fn default() -> Self {
        Self {
            lock_timeout: Duration::from_secs(2),
            lock_poll_interval: Duration::from_millis(2),
            tool_registry: default_tool_registry(),
            require_registered_tools: true,
        }
    }
}

fn default_tool_registry() -> BTreeMap<String, ToolManifest> {
    BTreeMap::from([
        tool_manifest(
            "fs.write",
            RiskClass::Low,
            [SideEffectClass::LocalWrite],
            false,
        ),
        tool_manifest("t", RiskClass::Low, [SideEffectClass::LocalWrite], false),
        tool_manifest("shell", RiskClass::Low, [], true),
        tool_manifest(
            "deployer",
            RiskClass::High,
            [SideEffectClass::Deployment],
            false,
        ),
        tool_manifest(
            "tool:test",
            RiskClass::Low,
            [SideEffectClass::LocalWrite],
            false,
        ),
        tool_manifest(
            "tool:deploy",
            RiskClass::High,
            [SideEffectClass::Deployment],
            false,
        ),
        tool_manifest(
            "tool:beater-osd-runtime",
            RiskClass::Low,
            [SideEffectClass::LocalWrite],
            false,
        ),
        tool_manifest(
            "tool:beater-os-runtime",
            RiskClass::Low,
            [SideEffectClass::None],
            false,
        ),
        tool_manifest(
            "tool:payment",
            RiskClass::High,
            [SideEffectClass::Payment],
            false,
        ),
    ])
}

fn tool_manifest(
    tool_id: &str,
    risk_class: RiskClass,
    side_effects: impl IntoIterator<Item = SideEffectClass>,
    sandbox_required: bool,
) -> (String, ToolManifest) {
    (
        tool_id.to_string(),
        ToolManifest {
            tool_id: tool_id.to_string(),
            publisher: "beater.local".to_string(),
            version: "1.0.0".to_string(),
            transport: "local".to_string(),
            required_capabilities: Vec::new(),
            side_effects: side_effects.into_iter().collect(),
            risk_class,
            sandbox_required,
        },
    )
}

/// Durable daemon-owned store for sessions, journals, and receipt ledgers.
#[derive(Debug, Clone)]
pub struct Store {
    root: PathBuf,
    options: StoreOptions,
}

/// Event-sourced read model for one session.
#[derive(Debug, Clone)]
pub struct SessionProjection {
    pub session: AgentSession,
    pub grants: Vec<CapabilityGrant>,
    pub revoked_handles: BTreeSet<String>,
    pub mandates: Vec<PaymentMandate>,
    pub manifests: Vec<ActionManifest>,
    pub decisions: Vec<PolicyDecision>,
    pub execution_leases: Vec<ExecutionLease>,
    pub execution_lease_heartbeats: Vec<ExecutionLeaseHeartbeat>,
    pub execution_reconciliations: Vec<ExecutionLeaseReconciliation>,
    pub approvals: Vec<ApprovalEvidence>,
    pub simulations: Vec<SimulationEvidence>,
    pub receipts: Vec<CapabilityReceipt>,
}

#[derive(Clone, Debug, Serialize, Deserialize, PartialEq, Eq)]
#[serde(rename_all = "snake_case")]
pub enum SchedulerExecutionLeaseStatus {
    LiveOpen,
    ExpiredRecoverable,
}

#[derive(Clone, Debug, Serialize, Deserialize)]
pub struct SchedulerOpenExecutionLease {
    pub action_id: String,
    pub lease_id: String,
    pub expires_at: DateTime<Utc>,
    pub status: SchedulerExecutionLeaseStatus,
}

#[derive(Clone, Debug)]
pub struct OpenExecutionLeasePreparation {
    pub projection: SessionProjection,
    pub lease: ExecutionLease,
}

#[derive(Clone, Debug, Serialize, Deserialize)]
pub struct SchedulerProjection {
    pub pending_allowed_action_ids: Vec<String>,
    pub runnable_pending_action_ids: Vec<String>,
    pub open_execution_lease_statuses: Vec<SchedulerOpenExecutionLease>,
    pub open_execution_lease_ids: Vec<String>,
    pub live_open_execution_lease_ids: Vec<String>,
    pub expired_recoverable_execution_lease_ids: Vec<String>,
    pub recovery_blocked: bool,
    pub admission_blocked: bool,
    pub admission_blockers: Vec<String>,
}

/// Consistent read-only export inputs for one live session trace.
#[derive(Debug, Clone)]
pub struct SessionTraceExport {
    pub projection: SessionProjection,
    pub journal: JournalSnapshot,
}

/// Exact local-shell tool version to persist in the daemon-owned tool registry.
#[derive(Clone, Debug)]
pub struct LocalShellToolRegistration {
    pub workspace_id: String,
    pub tool_id: String,
    pub version: String,
    pub content_digest: String,
    pub side_effects: BTreeSet<SideEffectClass>,
    pub risk_class: RiskClass,
}

impl SessionProjection {
    /// Grants that are active at `now`, projected from daemon-owned journal state.
    pub fn active_grants(&self, now: DateTime<Utc>) -> Vec<CapabilityGrant> {
        let grants_by_id: BTreeMap<&str, &CapabilityGrant> = self
            .grants
            .iter()
            .map(|grant| (grant.grant_id.as_str(), grant))
            .collect();
        self.grants
            .iter()
            .filter(|grant| {
                grant_effectively_active(grant, now, &self.revoked_handles, &grants_by_id)
            })
            .cloned()
            .collect()
    }

    /// Proposed manifest for `action_id`, if the daemon has journaled it.
    pub fn manifest(&self, action_id: &str) -> Option<&ActionManifest> {
        self.manifests
            .iter()
            .find(|manifest| manifest.action_id == action_id)
    }

    /// Latest policy decision for `action_id`, if one exists.
    pub fn latest_decision(&self, action_id: &str) -> Option<&PolicyDecision> {
        self.decisions
            .iter()
            .rev()
            .find(|decision| decision.action_id == action_id)
    }

    /// Scheduler-facing state derived from the daemon journal projection.
    ///
    /// Open execution leases are split into live and expired-recoverable states
    /// using the same `expires_at <= now` boundary as explicit reconciliation.
    /// Callers should wait or inspect the owning worker for live leases, and
    /// only attempt `outcome_unknown` reconciliation for expired recoverable
    /// leases.
    pub fn scheduler_projection(&self, now: DateTime<Utc>) -> SchedulerProjection {
        let closed_actions = self.closed_execution_actions();
        let open_execution_leases: BTreeMap<&str, &ExecutionLease> = self
            .execution_leases
            .iter()
            .filter(|lease| !closed_actions.contains(lease.action_id.as_str()))
            .map(|lease| (lease.action_id.as_str(), lease))
            .collect();
        let latest_decisions: BTreeMap<&str, bool> = self
            .decisions
            .iter()
            .map(|decision| {
                (
                    decision.action_id.as_str(),
                    decision.result == DecisionResult::Allowed,
                )
            })
            .collect();
        let pending_allowed_action_ids: Vec<String> = latest_decisions
            .iter()
            .filter(|(action_id, allowed)| **allowed && !closed_actions.contains(*action_id))
            .map(|(action_id, _)| (*action_id).to_string())
            .collect();
        let runnable_pending_action_ids: Vec<String> = pending_allowed_action_ids
            .iter()
            .filter(|action_id| !open_execution_leases.contains_key(action_id.as_str()))
            .cloned()
            .collect();
        let open_execution_lease_statuses: Vec<SchedulerOpenExecutionLease> = open_execution_leases
            .values()
            .map(|lease| SchedulerOpenExecutionLease {
                action_id: lease.action_id.clone(),
                lease_id: lease.lease_id.clone(),
                expires_at: lease.expires_at,
                status: if lease.expires_at <= now {
                    SchedulerExecutionLeaseStatus::ExpiredRecoverable
                } else {
                    SchedulerExecutionLeaseStatus::LiveOpen
                },
            })
            .collect();
        let open_execution_lease_ids: Vec<String> = open_execution_lease_statuses
            .iter()
            .map(|lease| lease.lease_id.clone())
            .collect();
        let live_open_execution_lease_ids: Vec<String> = open_execution_lease_statuses
            .iter()
            .filter(|lease| lease.status == SchedulerExecutionLeaseStatus::LiveOpen)
            .map(|lease| lease.lease_id.clone())
            .collect();
        let expired_recoverable_execution_lease_ids: Vec<String> = open_execution_lease_statuses
            .iter()
            .filter(|lease| lease.status == SchedulerExecutionLeaseStatus::ExpiredRecoverable)
            .map(|lease| lease.lease_id.clone())
            .collect();
        let recovery_blocked = !open_execution_lease_ids.is_empty();
        let mut admission_blockers = Vec::new();
        if self.session.status != SessionStatus::Running {
            admission_blockers.push(format!("session_status:{:?}", self.session.status));
        }
        if recovery_blocked {
            admission_blockers.push("open_execution_lease".to_string());
        }
        SchedulerProjection {
            pending_allowed_action_ids,
            runnable_pending_action_ids,
            open_execution_lease_statuses,
            open_execution_lease_ids,
            live_open_execution_lease_ids,
            expired_recoverable_execution_lease_ids,
            recovery_blocked,
            admission_blocked: !admission_blockers.is_empty(),
            admission_blockers,
        }
    }

    fn closed_execution_actions(&self) -> BTreeSet<&str> {
        let mut closed_actions: BTreeSet<&str> = self
            .receipts
            .iter()
            .map(|receipt| receipt.action_id.as_str())
            .collect();
        closed_actions.extend(
            self.execution_reconciliations
                .iter()
                .map(|reconciliation| reconciliation.action_id.as_str()),
        );
        closed_actions
    }
}

/// Result of a daemon-owned policy admission transaction.
#[derive(Debug, Clone)]
pub struct AdmissionOutcome {
    pub proposal_record: JournalRecord,
    pub decision_record: JournalRecord,
    pub decision: PolicyDecision,
    pub receipt_root_hash: HashValue,
}

/// Result of appending a receipt through the daemon boundary.
#[derive(Debug, Clone)]
pub struct ReceiptAppendOutcome {
    pub receipt_record: JournalRecord,
    pub receipt: CapabilityReceipt,
}

/// Result of issuing a durable execution lease through the daemon boundary.
#[derive(Debug, Clone)]
pub struct ExecutionLeaseOutcome {
    pub lease_record: JournalRecord,
    pub lease: ExecutionLease,
}

/// Daemon-owned request to claim an execution lease from already-admitted
/// journal state. Callers provide only compare-and-set identity fields; the
/// duplicated executable authority on [`ExecutionLease`] is copied from the
/// stored manifest and latest allowed decision under the session lock.
#[derive(Debug, Clone)]
pub struct ExecutionLeaseClaimRequest {
    pub lease_id: String,
    pub action_id: String,
    pub expected_manifest_hash: HashValue,
    pub expected_decision_id: String,
    pub expected_tool_version: String,
    pub expected_tool_digest: HashValue,
    pub initial_lease_ms: Option<u64>,
}

#[derive(Debug, Clone)]
pub struct ExecutionLeaseHeartbeatRequest {
    pub heartbeat_id: String,
    pub lease_id: String,
    pub action_id: String,
    pub expected_manifest_hash: HashValue,
    pub expected_decision_id: String,
    pub previous_expires_at: DateTime<Utc>,
    pub extend_by_ms: u64,
    pub observed_by: Option<String>,
    pub evidence_refs: Vec<String>,
}

#[derive(Debug, Clone)]
pub struct ExecutionLeaseHeartbeatOutcome {
    pub heartbeat_record: JournalRecord,
    pub heartbeat: ExecutionLeaseHeartbeat,
}

/// Store-owned projection of an admitted execute action that is currently
/// claimable by a scheduler worker.
#[derive(Debug, Clone, Serialize)]
pub struct ClaimableExecutionAction {
    pub session_id: String,
    pub action_id: String,
    pub manifest: ActionManifest,
    pub manifest_hash: HashValue,
    pub decision_id: String,
    pub tool_id: String,
    pub expected_tool_version: String,
    pub expected_tool_digest: HashValue,
    pub target: beater_os_core::CapabilitySelector,
    pub required_grants: BTreeSet<String>,
    pub requested_budget: Budget,
}

/// Durable lifecycle transition applied by the daemon store.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum SessionTransition {
    Pause,
    Resume,
    Cancel,
}

impl Store {
    /// Open a durable store with default bounded lock behavior.
    pub fn open(root: impl Into<PathBuf>) -> DaemonResult<Self> {
        Self::open_with_options(root, StoreOptions::default())
    }

    /// Open a durable store with explicit options.
    pub fn open_with_options(
        root: impl Into<PathBuf>,
        options: StoreOptions,
    ) -> DaemonResult<Self> {
        let root = root.into();
        fs::create_dir_all(root.join(SESSIONS_DIR))?;
        Ok(Self { root, options })
    }

    pub fn root(&self) -> &Path {
        &self.root
    }

    /// Load the daemon-owned durable tool registry.
    pub fn load_tool_registry(&self) -> DaemonResult<ToolRegistry> {
        self.with_registry_lock(|| self.load_tool_registry_unlocked())
    }

    /// Register or confirm an exact local-shell tool version in the durable
    /// registry, then allow it for one workspace.
    ///
    /// This is intentionally a store operation, not CLI-local state: the
    /// gateway receives a registry loaded from durable daemon-owned storage, and
    /// a conflicting version+digest fails closed rather than being silently
    /// replaced by the caller.
    pub fn register_local_shell_tool(
        &self,
        registration: LocalShellToolRegistration,
    ) -> DaemonResult<ToolRegistry> {
        self.with_registry_lock(|| {
            let mut registry = self.load_tool_registry_unlocked()?;
            match registry.get(&registration.tool_id, &registration.version) {
                Ok(existing) if existing.content_digest != registration.content_digest => {
                    return Err(DaemonError::Refused(format!(
                        "registered tool {}@{} has digest {}, not requested {}",
                        registration.tool_id,
                        registration.version,
                        existing.content_digest,
                        registration.content_digest
                    )));
                }
                Ok(_) => {}
                Err(beater_os_tool_registry::RegistryError::Unregistered { .. }) => {
                    registry.register(RegisteredTool {
                        manifest: ToolManifest {
                            tool_id: registration.tool_id.clone(),
                            publisher: "beaterosd.local".to_string(),
                            version: registration.version.clone(),
                            transport: "local_shell".to_string(),
                            required_capabilities: Vec::new(),
                            side_effects: registration.side_effects.clone(),
                            risk_class: registration.risk_class,
                            sandbox_required: true,
                        },
                        content_digest: registration.content_digest.clone(),
                        signature: None,
                        test_status: TestStatus::Passing,
                        trust: ToolTrust::Trusted,
                        registered_at: Utc::now(),
                        notes: "daemon-owned local shell registration".to_string(),
                    })?;
                }
                Err(err) => return Err(err.into()),
            }
            registry.pin(&registration.tool_id, &registration.version)?;
            registry.allow_workspace_tool(&registration.workspace_id, registration.tool_id);
            self.write_tool_registry_unlocked(&registry)?;
            Ok(registry)
        })
    }

    /// Whether a session exists and its journal genesis verifies.
    pub fn session_exists(&self, session_id: &str) -> DaemonResult<bool> {
        self.with_session_lock(session_id, || self.session_exists_unlocked(session_id))
    }

    /// List session ids with valid journal files, sorted for deterministic CLI output.
    pub fn list_sessions(&self) -> DaemonResult<Vec<String>> {
        let dir = self.root.join(SESSIONS_DIR);
        let mut out = Vec::new();
        if !dir.is_dir() {
            return Ok(out);
        }
        for entry in fs::read_dir(dir)? {
            let entry = entry?;
            let Some(name) = entry.file_name().to_str().map(str::to_string) else {
                continue;
            };
            if name.ends_with(LOCK_SUFFIX) || validate_session_id(&name).is_err() {
                continue;
            }
            if let Ok(true) = self.session_exists(&name) {
                out.push(name);
            }
        }
        out.sort();
        Ok(out)
    }

    /// Create a session and write its genesis journal record under the
    /// single-writer lock.
    pub fn create_session(&self, session: &AgentSession) -> DaemonResult<JournalRecord> {
        self.with_session_lock(&session.session_id, || {
            if self.session_exists_unlocked(&session.session_id)? {
                return Err(DaemonError::SessionExists(session.session_id.clone()));
            }
            let mut session = session.clone();
            session.status = SessionStatus::Running;
            fs::create_dir_all(self.session_dir(&session.session_id)?)?;
            File::create(self.journal_path(&session.session_id)?)?;
            File::create(self.receipts_path(&session.session_id)?)?;
            let mut journal = InMemoryJournal::new();
            let record = journal.append(
                JournalEvent::SessionCreated {
                    session: session.clone(),
                },
                session.created_at,
            )?;
            journal.verify_chain()?;
            self.write_journal_record_unlocked(&session.session_id, &record)?;
            Ok(record)
        })
    }

    /// Apply a daemon-owned session lifecycle transition under the per-session
    /// writer lock. Status changes are explicit journal events, not rewritten
    /// session snapshots, so the genesis authority object remains immutable.
    pub fn transition_session(
        &self,
        session_id: &str,
        transition: SessionTransition,
        created_at: DateTime<Utc>,
    ) -> DaemonResult<JournalRecord> {
        self.with_session_lock(session_id, || {
            let mut journal = self.load_journal_unlocked(session_id)?;
            let admission_state = admission_state_from_journal(session_id, &journal)?;
            let from = admission_state.session.status.clone();
            let to = next_session_status(transition, &from)?;
            if matches!(transition, SessionTransition::Resume)
                && !admission_state.open_execution_leases.is_empty()
            {
                return Err(open_execution_lease_refusal(
                    session_id,
                    &admission_state.open_execution_leases,
                    "resume session",
                ));
            }
            if matches!(transition, SessionTransition::Pause)
                && !admission_state.open_execution_leases.is_empty()
            {
                return Err(open_execution_lease_refusal(
                    session_id,
                    &admission_state.open_execution_leases,
                    "pause session",
                ));
            }
            if matches!(transition, SessionTransition::Cancel)
                && !admission_state.open_execution_leases.is_empty()
            {
                return Err(open_execution_lease_refusal(
                    session_id,
                    &admission_state.open_execution_leases,
                    "cancel session",
                ));
            }
            let transition_id = format!(
                "session:{session_id}:transition:{}",
                journal.records().len()
            );
            let record = journal.append(
                JournalEvent::SessionStatusChanged {
                    transition_id,
                    session_id: session_id.to_string(),
                    from,
                    to,
                },
                created_at,
            )?;
            journal.verify_chain()?;
            self.write_journal_record_unlocked(session_id, &record)?;
            Ok(record)
        })
    }

    /// Append one non-authority journal event while holding the per-session
    /// writer lock.
    ///
    /// Capability grants, action proposals, policy decisions, and receipts have
    /// dedicated daemon APIs because they form the admission and execution
    /// authority boundary.
    pub fn append_event(
        &self,
        session_id: &str,
        event: JournalEvent,
        created_at: DateTime<Utc>,
    ) -> DaemonResult<JournalRecord> {
        match event {
            JournalEvent::SessionCreated { .. } => {
                return Err(DaemonError::Refused(
                    "SessionCreated must be written through create_session".to_string(),
                ));
            }
            JournalEvent::SessionStatusChanged { .. } => {
                return Err(DaemonError::Refused(
                    "SessionStatusChanged must be written through transition_session".to_string(),
                ));
            }
            JournalEvent::CapabilityGranted { .. } => {
                return Err(DaemonError::Refused(
                    "CapabilityGranted must be written through issue_grant".to_string(),
                ));
            }
            JournalEvent::CapabilityRevoked { .. } => {
                return Err(DaemonError::Refused(
                    "CapabilityRevoked must be written through revoke_grant".to_string(),
                ));
            }
            JournalEvent::PaymentMandateIssued { .. } => {
                return Err(DaemonError::Refused(
                    "PaymentMandateIssued must be written through issue_payment_mandate"
                        .to_string(),
                ));
            }
            JournalEvent::ActionProposed { .. } => {
                return Err(DaemonError::Refused(
                    "ActionProposed must be written through admit_action".to_string(),
                ));
            }
            JournalEvent::ReceiptAppended { .. } => {
                return Err(DaemonError::Refused(
                    "ReceiptAppended must be written through append_receipt".to_string(),
                ));
            }
            JournalEvent::ExecutionLeaseIssued { .. } => {
                return Err(DaemonError::Refused(
                    "ExecutionLeaseIssued must be written through execute_and_append_receipt"
                        .to_string(),
                ));
            }
            JournalEvent::ExecutionLeaseHeartbeated { .. } => {
                return Err(DaemonError::Refused(
                    "ExecutionLeaseHeartbeated must be written through heartbeat_execution_lease"
                        .to_string(),
                ));
            }
            JournalEvent::ExecutionLeaseReconciled { .. } => {
                return Err(DaemonError::Refused(
                    "ExecutionLeaseReconciled must be written through reconcile_execution_lease"
                        .to_string(),
                ));
            }
            JournalEvent::PolicyDecided { .. } => {
                return Err(DaemonError::Refused(
                    "PolicyDecided must be written through admit_action".to_string(),
                ));
            }
            JournalEvent::ApprovalRecorded { .. } => {
                return Err(DaemonError::Refused(
                    "ApprovalRecorded must be written through record_approval".to_string(),
                ));
            }
            JournalEvent::SimulationRecorded { .. } => {
                return Err(DaemonError::Refused(
                    "SimulationRecorded must be written through record_simulation".to_string(),
                ));
            }
            _ => {}
        }
        self.with_session_lock(session_id, || {
            self.append_event_unlocked(session_id, event, created_at)
        })
    }

    /// Record action-bound human approval evidence through the daemon boundary.
    pub fn record_approval(
        &self,
        session_id: &str,
        approval: ApprovalEvidence,
        created_at: DateTime<Utc>,
    ) -> DaemonResult<JournalRecord> {
        self.with_session_lock(session_id, || {
            let mut journal = self.load_journal_unlocked(session_id)?;
            let admission_state = admission_state_from_journal(session_id, &journal)?;
            ensure_session_running(&admission_state.session)?;
            validate_approval_evidence(&admission_state, &approval)?;
            let record = journal.append(JournalEvent::ApprovalRecorded { approval }, created_at)?;
            journal.verify_chain()?;
            self.write_journal_record_unlocked(session_id, &record)?;
            Ok(record)
        })
    }

    /// Record action-bound passed simulation evidence through the daemon boundary.
    pub fn record_simulation(
        &self,
        session_id: &str,
        simulation: SimulationEvidence,
        created_at: DateTime<Utc>,
    ) -> DaemonResult<JournalRecord> {
        self.with_session_lock(session_id, || {
            let mut journal = self.load_journal_unlocked(session_id)?;
            let admission_state = admission_state_from_journal(session_id, &journal)?;
            ensure_session_running(&admission_state.session)?;
            validate_simulation_evidence(&admission_state, &simulation)?;
            let record =
                journal.append(JournalEvent::SimulationRecorded { simulation }, created_at)?;
            journal.verify_chain()?;
            self.write_journal_record_unlocked(session_id, &record)?;
            Ok(record)
        })
    }

    /// Issue a capability grant through the daemon-owned grant boundary.
    pub fn issue_grant(
        &self,
        session_id: &str,
        grant: CapabilityGrant,
        created_at: DateTime<Utc>,
    ) -> DaemonResult<JournalRecord> {
        self.with_session_lock(session_id, || {
            if grant.session_id != session_id {
                return Err(DaemonError::Refused(format!(
                    "grant session {} does not match daemon session {session_id}",
                    grant.session_id
                )));
            }
            if grant.policy_version != DAEMON_POLICY_VERSION {
                return Err(DaemonError::Refused(format!(
                    "grant {} uses unsupported policy version {}",
                    grant.grant_id, grant.policy_version
                )));
            }
            let mut journal = self.load_journal_unlocked(session_id)?;
            let admission_state = admission_state_from_journal(session_id, &journal)?;
            ensure_session_running(&admission_state.session)?;
            if grant.holder != admission_state.session.agent_id {
                return Err(DaemonError::Refused(format!(
                    "grant {} holder {} does not match session agent {}",
                    grant.grant_id, grant.holder, admission_state.session.agent_id
                )));
            }
            if admission_state.grants.contains_key(grant.grant_id.as_str()) {
                return Err(DaemonError::Refused(format!(
                    "grant {} was already issued",
                    grant.grant_id
                )));
            }
            let grant = normalize_grant_file_authority(grant)?;
            if grant.revocation_handle == grant.grant_id {
                return Err(DaemonError::Refused(format!(
                    "grant {} revocation handle must not equal the grant id",
                    grant.grant_id
                )));
            }
            if admission_state.event_ids.contains(&grant.revocation_handle) {
                return Err(DaemonError::Refused(format!(
                    "grant {} revocation handle {} collides with an existing journal event id",
                    grant.grant_id, grant.revocation_handle
                )));
            }
            if admission_state
                .issued_revocation_handles
                .contains(&grant.revocation_handle)
            {
                return Err(DaemonError::Refused(format!(
                    "grant {} revocation handle {} was already issued",
                    grant.grant_id, grant.revocation_handle
                )));
            }
            validate_grant_authority(&admission_state, &grant)?;
            let record = journal.append(JournalEvent::CapabilityGranted { grant }, created_at)?;
            journal.verify_chain()?;
            self.write_journal_record_unlocked(session_id, &record)?;
            Ok(record)
        })
    }

    /// Revoke an issued grant by resolving its daemon-stored revocation handle
    /// and appending a `CapabilityRevoked` event. The caller supplies a grant id,
    /// not a handle, so revocation cannot conjure authority for a fake handle.
    pub fn revoke_grant(
        &self,
        session_id: &str,
        grant_id: &str,
        revoked_by: impl Into<String>,
        reason: impl Into<String>,
        created_at: DateTime<Utc>,
    ) -> DaemonResult<JournalRecord> {
        self.with_session_lock(session_id, || {
            let mut journal = self.load_journal_unlocked(session_id)?;
            let admission_state = admission_state_from_journal(session_id, &journal)?;
            let Some(grant) = admission_state.grants.get(grant_id) else {
                return Err(DaemonError::Refused(format!(
                    "grant {grant_id} has not been issued in session {session_id}"
                )));
            };
            if admission_state
                .revoked_handles
                .contains(&grant.revocation_handle)
            {
                return Err(DaemonError::Refused(format!(
                    "grant {grant_id} is already revoked"
                )));
            }
            let revoked_by = revoked_by.into();
            if revoked_by.trim().is_empty() {
                return Err(DaemonError::Refused(
                    "revocation actor must not be empty".to_string(),
                ));
            }
            let reason = reason.into();
            if reason.trim().is_empty() {
                return Err(DaemonError::Refused(
                    "revocation reason must not be empty".to_string(),
                ));
            }
            let record = journal.append(
                JournalEvent::CapabilityRevoked {
                    grant_id: grant.grant_id.clone(),
                    revocation_handle: grant.revocation_handle.clone(),
                    revoked_by,
                    reason,
                },
                created_at,
            )?;
            journal.verify_chain()?;
            self.write_journal_record_unlocked(session_id, &record)?;
            Ok(record)
        })
    }

    /// Reconcile an expired open execution lease without creating a receipt.
    ///
    /// This closes the daemon recovery blocker for a side-effect outcome that
    /// remains unknown. It does not prove success, failure, or absence of side
    /// effects, and it does not make the action executable again.
    pub fn reconcile_execution_lease(
        &self,
        session_id: &str,
        mut reconciliation: ExecutionLeaseReconciliation,
        _created_at: DateTime<Utc>,
    ) -> DaemonResult<JournalRecord> {
        self.with_session_lock(session_id, || {
            if reconciliation.session_id != session_id {
                return Err(DaemonError::Refused(format!(
                    "execution lease reconciliation {} is bound to session {}, not {session_id}",
                    reconciliation.reconciliation_id, reconciliation.session_id
                )));
            }
            let mut journal = self.load_journal_unlocked(session_id)?;
            let admission_state = admission_state_from_journal(session_id, &journal)?;
            if !matches!(
                admission_state.session.status,
                SessionStatus::Running | SessionStatus::Paused
            ) {
                return Err(DaemonError::Refused(format!(
                    "session {} is not reconcilable (status {:?})",
                    admission_state.session.session_id, admission_state.session.status
                )));
            }
            let Some(open_lease) = admission_state
                .open_execution_leases
                .get(&reconciliation.action_id)
            else {
                return Err(DaemonError::Refused(format!(
                    "action {} has no open execution lease to reconcile",
                    reconciliation.action_id
                )));
            };
            if open_lease.lease_id != reconciliation.lease_id {
                return Err(DaemonError::Refused(format!(
                    "action {} open lease is {}, not {}",
                    reconciliation.action_id, open_lease.lease_id, reconciliation.lease_id
                )));
            }
            if admission_state
                .reconciled_execution_actions
                .contains_key(&reconciliation.action_id)
            {
                return Err(DaemonError::Refused(format!(
                    "action {} already has an execution lease reconciliation",
                    reconciliation.action_id
                )));
            }
            let now = Utc::now();
            if now < open_lease.expires_at {
                return Err(DaemonError::Refused(format!(
                    "execution lease {} is still live until {}",
                    open_lease.lease_id,
                    open_lease.expires_at.to_rfc3339()
                )));
            }
            reconciliation.manifest_hash = open_lease.manifest_hash.clone();
            reconciliation.decision_id = open_lease.decision_id.clone();
            reconciliation.reconciled_at = now;
            let record = journal.append(
                JournalEvent::ExecutionLeaseReconciled { reconciliation },
                now,
            )?;
            journal.verify_chain()?;
            self.write_journal_record_unlocked(session_id, &record)?;
            Ok(record)
        })
    }

    /// Issue bounded economic authority through the daemon-owned payment boundary.
    pub fn issue_payment_mandate(
        &self,
        session_id: &str,
        mandate: PaymentMandate,
        created_at: DateTime<Utc>,
    ) -> DaemonResult<JournalRecord> {
        validate_session_id(session_id)?;
        self.with_session_lock(session_id, || {
            let mut journal = self.load_journal_unlocked(session_id)?;
            let admission_state = admission_state_from_journal(session_id, &journal)?;
            ensure_session_running(&admission_state.session)?;
            if mandate.session_id != session_id {
                return Err(DaemonError::Refused(format!(
                    "payment mandate {} is bound to session {}, not {session_id}",
                    mandate.mandate_id, mandate.session_id
                )));
            }
            if mandate.holder != admission_state.session.agent_id {
                return Err(DaemonError::Refused(format!(
                    "payment mandate {} holder {} does not match session agent {}",
                    mandate.mandate_id, mandate.holder, admission_state.session.agent_id
                )));
            }
            if mandate.issuer.trim().is_empty() {
                return Err(DaemonError::Refused(format!(
                    "payment mandate {} issuer must not be empty",
                    mandate.mandate_id
                )));
            }
            if admission_state.mandates.contains_key(&mandate.mandate_id) {
                return Err(DaemonError::Refused(format!(
                    "payment mandate {} was already issued",
                    mandate.mandate_id
                )));
            }
            let record =
                journal.append(JournalEvent::PaymentMandateIssued { mandate }, created_at)?;
            journal.verify_chain()?;
            self.write_journal_record_unlocked(session_id, &record)?;
            Ok(record)
        })
    }

    /// Admit an action through the daemon-owned policy path and append the
    /// proposal plus policy decision under one per-session writer lock.
    pub fn admit_action(
        &self,
        session_id: &str,
        manifest: ActionManifest,
    ) -> DaemonResult<AdmissionOutcome> {
        self.admit_action_with_revoked_handles(session_id, manifest, BTreeSet::new())
    }

    /// Admit an action through the daemon-owned policy path while applying the
    /// durable journal-projected revocation registry plus an optional
    /// operator-supplied registry snapshot.
    ///
    /// Revocation handles are live external evidence: the CLI or daemon front
    /// end may receive a monotonic registry epoch from an operator, but the
    /// daemon remains the single authority that projects grants, builds the
    /// admission context, and journals the proposal/decision pair.
    pub fn admit_action_with_revoked_handles(
        &self,
        session_id: &str,
        manifest: ActionManifest,
        external_revoked_handles: BTreeSet<String>,
    ) -> DaemonResult<AdmissionOutcome> {
        self.with_session_lock(session_id, || {
            if manifest.session_id != session_id {
                return Err(DaemonError::Refused(format!(
                    "manifest session {} does not match daemon session {session_id}",
                    manifest.session_id
                )));
            }
            let mut journal = self.load_journal_unlocked(session_id)?;
            let admission_state = admission_state_from_journal(session_id, &journal)?;
            ensure_session_running(&admission_state.session)?;
            if !admission_state.open_execution_leases.is_empty() {
                return Err(open_execution_lease_refusal(
                    session_id,
                    &admission_state.open_execution_leases,
                    "admit new action",
                ));
            }
            let existing_proposal = admission_state.proposals.get(manifest.action_id.as_str());
            if let Some(decision) = admission_state
                .latest_decisions
                .get(manifest.action_id.as_str())
                && decision.result == DecisionResult::Allowed
            {
                return Err(DaemonError::Refused(format!(
                    "action {} already has an allowed policy decision",
                    manifest.action_id
                )));
            }
            if let Some(proposal) = existing_proposal
                && proposal.manifest != manifest
            {
                return Err(DaemonError::Refused(format!(
                    "action {} was already proposed with a different manifest",
                    manifest.action_id
                )));
            }
            if admission_state
                .receipted_actions
                .contains(&manifest.action_id)
            {
                return Err(DaemonError::Refused(format!(
                    "action {} already has a receipt and cannot be re-admitted",
                    manifest.action_id
                )));
            }
            if admission_state
                .reconciled_execution_actions
                .contains_key(&manifest.action_id)
            {
                return Err(DaemonError::Refused(format!(
                    "action {} has outcome-unknown reconciliation and cannot be re-admitted",
                    manifest.action_id
                )));
            }

            let now = Utc::now();
            let payment_reserved_by_mandate = payment_reserved_by_mandate_excluding(
                &admission_state,
                manifest.action_id.as_str(),
            );
            let session_budget_used =
                runtime_budget_used_excluding(&admission_state, manifest.action_id.as_str());
            let mut revoked_handles = admission_state.revoked_handles;
            revoked_handles.extend(external_revoked_handles);
            let ctx = AdmissionContext {
                now,
                actor_id: admission_state.session.agent_id,
                session_id: admission_state.session.session_id,
                policy_version: DAEMON_POLICY_VERSION.to_string(),
                session_budget: admission_state.session.budget,
                session_budget_used,
                grants: admission_state.grants.into_values().collect(),
                approvals: admission_state.approvals,
                simulations: admission_state.simulations,
                mandates: admission_state.mandates.into_values().collect(),
                payment_reserved_by_mandate,
                revoked_handles,
                tool_registry: self.options.tool_registry.clone(),
                require_registered_tools: self.options.require_registered_tools,
            };
            let decision = PolicyEngine::new().admit(&manifest, &ctx)?;
            let mut records_to_write = Vec::new();
            let proposal_record = if let Some(proposal) = existing_proposal {
                proposal.record.clone()
            } else {
                let record = journal.append(
                    JournalEvent::ActionProposed {
                        manifest: Box::new(manifest),
                    },
                    now,
                )?;
                records_to_write.push(record.clone());
                record
            };
            let decision_record = journal.append(
                JournalEvent::PolicyDecided {
                    decision: decision.clone(),
                },
                now,
            )?;
            records_to_write.push(decision_record.clone());
            journal.verify_chain()?;
            let receipt_root_hash = self
                .receipt_ledger_from_journal_unlocked(session_id)?
                .root_hash();
            self.write_journal_records_unlocked(session_id, records_to_write.iter())?;
            Ok(AdmissionOutcome {
                proposal_record,
                decision_record,
                decision,
                receipt_root_hash,
            })
        })
    }

    /// Build and persist a receipt under the same per-session writer lock as the
    /// receipt journal event. Journal-first ordering is preserved: the
    /// `ReceiptAppended` event is written before the receipt-ledger line.
    pub fn append_receipt(
        &self,
        session_id: &str,
        input: CapabilityReceiptInput,
        created_at: DateTime<Utc>,
    ) -> DaemonResult<CapabilityReceipt> {
        Ok(self
            .append_receipt_with_record(session_id, input, created_at)?
            .receipt)
    }

    /// Build and persist a receipt and return the exact `ReceiptAppended`
    /// journal record that made it durable.
    pub fn append_receipt_with_record(
        &self,
        session_id: &str,
        input: CapabilityReceiptInput,
        _created_at: DateTime<Utc>,
    ) -> DaemonResult<ReceiptAppendOutcome> {
        self.with_session_lock(session_id, || {
            let journal = self.load_journal_unlocked(session_id)?;
            let projection = project_journal(session_id, &journal)?;
            ensure_session_running(&projection.session)?;
            let admission_state = admission_state_from_journal(session_id, &journal)?;
            if let Some(open_lease) = admission_state.open_execution_leases.get(&input.action_id)
            {
                return Err(DaemonError::Refused(format!(
                    "receipt for action {} must complete execution lease {} through lease-bound completion",
                    input.action_id, open_lease.lease_id
                )));
            }
            let mut ledger = self.receipt_ledger_from_journal_unlocked(session_id)?;
            let receipt = ledger.append(input)?;
            let receipt_record = self.append_event_unlocked(
                session_id,
                JournalEvent::ReceiptAppended {
                    receipt: receipt.clone(),
                },
                Utc::now(),
            )?;
            Ok(ReceiptAppendOutcome {
                receipt_record,
                receipt,
            })
        })
    }

    /// Atomically claim a latest-allowed execute action by appending a durable
    /// execution lease without running the tool inline. The same journal replay
    /// invariants as [`Self::execute_and_append_receipt`] are enforced under
    /// the session lock, so stale scheduler workers cannot claim already
    /// receipted, reconciled, denied, or already-leased actions.
    pub fn claim_execution_lease(
        &self,
        session_id: &str,
        lease: ExecutionLease,
        _created_at: DateTime<Utc>,
    ) -> DaemonResult<ExecutionLeaseOutcome> {
        self.with_session_lock(session_id, || {
            let mut journal = self.load_journal_unlocked(session_id)?;
            let outcome =
                self.append_execution_lease_claim_unlocked(session_id, &mut journal, lease)?;
            Ok(outcome)
        })
    }

    /// Atomically claim a latest-allowed execute action by deriving the lease
    /// from daemon-owned admission state instead of accepting duplicated
    /// authority fields from the caller.
    pub fn claim_execution_lease_from_admission(
        &self,
        session_id: &str,
        request: ExecutionLeaseClaimRequest,
        _created_at: DateTime<Utc>,
    ) -> DaemonResult<ExecutionLeaseOutcome> {
        self.with_session_lock(session_id, || {
            if request.lease_id.trim().is_empty() {
                return Err(DaemonError::invalid("lease_id", request.lease_id));
            }
            if request.action_id.trim().is_empty() {
                return Err(DaemonError::invalid("action_id", request.action_id));
            }
            let mut journal = self.load_journal_unlocked(session_id)?;
            let projection = project_journal(session_id, &journal)?;
            let admission_state = admission_state_from_journal(session_id, &journal)?;
            let decision = admission_state
                .latest_decisions
                .get(&request.action_id)
                .ok_or_else(|| {
                    DaemonError::Refused(format!(
                        "action {} has no policy decision for execution lease",
                        request.action_id
                    ))
                })?;
            if decision.decision_id != request.expected_decision_id {
                return Err(DaemonError::Refused(format!(
                    "action {} latest decision {} does not match expected decision {}",
                    request.action_id, decision.decision_id, request.expected_decision_id
                )));
            }
            if decision.manifest_hash != request.expected_manifest_hash {
                return Err(DaemonError::Refused(format!(
                    "action {} latest decision manifest hash does not match expected manifest hash",
                    request.action_id
                )));
            }
            let proposal = admission_state
                .proposals
                .get(&request.action_id)
                .ok_or_else(|| {
                    DaemonError::Refused(format!(
                        "action {} has no proposed manifest for execution lease",
                        request.action_id
                    ))
                })?;
            let manifest_hash = proposal.manifest.digest().map_err(DaemonError::from)?;
            if manifest_hash != request.expected_manifest_hash {
                return Err(DaemonError::Refused(format!(
                    "action {} stored manifest digest does not match expected manifest hash",
                    request.action_id
                )));
            }
            if proposal.manifest.action_kind != ActionKind::Execute {
                return Err(DaemonError::Refused(format!(
                    "action {} is not an execute action and cannot receive a scheduler execution lease",
                    request.action_id
                )));
            }
            let requested_wall_ms = proposal.manifest.requested_budget.max_wall_ms.ok_or_else(
                || {
                    DaemonError::Refused(format!(
                        "action {} must declare finite requested wall budget for execution lease",
                        request.action_id
                    ))
                },
            )?;
            let lease_wall_ms = requested_wall_ms
                .checked_add(EXECUTION_LEASE_OVERHEAD_GRACE_MS)
                .ok_or_else(|| {
                    DaemonError::Refused(format!(
                        "action {} execution lease duration overflowed requested wall budget",
                        request.action_id
                    ))
                })?;
            let lease_issued_at = Utc::now();
            let initial_lease_wall_ms = match request.initial_lease_ms {
                Some(0) => {
                    return Err(DaemonError::Refused(
                        "initial_lease_ms must be greater than zero".to_string(),
                    ));
                }
                Some(initial_lease_ms) if initial_lease_ms > lease_wall_ms => {
                    return Err(DaemonError::Refused(format!(
                        "initial_lease_ms must be at most the action wall budget plus daemon grace ({lease_wall_ms} ms)"
                    )));
                }
                Some(initial_lease_ms) => initial_lease_ms,
                None => lease_wall_ms,
            };
            let lease_expires_at = lease_issued_at
                .checked_add_signed(TimeDelta::milliseconds(
                    i64::try_from(initial_lease_wall_ms).map_err(|_| {
                        DaemonError::Refused(format!(
                            "action {} execution lease duration cannot fit signed milliseconds",
                            request.action_id
                        ))
                    })?,
                ))
                .ok_or_else(|| {
                    DaemonError::Refused(format!(
                        "action {} execution lease expiration overflowed daemon time",
                        request.action_id
                    ))
                })?;
            let target = proposal
                .manifest
                .resolved_target
                .clone()
                .unwrap_or_else(|| proposal.manifest.target.clone());
            let registry = self.load_tool_registry_unlocked()?;
            let registered_tool = registry.resolve(
                &ResolveRequest::new(
                    proposal.manifest.tool_id.clone(),
                    request.expected_tool_version,
                )
                .in_workspace(projection.session.workspace_id)
                .expecting_digest(request.expected_tool_digest),
            )?;
            let tool_ref = format!(
                "{}@{}#{}",
                registered_tool.manifest.tool_id,
                registered_tool.manifest.version,
                registered_tool.content_digest
            );
            let lease = ExecutionLease {
                lease_id: request.lease_id,
                session_id: session_id.to_string(),
                action_id: request.action_id,
                manifest_hash: decision.manifest_hash.clone(),
                decision_id: decision.decision_id.clone(),
                tool_id: proposal.manifest.tool_id.clone(),
                tool_ref,
                target,
                required_grants: proposal.manifest.required_grants.clone(),
                requested_budget: proposal.manifest.requested_budget.clone(),
                leased_at: lease_issued_at,
                expires_at: lease_expires_at,
            };
            self.append_execution_lease_claim_unlocked(session_id, &mut journal, lease)
        })
    }

    /// Journal a live worker heartbeat and extend the open lease by a bounded
    /// window, never beyond the original action wall-clock budget plus daemon
    /// grace. Heartbeats are liveness evidence only: they do not complete the
    /// action, create receipts, or make expired leases recoverable again.
    pub fn heartbeat_execution_lease(
        &self,
        session_id: &str,
        request: ExecutionLeaseHeartbeatRequest,
        _created_at: DateTime<Utc>,
    ) -> DaemonResult<ExecutionLeaseHeartbeatOutcome> {
        self.with_session_lock(session_id, || {
            if request.heartbeat_id.trim().is_empty() {
                return Err(DaemonError::invalid("heartbeat_id", request.heartbeat_id));
            }
            if request.lease_id.trim().is_empty() {
                return Err(DaemonError::invalid("lease_id", request.lease_id));
            }
            if request.action_id.trim().is_empty() {
                return Err(DaemonError::invalid("action_id", request.action_id));
            }
            if request.extend_by_ms == 0
                || request.extend_by_ms > EXECUTION_LEASE_HEARTBEAT_WINDOW_MS
            {
                return Err(DaemonError::Refused(format!(
                    "extend_by_ms must be between 1 and {EXECUTION_LEASE_HEARTBEAT_WINDOW_MS}"
                )));
            }
            if request
                .evidence_refs
                .iter()
                .any(|reference| reference.trim().is_empty())
            {
                return Err(DaemonError::Refused(
                    "heartbeat evidence_refs must not contain empty references".to_string(),
                ));
            }
            let journal = self.load_journal_unlocked(session_id)?;
            let projection = project_journal(session_id, &journal)?;
            ensure_session_running(&projection.session)?;
            let admission_state = admission_state_from_journal(session_id, &journal)?;
            let open_lease = admission_state
                .open_execution_leases
                .get(&request.action_id)
                .ok_or_else(|| {
                    DaemonError::Refused(format!(
                        "action {} has no open execution lease",
                        request.action_id
                    ))
                })?;
            if open_lease.lease_id != request.lease_id {
                return Err(DaemonError::Refused(format!(
                    "heartbeat lease {} does not match open lease {} for action {}",
                    request.lease_id, open_lease.lease_id, request.action_id
                )));
            }
            if open_lease.manifest_hash != request.expected_manifest_hash {
                return Err(DaemonError::Refused(format!(
                    "heartbeat manifest hash does not match open lease {}",
                    open_lease.lease_id
                )));
            }
            if open_lease.decision_id != request.expected_decision_id {
                return Err(DaemonError::Refused(format!(
                    "heartbeat decision id does not match open lease {}",
                    open_lease.lease_id
                )));
            }
            if open_lease.expires_at != request.previous_expires_at {
                return Err(DaemonError::Refused(format!(
                    "heartbeat previous_expires_at {} does not match open lease {} expiration {}",
                    request.previous_expires_at, open_lease.lease_id, open_lease.expires_at
                )));
            }
            let now = Utc::now();
            if now >= open_lease.expires_at {
                return Err(DaemonError::Refused(format!(
                    "execution lease {} expired at {} and cannot be heartbeated",
                    open_lease.lease_id, open_lease.expires_at
                )));
            }
            let requested_wall_ms = open_lease.requested_budget.max_wall_ms.ok_or_else(|| {
                DaemonError::Refused(format!(
                    "execution lease {} has no finite wall budget for heartbeat renewal",
                    open_lease.lease_id
                ))
            })?;
            let max_wall_ms = requested_wall_ms
                .checked_add(EXECUTION_LEASE_OVERHEAD_GRACE_MS)
                .ok_or_else(|| {
                    DaemonError::Refused(format!(
                        "execution lease {} heartbeat budget overflowed",
                        open_lease.lease_id
                    ))
                })?;
            let max_expires_at = open_lease
                .leased_at
                .checked_add_signed(TimeDelta::milliseconds(
                    i64::try_from(max_wall_ms).map_err(|_| {
                        DaemonError::Refused(format!(
                            "execution lease {} heartbeat budget cannot fit signed milliseconds",
                            open_lease.lease_id
                        ))
                    })?,
                ))
                .ok_or_else(|| {
                    DaemonError::Refused(format!(
                        "execution lease {} heartbeat maximum expiration overflowed daemon time",
                        open_lease.lease_id
                    ))
                })?;
            let requested_expires_at = now
                .checked_add_signed(TimeDelta::milliseconds(
                    i64::try_from(request.extend_by_ms).map_err(|_| {
                        DaemonError::Refused(format!(
                            "execution lease {} heartbeat extension cannot fit signed milliseconds",
                            open_lease.lease_id
                        ))
                    })?,
                ))
                .ok_or_else(|| {
                    DaemonError::Refused(format!(
                        "execution lease {} heartbeat expiration overflowed daemon time",
                        open_lease.lease_id
                    ))
                })?;
            let extended_expires_at = requested_expires_at.min(max_expires_at);
            if extended_expires_at <= open_lease.expires_at {
                return Err(DaemonError::Refused(format!(
                    "execution lease {} heartbeat would not extend the current expiration",
                    open_lease.lease_id
                )));
            }
            let observed_by = request
                .observed_by
                .filter(|value| !value.trim().is_empty())
                .unwrap_or_else(|| projection.session.created_by.clone());
            let heartbeat = ExecutionLeaseHeartbeat {
                heartbeat_id: request.heartbeat_id,
                lease_id: open_lease.lease_id.clone(),
                session_id: session_id.to_string(),
                action_id: open_lease.action_id.clone(),
                manifest_hash: open_lease.manifest_hash.clone(),
                decision_id: open_lease.decision_id.clone(),
                previous_expires_at: open_lease.expires_at,
                extended_expires_at,
                observed_by,
                evidence_refs: request.evidence_refs,
                heartbeat_at: now,
            };
            let heartbeat_record = self.append_event_unlocked(
                session_id,
                JournalEvent::ExecutionLeaseHeartbeated {
                    heartbeat: heartbeat.clone(),
                },
                now,
            )?;
            Ok(ExecutionLeaseHeartbeatOutcome {
                heartbeat_record,
                heartbeat,
            })
        })
    }

    /// Return execute actions that can be claimed without re-deriving authority
    /// in an HTTP or worker adapter.
    ///
    /// The projection is intentionally store-owned: it is computed under the
    /// same session lock and registry resolution rules as
    /// [`Self::claim_execution_lease_from_admission`]. Entries are included only
    /// when the latest decision is allowed, the action is unreceipted,
    /// unreconciled, not already leased, has a finite wall budget, and resolves
    /// to the daemon registry's current pinned tool version/digest.
    pub fn claimable_execution_actions(
        &self,
        session_id: &str,
    ) -> DaemonResult<Vec<ClaimableExecutionAction>> {
        self.with_session_lock(session_id, || {
            let journal = self.load_journal_unlocked(session_id)?;
            let projection = project_journal(session_id, &journal)?;
            ensure_session_running(&projection.session)?;
            let admission_state = admission_state_from_journal(session_id, &journal)?;
            if !admission_state.open_execution_leases.is_empty() {
                return Ok(Vec::new());
            }
            let registry = self.load_tool_registry_unlocked()?;
            let mut actions = Vec::new();
            for manifest in projection.manifests {
                if manifest.action_kind != ActionKind::Execute {
                    continue;
                }
                if admission_state
                    .receipted_actions
                    .contains(&manifest.action_id)
                    || admission_state
                        .reconciled_execution_actions
                        .contains_key(&manifest.action_id)
                    || admission_state
                        .open_execution_leases
                        .contains_key(&manifest.action_id)
                    || manifest.requested_budget.max_wall_ms.is_none()
                {
                    continue;
                }
                let Some(decision) = admission_state.latest_decisions.get(&manifest.action_id)
                else {
                    continue;
                };
                if decision.result != DecisionResult::Allowed {
                    continue;
                }
                let manifest_hash = manifest.digest().map_err(DaemonError::from)?;
                if manifest_hash != decision.manifest_hash {
                    return Err(DaemonError::Refused(format!(
                        "action {} manifest hash no longer matches latest decision {}",
                        manifest.action_id, decision.decision_id
                    )));
                }
                let Some(pin) = registry.pin_for(&manifest.tool_id) else {
                    continue;
                };
                let registered_tool = registry.resolve(
                    &ResolveRequest::new(manifest.tool_id.clone(), pin.version.clone())
                        .in_workspace(projection.session.workspace_id.clone())
                        .expecting_digest(pin.content_digest.clone()),
                )?;
                let target = manifest
                    .resolved_target
                    .clone()
                    .unwrap_or_else(|| manifest.target.clone());
                actions.push(ClaimableExecutionAction {
                    session_id: session_id.to_string(),
                    action_id: manifest.action_id.clone(),
                    manifest: manifest.clone(),
                    manifest_hash,
                    decision_id: decision.decision_id.clone(),
                    tool_id: manifest.tool_id.clone(),
                    expected_tool_version: registered_tool.manifest.version.clone(),
                    expected_tool_digest: registered_tool.content_digest.clone(),
                    target,
                    required_grants: manifest.required_grants.clone(),
                    requested_budget: manifest.requested_budget.clone(),
                });
            }
            Ok(actions)
        })
    }

    /// Complete a previously claimed execution lease with a receipt. The lease
    /// must still be open and match `lease_id`; core journal causality verifies
    /// timing, target, tool, input digest, side-effect, and receipt-chain
    /// compatibility before the append is accepted. Completion intentionally
    /// does not require the session to still be running: the lease is durable
    /// proof that execution authority was acquired while the session was live,
    /// and pause/cancel after lease start must not block a terminal receipt for
    /// already-started side effects.
    pub fn append_receipt_for_execution_lease(
        &self,
        session_id: &str,
        lease_id: &str,
        input: CapabilityReceiptInput,
        _created_at: DateTime<Utc>,
    ) -> DaemonResult<ReceiptAppendOutcome> {
        self.with_session_lock(session_id, || {
            let journal = self.load_journal_unlocked(session_id)?;
            let admission_state = admission_state_from_journal(session_id, &journal)?;
            let open_lease = admission_state
                .open_execution_leases
                .values()
                .find(|lease| lease.lease_id == lease_id)
                .ok_or_else(|| {
                    DaemonError::Refused(format!("execution lease {lease_id} is not open"))
                })?;
            if open_lease.action_id != input.action_id {
                return Err(DaemonError::Refused(format!(
                    "receipt for action {} does not match execution lease {} action {}",
                    input.action_id, lease_id, open_lease.action_id
                )));
            }
            let mut ledger = self.receipt_ledger_from_journal_unlocked(session_id)?;
            let receipt = ledger.append(input)?;
            let receipt_record = self.append_event_unlocked(
                session_id,
                JournalEvent::ReceiptAppended {
                    receipt: receipt.clone(),
                },
                Utc::now(),
            )?;
            Ok(ReceiptAppendOutcome {
                receipt_record,
                receipt,
            })
        })
    }

    /// Snapshot the open execution lease and projection needed for worker
    /// pre-execution checks, then release the session lock before any blocking
    /// sandbox work starts.
    ///
    /// New execution starts still require a running session here. Later receipt
    /// completion is allowed through [`Self::append_receipt_for_execution_lease`]
    /// even if local lifecycle controls pause or cancel the session while the
    /// sandboxed process is already running.
    pub fn prepare_open_execution_lease(
        &self,
        session_id: &str,
        lease_id: &str,
    ) -> DaemonResult<OpenExecutionLeasePreparation> {
        self.with_session_lock(session_id, || {
            let journal = self.load_journal_unlocked(session_id)?;
            let projection = project_journal(session_id, &journal)?;
            ensure_session_running(&projection.session)?;
            let admission_state = admission_state_from_journal(session_id, &journal)?;
            let lease = admission_state
                .open_execution_leases
                .values()
                .find(|lease| lease.lease_id == lease_id)
                .cloned()
                .ok_or_else(|| {
                    DaemonError::Refused(format!("execution lease {lease_id} is not open"))
                })?;
            Ok(OpenExecutionLeasePreparation { projection, lease })
        })
    }

    /// Run a lease-bound execution callback while holding the session lock,
    /// then append the callback's receipt to complete the already-open lease.
    ///
    /// This is the split worker counterpart to
    /// [`Self::execute_and_append_receipt`]: the lease has already been claimed,
    /// but grants, lifecycle state, open-lease identity, and receipt causality
    /// are still rechecked under the daemon lock immediately before the tool
    /// enters the sandbox.
    pub fn execute_open_lease_and_append_receipt<T, E>(
        &self,
        session_id: &str,
        lease_id: &str,
        execute: impl FnOnce(
            &SessionProjection,
            &ExecutionLease,
        ) -> Result<(CapabilityReceiptInput, T), E>,
    ) -> Result<(ReceiptAppendOutcome, T), E>
    where
        E: From<DaemonError>,
    {
        let _lock = self.acquire_session_lock(session_id).map_err(E::from)?;
        let journal = self.load_journal_unlocked(session_id).map_err(E::from)?;
        let projection = project_journal(session_id, &journal).map_err(E::from)?;
        ensure_session_running(&projection.session).map_err(E::from)?;
        let admission_state =
            admission_state_from_journal(session_id, &journal).map_err(E::from)?;
        let open_lease = admission_state
            .open_execution_leases
            .values()
            .find(|lease| lease.lease_id == lease_id)
            .ok_or_else(|| DaemonError::Refused(format!("execution lease {lease_id} is not open")))
            .map_err(E::from)?;
        let (input, outcome) = execute(&projection, open_lease)?;
        if input.action_id != open_lease.action_id {
            return Err(E::from(DaemonError::Refused(format!(
                "receipt for action {} does not match execution lease {} action {}",
                input.action_id, lease_id, open_lease.action_id
            ))));
        }
        let mut ledger = self
            .receipt_ledger_from_journal_unlocked(session_id)
            .map_err(E::from)?;
        let receipt = ledger
            .append(input)
            .map_err(DaemonError::from)
            .map_err(E::from)?;
        let receipt_record = self
            .append_event_unlocked(
                session_id,
                JournalEvent::ReceiptAppended {
                    receipt: receipt.clone(),
                },
                Utc::now(),
            )
            .map_err(E::from)?;
        Ok((
            ReceiptAppendOutcome {
                receipt_record,
                receipt,
            },
            outcome,
        ))
    }

    /// Append a durable execution lease, run one execution callback while
    /// holding the per-session daemon lock, then append the returned receipt
    /// input before releasing the lock.
    ///
    /// This is the gateway runtime authority handoff for local tool execution:
    /// `Allowed` is not executable authority until this method journals and
    /// syncs the lease. Lifecycle transitions and competing daemon writes
    /// cannot interleave between lease issuance, execution, and receipt append.
    pub fn execute_and_append_receipt<T, E>(
        &self,
        session_id: &str,
        lease: ExecutionLease,
        _created_at: DateTime<Utc>,
        execute: impl FnOnce(&SessionProjection) -> Result<(CapabilityReceiptInput, T), E>,
    ) -> Result<(ExecutionLeaseOutcome, ReceiptAppendOutcome, T), E>
    where
        E: From<DaemonError>,
    {
        let _lock = self.acquire_session_lock(session_id).map_err(E::from)?;
        let mut journal = self.load_journal_unlocked(session_id).map_err(E::from)?;
        let lease_outcome = self
            .append_execution_lease_claim_unlocked(session_id, &mut journal, lease)
            .map_err(E::from)?;
        let projection = project_journal(session_id, &journal).map_err(E::from)?;
        let (input, outcome) = execute(&projection)?;
        let mut ledger = self
            .receipt_ledger_from_journal_unlocked(session_id)
            .map_err(E::from)?;
        let receipt = ledger
            .append(input)
            .map_err(DaemonError::from)
            .map_err(E::from)?;
        let receipt_record = self
            .append_event_unlocked(
                session_id,
                JournalEvent::ReceiptAppended {
                    receipt: receipt.clone(),
                },
                Utc::now(),
            )
            .map_err(E::from)?;
        Ok((
            lease_outcome,
            ReceiptAppendOutcome {
                receipt_record,
                receipt,
            },
            outcome,
        ))
    }

    fn append_execution_lease_claim_unlocked(
        &self,
        session_id: &str,
        journal: &mut InMemoryJournal,
        mut lease: ExecutionLease,
    ) -> DaemonResult<ExecutionLeaseOutcome> {
        if lease.session_id != session_id {
            return Err(DaemonError::Refused(format!(
                "execution lease {} is bound to session {}, not {session_id}",
                lease.lease_id, lease.session_id
            )));
        }
        let projection = project_journal(session_id, journal)?;
        ensure_session_running(&projection.session)?;
        let admission_state = admission_state_from_journal(session_id, journal)?;
        if admission_state
            .open_execution_leases
            .contains_key(&lease.action_id)
        {
            return Err(DaemonError::Refused(format!(
                "action {} already has an open execution lease",
                lease.action_id
            )));
        }
        if admission_state
            .reconciled_execution_actions
            .contains_key(&lease.action_id)
        {
            return Err(DaemonError::Refused(format!(
                "action {} has an outcome-unknown execution lease reconciliation and cannot be re-executed",
                lease.action_id
            )));
        }
        if !admission_state.open_execution_leases.is_empty() {
            return Err(open_execution_lease_refusal(
                session_id,
                &admission_state.open_execution_leases,
                "issue execution lease",
            ));
        }
        if admission_state.receipted_actions.contains(&lease.action_id) {
            return Err(DaemonError::Refused(format!(
                "action {} already has a receipt and cannot be re-executed",
                lease.action_id
            )));
        }
        let decision = admission_state
            .latest_decisions
            .get(&lease.action_id)
            .ok_or_else(|| {
                DaemonError::Refused(format!(
                    "action {} has no policy decision for execution lease",
                    lease.action_id
                ))
            })?;
        if decision.result != DecisionResult::Allowed {
            return Err(DaemonError::Refused(format!(
                "action {} latest policy decision is not allowed",
                lease.action_id
            )));
        }
        if decision.decision_id != lease.decision_id {
            return Err(DaemonError::Refused(format!(
                "execution lease {} decision {} does not match latest decision {} for action {}",
                lease.lease_id, lease.decision_id, decision.decision_id, lease.action_id
            )));
        }
        if decision.manifest_hash != lease.manifest_hash {
            return Err(DaemonError::Refused(format!(
                "execution lease {} manifest hash does not match decision {}",
                lease.lease_id, decision.decision_id
            )));
        }
        let proposal = admission_state
            .proposals
            .get(&lease.action_id)
            .ok_or_else(|| {
                DaemonError::Refused(format!(
                    "action {} has no proposed manifest for execution lease",
                    lease.action_id
                ))
            })?;
        let manifest_hash = proposal.manifest.digest().map_err(DaemonError::from)?;
        if manifest_hash != decision.manifest_hash {
            return Err(DaemonError::Refused(format!(
                "action {} manifest hash no longer matches latest decision {}",
                lease.action_id, decision.decision_id
            )));
        }
        if proposal.manifest.tool_id != lease.tool_id {
            return Err(DaemonError::Refused(format!(
                "execution lease {} tool {} does not match manifest tool {}",
                lease.lease_id, lease.tool_id, proposal.manifest.tool_id
            )));
        }
        let admitted_target = proposal
            .manifest
            .resolved_target
            .as_ref()
            .unwrap_or(&proposal.manifest.target);
        if admitted_target != &lease.target {
            return Err(DaemonError::Refused(format!(
                "execution lease {} target does not match admitted manifest target",
                lease.lease_id
            )));
        }
        if proposal.manifest.required_grants != lease.required_grants {
            return Err(DaemonError::Refused(format!(
                "execution lease {} grants do not match admitted manifest grants",
                lease.lease_id
            )));
        }
        if proposal.manifest.requested_budget != lease.requested_budget {
            return Err(DaemonError::Refused(format!(
                "execution lease {} budget does not match admitted manifest budget",
                lease.lease_id
            )));
        }
        let lease_window = lease.expires_at.signed_duration_since(lease.leased_at);
        if lease_window <= TimeDelta::zero() {
            return Err(DaemonError::Refused(format!(
                "execution lease {} has a non-positive duration",
                lease.lease_id
            )));
        }
        let requested_wall_ms = lease.requested_budget.max_wall_ms.ok_or_else(|| {
            DaemonError::Refused(format!(
                "execution lease {} must declare finite requested wall budget",
                lease.lease_id
            ))
        })?;
        let max_lease_ms = requested_wall_ms
            .checked_add(EXECUTION_LEASE_OVERHEAD_GRACE_MS)
            .ok_or_else(|| {
                DaemonError::Refused(format!(
                    "execution lease {} duration overflowed requested wall budget",
                    lease.lease_id
                ))
            })?;
        let max_lease_window =
            TimeDelta::milliseconds(i64::try_from(max_lease_ms).map_err(|_| {
                DaemonError::Refused(format!(
                    "execution lease {} duration cannot fit signed milliseconds",
                    lease.lease_id
                ))
            })?);
        if lease_window > max_lease_window {
            return Err(DaemonError::Refused(format!(
                "execution lease {} duration exceeds requested wall budget plus daemon overhead grace",
                lease.lease_id
            )));
        }
        let lease_issued_at = Utc::now();
        lease.leased_at = lease_issued_at;
        lease.expires_at = lease_issued_at
            .checked_add_signed(lease_window)
            .ok_or_else(|| {
                DaemonError::Refused(format!(
                    "execution lease {} expiration overflowed daemon time",
                    lease.lease_id
                ))
            })?;
        let lease_record = journal
            .append(
                JournalEvent::ExecutionLeaseIssued {
                    lease: lease.clone(),
                },
                lease_issued_at,
            )
            .map_err(DaemonError::from)?;
        journal.verify_chain().map_err(DaemonError::from)?;
        self.write_journal_record_unlocked(session_id, &lease_record)?;
        Ok(ExecutionLeaseOutcome {
            lease_record,
            lease,
        })
    }

    /// Load and verify a session journal under the writer lock so readers never
    /// observe a half-written append from another process.
    pub fn load_journal(&self, session_id: &str) -> DaemonResult<InMemoryJournal> {
        self.with_session_lock(session_id, || self.load_journal_unlocked(session_id))
    }

    /// Load and verify a session receipt ledger under the writer lock.
    pub fn load_receipts(&self, session_id: &str) -> DaemonResult<ReceiptLedger> {
        self.with_session_lock(session_id, || {
            self.receipt_ledger_from_journal_unlocked(session_id)
        })
    }

    /// Rebuild the read model from the journal under the writer lock.
    pub fn project(&self, session_id: &str) -> DaemonResult<SessionProjection> {
        self.with_session_lock(session_id, || self.project_unlocked(session_id))
    }

    /// Return unresolved execution leases from the journal-derived runtime
    /// state under the session lock.
    ///
    /// This is the daemon recovery gate for the crash window after
    /// `ExecutionLeaseIssued` is durable but before `ReceiptAppended` resolves
    /// the side effect. Callers must treat every returned lease as
    /// outcome-unknown authority: do not re-execute the action, synthesize a
    /// success receipt, or resume an agent loop until an explicit
    /// reconciliation path closes the lease.
    pub fn open_execution_leases(&self, session_id: &str) -> DaemonResult<Vec<ExecutionLease>> {
        self.with_session_lock(session_id, || {
            let journal = self.load_journal_unlocked(session_id)?;
            let admission_state = admission_state_from_journal(session_id, &journal)?;
            Ok(admission_state
                .open_execution_leases
                .into_values()
                .collect())
        })
    }

    /// Export a consistent read-only trace view under one session lock.
    pub fn export_session_trace(&self, session_id: &str) -> DaemonResult<SessionTraceExport> {
        self.with_session_lock(session_id, || {
            let journal = self.load_journal_unlocked(session_id)?;
            let projection = project_journal(session_id, &journal)?;
            Ok(SessionTraceExport {
                projection,
                journal: journal.snapshot(),
            })
        })
    }

    fn append_event_unlocked(
        &self,
        session_id: &str,
        event: JournalEvent,
        created_at: DateTime<Utc>,
    ) -> DaemonResult<JournalRecord> {
        if !self.session_exists_unlocked(session_id)? {
            return Err(DaemonError::SessionNotFound(session_id.to_string()));
        }
        let mut journal = self.load_journal_unlocked(session_id)?;
        let record = journal.append(event, created_at)?;
        journal.verify_chain()?;
        self.write_journal_record_unlocked(session_id, &record)?;
        Ok(record)
    }

    fn write_journal_record_unlocked(
        &self,
        session_id: &str,
        record: &JournalRecord,
    ) -> DaemonResult<()> {
        self.write_journal_records_unlocked(session_id, [record])
    }

    fn write_journal_records_unlocked<'a>(
        &self,
        session_id: &str,
        records: impl IntoIterator<Item = &'a JournalRecord>,
    ) -> DaemonResult<()> {
        let mut batch = String::new();
        for record in records {
            batch.push_str(&serde_json::to_string(record)?);
            batch.push('\n');
        }
        let mut file = OpenOptions::new()
            .append(true)
            .open(self.journal_path(session_id)?)?;
        file.write_all(batch.as_bytes())?;
        file.sync_all()?;
        Ok(())
    }

    fn load_journal_unlocked(&self, session_id: &str) -> DaemonResult<InMemoryJournal> {
        let path = self.journal_path(session_id)?;
        if !path.is_file() {
            return Err(DaemonError::SessionNotFound(session_id.to_string()));
        }
        let mut records = Vec::new();
        for line in fs::read_to_string(path)?.lines() {
            let line = line.trim();
            if !line.is_empty() {
                records.push(serde_json::from_str::<JournalRecord>(line)?);
            }
        }
        let journal = InMemoryJournal::from_records(records);
        journal.verify_chain()?;
        ensure_genesis(session_id, &journal)?;
        Ok(journal)
    }

    fn receipt_ledger_from_journal_unlocked(
        &self,
        session_id: &str,
    ) -> DaemonResult<ReceiptLedger> {
        let journal = self.load_journal_unlocked(session_id)?;
        let mut receipts = Vec::new();
        for record in journal.records() {
            if let JournalEvent::ReceiptAppended { receipt } = &record.event {
                receipts.push(receipt.clone());
            }
        }
        let ledger = ReceiptLedger::from_receipts(receipts);
        ledger.verify_chain()?;
        Ok(ledger)
    }

    fn load_tool_registry_unlocked(&self) -> DaemonResult<ToolRegistry> {
        let path = self.tool_registry_path();
        if !path.is_file() {
            return Ok(default_runtime_tool_registry());
        }
        Ok(serde_json::from_str(&fs::read_to_string(path)?)?)
    }

    fn write_tool_registry_unlocked(&self, registry: &ToolRegistry) -> DaemonResult<()> {
        fs::create_dir_all(&self.root)?;
        let tmp_path = self.root.join(format!("{TOOL_REGISTRY_FILE}.tmp"));
        let mut file = File::create(&tmp_path)?;
        file.write_all(serde_json::to_string_pretty(registry)?.as_bytes())?;
        file.write_all(
            b"
",
        )?;
        file.sync_all()?;
        drop(file);
        fs::rename(tmp_path, self.tool_registry_path())?;
        Ok(())
    }

    fn project_unlocked(&self, session_id: &str) -> DaemonResult<SessionProjection> {
        let journal = self.load_journal_unlocked(session_id)?;
        project_journal(session_id, &journal)
    }

    fn session_exists_unlocked(&self, session_id: &str) -> DaemonResult<bool> {
        let path = self.journal_path(session_id)?;
        if !path.is_file() {
            return Ok(false);
        }
        match self.load_journal_unlocked(session_id) {
            Ok(_) => Ok(true),
            Err(DaemonError::SessionNotFound(_)) => Ok(false),
            Err(err) => Err(err),
        }
    }

    fn session_dir(&self, session_id: &str) -> DaemonResult<PathBuf> {
        validate_session_id(session_id)?;
        Ok(self.root.join(SESSIONS_DIR).join(session_id))
    }

    fn journal_path(&self, session_id: &str) -> DaemonResult<PathBuf> {
        Ok(self.session_dir(session_id)?.join(JOURNAL_FILE))
    }

    fn receipts_path(&self, session_id: &str) -> DaemonResult<PathBuf> {
        Ok(self.session_dir(session_id)?.join(RECEIPTS_FILE))
    }

    fn tool_registry_path(&self) -> PathBuf {
        self.root.join(TOOL_REGISTRY_FILE)
    }

    fn registry_lock_path(&self) -> PathBuf {
        self.root.join(TOOL_REGISTRY_LOCK)
    }

    fn lock_path(&self, session_id: &str) -> DaemonResult<PathBuf> {
        validate_session_id(session_id)?;
        Ok(self
            .root
            .join(SESSIONS_DIR)
            .join(format!("{session_id}{LOCK_SUFFIX}")))
    }

    fn with_session_lock<T>(
        &self,
        session_id: &str,
        f: impl FnOnce() -> DaemonResult<T>,
    ) -> DaemonResult<T> {
        let _lock = self.acquire_session_lock(session_id)?;
        f()
    }

    fn with_registry_lock<T>(&self, f: impl FnOnce() -> DaemonResult<T>) -> DaemonResult<T> {
        let _lock = self.acquire_lock_path(self.registry_lock_path(), "tool-registry")?;
        f()
    }

    fn acquire_session_lock(&self, session_id: &str) -> DaemonResult<SessionLock> {
        let path = self.lock_path(session_id)?;
        self.acquire_lock_path(path, session_id)
    }

    fn acquire_lock_path(&self, path: PathBuf, label: &str) -> DaemonResult<SessionLock> {
        let deadline = Instant::now()
            .checked_add(self.options.lock_timeout)
            .unwrap_or_else(Instant::now);
        loop {
            match fs::create_dir(&path) {
                Ok(()) => return Ok(SessionLock { path }),
                Err(err) if err.kind() == std::io::ErrorKind::AlreadyExists => {
                    let now = Instant::now();
                    if now >= deadline {
                        return Err(DaemonError::LockTimeout(label.to_string()));
                    }
                    let remaining = deadline.saturating_duration_since(now);
                    let poll = if self.options.lock_poll_interval.is_zero() {
                        Duration::from_millis(1)
                    } else {
                        self.options.lock_poll_interval
                    };
                    thread::sleep(poll.min(remaining));
                }
                Err(err) => return Err(err.into()),
            }
        }
    }
}

fn project_journal(session_id: &str, journal: &InMemoryJournal) -> DaemonResult<SessionProjection> {
    let mut session = None;
    let mut grants = Vec::new();
    let mut revoked_handles = BTreeSet::new();
    let mut mandates = Vec::new();
    let mut manifests = Vec::new();
    let mut decisions = Vec::new();
    let mut execution_leases = Vec::new();
    let mut execution_lease_heartbeats = Vec::new();
    let mut execution_reconciliations = Vec::new();
    let mut approvals = Vec::new();
    let mut simulations = Vec::new();
    let mut receipts = Vec::new();
    for record in journal.records() {
        match &record.event {
            JournalEvent::SessionCreated { session: created } => {
                if created.session_id != session_id {
                    return Err(DaemonError::Refused(format!(
                        "session {session_id} journal contains SessionCreated for {}",
                        created.session_id
                    )));
                }
                if session.is_some() {
                    return Err(DaemonError::Refused(format!(
                        "session {session_id} journal contains more than one SessionCreated event"
                    )));
                }
                session = Some(created.clone());
            }
            JournalEvent::SessionStatusChanged {
                session_id: event_session_id,
                to,
                ..
            } => {
                if event_session_id != session_id {
                    return Err(DaemonError::Refused(format!(
                        "session {session_id} journal contains status transition for {event_session_id}"
                    )));
                }
                let Some(projected) = session.as_mut() else {
                    return Err(DaemonError::Refused(format!(
                        "session transition for {event_session_id} appears before SessionCreated"
                    )));
                };
                projected.status = to.clone();
            }
            JournalEvent::CapabilityGranted { grant } => grants.push(grant.clone()),
            JournalEvent::CapabilityRevoked {
                revocation_handle, ..
            } => {
                revoked_handles.insert(revocation_handle.clone());
            }
            JournalEvent::PaymentMandateIssued { mandate } => mandates.push(mandate.clone()),
            JournalEvent::ActionProposed { manifest } => {
                manifests.push(manifest.as_ref().clone());
            }
            JournalEvent::PolicyDecided { decision } => decisions.push(decision.clone()),
            JournalEvent::ExecutionLeaseIssued { lease } => execution_leases.push(lease.clone()),
            JournalEvent::ExecutionLeaseHeartbeated { heartbeat } => {
                if let Some(lease) = execution_leases
                    .iter_mut()
                    .rev()
                    .find(|lease| lease.action_id == heartbeat.action_id)
                {
                    lease.expires_at = heartbeat.extended_expires_at;
                }
                execution_lease_heartbeats.push(heartbeat.clone());
            }
            JournalEvent::ExecutionLeaseReconciled { reconciliation } => {
                execution_reconciliations.push(reconciliation.clone());
            }
            JournalEvent::ApprovalRecorded { approval } => approvals.push(approval.clone()),
            JournalEvent::SimulationRecorded { simulation } => simulations.push(simulation.clone()),
            JournalEvent::ReceiptAppended { receipt } => receipts.push(receipt.clone()),
            JournalEvent::MemoryWritten { .. }
            | JournalEvent::ScenarioEvaluated { .. }
            | JournalEvent::IncidentAnnotated { .. } => {}
        }
    }
    let session = session.ok_or_else(|| {
        DaemonError::Refused(format!(
            "session {session_id} journal has no SessionCreated event"
        ))
    })?;
    Ok(SessionProjection {
        session,
        grants,
        revoked_handles,
        mandates,
        manifests,
        decisions,
        execution_leases,
        execution_lease_heartbeats,
        execution_reconciliations,
        approvals,
        simulations,
        receipts,
    })
}

fn default_runtime_tool_registry() -> ToolRegistry {
    ToolRegistry::new(RegistryPolicy {
        require_signature: false,
        ..Default::default()
    })
}

struct SessionLock {
    path: PathBuf,
}

impl Drop for SessionLock {
    fn drop(&mut self) {
        let _ = fs::remove_dir(&self.path);
    }
}

fn validate_session_id(session_id: &str) -> DaemonResult<()> {
    let is_safe = !session_id.is_empty()
        && session_id
            .bytes()
            .all(|b| b.is_ascii_alphanumeric() || b == b'-' || b == b'_');
    if is_safe {
        Ok(())
    } else {
        Err(DaemonError::invalid("session", session_id))
    }
}

fn ensure_genesis(session_id: &str, journal: &InMemoryJournal) -> DaemonResult<()> {
    let Some(first) = journal.records().first() else {
        return Err(DaemonError::SessionNotFound(session_id.to_string()));
    };
    match &first.event {
        JournalEvent::SessionCreated { session } if session.session_id == session_id => Ok(()),
        JournalEvent::SessionCreated { session } => Err(DaemonError::Refused(format!(
            "session {session_id} genesis belongs to {}",
            session.session_id
        ))),
        _ => Err(DaemonError::Refused(format!(
            "session {session_id} journal does not start with SessionCreated"
        ))),
    }
}

fn ensure_session_running(session: &AgentSession) -> DaemonResult<()> {
    if session.status == SessionStatus::Running {
        Ok(())
    } else {
        Err(DaemonError::Refused(format!(
            "session {} is not running (status {:?})",
            session.session_id, session.status
        )))
    }
}

fn open_execution_lease_refusal(
    session_id: &str,
    open_execution_leases: &BTreeMap<String, ExecutionLease>,
    action: &str,
) -> DaemonError {
    let lease_ids = open_execution_leases
        .values()
        .map(|lease| format!("{}:{}", lease.action_id, lease.lease_id))
        .collect::<Vec<_>>()
        .join(", ");
    DaemonError::Refused(format!(
        "cannot {action} for session {session_id}: unresolved open execution lease(s) [{lease_ids}] require operator reconciliation"
    ))
}

fn next_session_status(
    transition: SessionTransition,
    current: &SessionStatus,
) -> DaemonResult<SessionStatus> {
    match (transition, current) {
        (SessionTransition::Pause, SessionStatus::Running) => Ok(SessionStatus::Paused),
        (SessionTransition::Resume, SessionStatus::Paused) => Ok(SessionStatus::Running),
        (SessionTransition::Cancel, SessionStatus::Running | SessionStatus::Paused) => {
            Ok(SessionStatus::Canceled)
        }
        _ => Err(DaemonError::Refused(format!(
            "illegal session transition {transition:?} from status {current:?}"
        ))),
    }
}

struct AdmissionState {
    session: AgentSession,
    grants: BTreeMap<String, CapabilityGrant>,
    revoked_handles: BTreeSet<String>,
    issued_revocation_handles: BTreeSet<String>,
    event_ids: BTreeSet<String>,
    mandates: BTreeMap<String, PaymentMandate>,
    payment_reserved_by_mandate: BTreeMap<String, u64>,
    session_budget_used: Budget,
    pending_runtime_budget_by_action: BTreeMap<String, Budget>,
    receipted_actions: BTreeSet<String>,
    open_execution_leases: BTreeMap<String, ExecutionLease>,
    reconciled_execution_actions: BTreeMap<String, ExecutionLeaseReconciliation>,
    proposals: BTreeMap<String, ProposedAction>,
    latest_decisions: BTreeMap<String, PolicyDecision>,
    approvals: Vec<ApprovalEvidence>,
    simulations: Vec<SimulationEvidence>,
}

struct ProposedAction {
    record: JournalRecord,
    manifest: ActionManifest,
}

fn admission_state_from_journal(
    session_id: &str,
    journal: &InMemoryJournal,
) -> DaemonResult<AdmissionState> {
    let mut session = None;
    let mut grants = BTreeMap::new();
    let mut revoked_handles = BTreeSet::new();
    let mut issued_revocation_handles = BTreeSet::new();
    let mut event_ids = BTreeSet::new();
    let mut mandates = BTreeMap::new();
    let mut payment_reserved_by_mandate = BTreeMap::new();
    let mut session_budget_used = Budget::default();
    let mut receipted_actions = BTreeSet::new();
    let mut open_execution_leases = BTreeMap::new();
    let mut reconciled_execution_actions = BTreeMap::new();
    let mut proposals = BTreeMap::new();
    let mut latest_decisions = BTreeMap::new();
    let mut approvals = Vec::new();
    let mut simulations = Vec::new();
    for record in journal.records() {
        match &record.event {
            JournalEvent::SessionCreated { session: created } => {
                if created.session_id != session_id {
                    return Err(DaemonError::Refused(format!(
                        "session {session_id} journal contains SessionCreated for {}",
                        created.session_id
                    )));
                }
                if session.is_some() {
                    return Err(DaemonError::Refused(format!(
                        "session {session_id} journal contains more than one SessionCreated event"
                    )));
                }
                session = Some(created.clone());
            }
            JournalEvent::SessionStatusChanged {
                session_id: event_session_id,
                to,
                ..
            } => {
                if event_session_id != session_id {
                    return Err(DaemonError::Refused(format!(
                        "session {session_id} journal contains status transition for {event_session_id}"
                    )));
                }
                let Some(projected) = session.as_mut() else {
                    return Err(DaemonError::Refused(format!(
                        "session transition for {event_session_id} appears before SessionCreated"
                    )));
                };
                projected.status = to.clone();
            }
            JournalEvent::CapabilityGranted { grant } => {
                issued_revocation_handles.insert(grant.revocation_handle.clone());
                grants.insert(grant.grant_id.clone(), grant.clone());
            }
            JournalEvent::CapabilityRevoked {
                grant_id,
                revocation_handle,
                ..
            } => {
                let Some(grant) = grants.get(grant_id) else {
                    return Err(DaemonError::Refused(format!(
                        "revocation references grant {grant_id} before it was issued"
                    )));
                };
                if grant.revocation_handle != *revocation_handle {
                    return Err(DaemonError::Refused(format!(
                        "revocation for grant {grant_id} uses handle {revocation_handle}, expected {}",
                        grant.revocation_handle
                    )));
                }
                if !revoked_handles.insert(revocation_handle.clone()) {
                    return Err(DaemonError::Refused(format!(
                        "revocation handle {revocation_handle} was recorded more than once"
                    )));
                }
            }
            JournalEvent::PaymentMandateIssued { mandate } => {
                mandates.insert(mandate.mandate_id.clone(), mandate.clone());
            }
            JournalEvent::ActionProposed { manifest } => {
                proposals.insert(
                    manifest.action_id.clone(),
                    ProposedAction {
                        record: record.clone(),
                        manifest: manifest.as_ref().clone(),
                    },
                );
            }
            JournalEvent::PolicyDecided { decision } => {
                latest_decisions.insert(decision.action_id.clone(), decision.clone());
            }
            JournalEvent::ExecutionLeaseIssued { lease } => {
                if reconciled_execution_actions.contains_key(&lease.action_id) {
                    return Err(DaemonError::Refused(format!(
                        "action {} already has an execution lease reconciliation",
                        lease.action_id
                    )));
                }
                if open_execution_leases
                    .insert(lease.action_id.clone(), lease.clone())
                    .is_some()
                {
                    return Err(DaemonError::Refused(format!(
                        "action {} already has an open execution lease",
                        lease.action_id
                    )));
                }
            }
            JournalEvent::ExecutionLeaseHeartbeated { heartbeat } => {
                let Some(open_lease) = open_execution_leases.get_mut(&heartbeat.action_id) else {
                    return Err(DaemonError::Refused(format!(
                        "execution lease heartbeat {} references action {} without an open execution lease",
                        heartbeat.heartbeat_id, heartbeat.action_id
                    )));
                };
                if open_lease.lease_id != heartbeat.lease_id {
                    return Err(DaemonError::Refused(format!(
                        "execution lease heartbeat {} targets {}, but open lease is {}",
                        heartbeat.heartbeat_id, heartbeat.lease_id, open_lease.lease_id
                    )));
                }
                open_lease.expires_at = heartbeat.extended_expires_at;
            }
            JournalEvent::ExecutionLeaseReconciled { reconciliation } => {
                let Some(open_lease) = open_execution_leases.get(&reconciliation.action_id) else {
                    return Err(DaemonError::Refused(format!(
                        "execution lease reconciliation {} references action {} without an open execution lease",
                        reconciliation.reconciliation_id, reconciliation.action_id
                    )));
                };
                if open_lease.lease_id != reconciliation.lease_id {
                    return Err(DaemonError::Refused(format!(
                        "execution lease reconciliation {} targets {}, but open lease is {}",
                        reconciliation.reconciliation_id,
                        reconciliation.lease_id,
                        open_lease.lease_id
                    )));
                }
                open_execution_leases.remove(&reconciliation.action_id);
                reconciled_execution_actions
                    .insert(reconciliation.action_id.clone(), reconciliation.clone());
            }
            JournalEvent::ApprovalRecorded { approval } => approvals.push(approval.clone()),
            JournalEvent::SimulationRecorded { simulation } => simulations.push(simulation.clone()),
            JournalEvent::ReceiptAppended { receipt } => {
                receipted_actions.insert(receipt.action_id.clone());
                open_execution_leases.remove(&receipt.action_id);
                debit_receipt_budget(&mut session_budget_used, receipt)?;
            }
            JournalEvent::MemoryWritten { .. }
            | JournalEvent::ScenarioEvaluated { .. }
            | JournalEvent::IncidentAnnotated { .. } => {}
        }
        if let Some(event_id) = journal_event_id(&record.event) {
            event_ids.insert(event_id.to_string());
        }
    }
    let session = session.ok_or_else(|| {
        DaemonError::Refused(format!(
            "session {session_id} journal has no SessionCreated event"
        ))
    })?;
    for (action_id, decision) in &latest_decisions {
        if !reserves_payment_capacity(decision.result.clone()) {
            continue;
        }
        let Some(proposal) = proposals.get(action_id) else {
            return Err(DaemonError::Refused(format!(
                "reserving policy decision for action {action_id} has no proposed manifest"
            )));
        };
        if !is_payment_manifest(&proposal.manifest) {
            continue;
        }
        let Some(intent) = &proposal.manifest.payment_intent else {
            return Err(DaemonError::Refused(format!(
                "reserving payment decision for action {action_id} has no payment_intent"
            )));
        };
        let entry = payment_reserved_by_mandate
            .entry(intent.mandate_id.clone())
            .or_insert(0_u64);
        *entry = entry
            .checked_add(intent.amount_minor_units)
            .ok_or_else(|| {
                DaemonError::Refused(format!(
                    "payment cumulative reservation overflowed mandate meter during replay for {}",
                    intent.mandate_id
                ))
            })?;
    }
    let mut pending_runtime_budget_by_action = BTreeMap::new();
    for (action_id, decision) in &latest_decisions {
        if !reserves_runtime_budget(decision.result.clone())
            || receipted_actions.contains(action_id)
        {
            continue;
        }
        let Some(proposal) = proposals.get(action_id) else {
            return Err(DaemonError::Refused(format!(
                "runtime-budget-reserving policy decision for action {action_id} has no proposed manifest"
            )));
        };
        let mut reserved = Budget::default();
        debit_manifest_budget(&mut reserved, &proposal.manifest)?;
        debit_budget(
            &mut session_budget_used,
            &reserved,
            &proposal.manifest.action_id,
        )?;
        pending_runtime_budget_by_action.insert(action_id.clone(), reserved);
    }

    Ok(AdmissionState {
        session,
        grants,
        revoked_handles,
        issued_revocation_handles,
        event_ids,
        mandates,
        payment_reserved_by_mandate,
        session_budget_used,
        pending_runtime_budget_by_action,
        receipted_actions,
        open_execution_leases,
        reconciled_execution_actions,
        proposals,
        latest_decisions,
        approvals,
        simulations,
    })
}

fn reserves_payment_capacity(result: beater_os_core::DecisionResult) -> bool {
    matches!(
        result,
        beater_os_core::DecisionResult::Allowed
            | beater_os_core::DecisionResult::NeedsApproval
            | beater_os_core::DecisionResult::NeedsSimulation
    )
}

fn debit_receipt_budget(budget: &mut Budget, receipt: &CapabilityReceipt) -> DaemonResult<()> {
    let tool_calls = budget.max_tool_calls.get_or_insert(0);
    *tool_calls = tool_calls.checked_add(1).ok_or_else(|| {
        DaemonError::Refused(
            "session budget replay overflowed committed tool-call usage".to_string(),
        )
    })?;

    let elapsed = receipt
        .finished_at
        .signed_duration_since(receipt.started_at);
    if elapsed.num_milliseconds() < 0 {
        return Err(DaemonError::Refused(format!(
            "receipt {} finished before it started",
            receipt.receipt_id
        )));
    }
    let elapsed_ms = u64::try_from(elapsed.num_milliseconds()).map_err(|_| {
        DaemonError::Refused(format!(
            "receipt {} wall-clock duration could not fit u64 milliseconds",
            receipt.receipt_id
        ))
    })?;
    let wall_ms = budget.max_wall_ms.get_or_insert(0);
    *wall_ms = wall_ms.checked_add(elapsed_ms).ok_or_else(|| {
        DaemonError::Refused(
            "session budget replay overflowed committed wall-clock usage".to_string(),
        )
    })?;
    Ok(())
}

fn reserves_runtime_budget(result: beater_os_core::DecisionResult) -> bool {
    matches!(
        result,
        beater_os_core::DecisionResult::Allowed
            | beater_os_core::DecisionResult::NeedsApproval
            | beater_os_core::DecisionResult::NeedsSimulation
    )
}

fn debit_manifest_budget(budget: &mut Budget, manifest: &ActionManifest) -> DaemonResult<()> {
    if let Some(requested) = manifest.requested_budget.max_tool_calls {
        let tool_calls = budget.max_tool_calls.get_or_insert(0);
        *tool_calls = tool_calls.checked_add(requested).ok_or_else(|| {
            DaemonError::Refused(format!(
                "session budget replay overflowed pending tool-call reservation for action {}",
                manifest.action_id
            ))
        })?;
    }
    if let Some(requested) = manifest.requested_budget.max_wall_ms {
        let wall_ms = budget.max_wall_ms.get_or_insert(0);
        *wall_ms = wall_ms.checked_add(requested).ok_or_else(|| {
            DaemonError::Refused(format!(
                "session budget replay overflowed pending wall-clock reservation for action {}",
                manifest.action_id
            ))
        })?;
    }
    Ok(())
}

fn debit_budget(budget: &mut Budget, requested: &Budget, action_id: &str) -> DaemonResult<()> {
    if let Some(requested) = requested.max_tool_calls {
        let tool_calls = budget.max_tool_calls.get_or_insert(0);
        *tool_calls = tool_calls.checked_add(requested).ok_or_else(|| {
            DaemonError::Refused(format!(
                "session budget replay overflowed pending tool-call reservation for action {action_id}"
            ))
        })?;
    }
    if let Some(requested) = requested.max_wall_ms {
        let wall_ms = budget.max_wall_ms.get_or_insert(0);
        *wall_ms = wall_ms.checked_add(requested).ok_or_else(|| {
            DaemonError::Refused(format!(
                "session budget replay overflowed pending wall-clock reservation for action {action_id}"
            ))
        })?;
    }
    Ok(())
}

fn runtime_budget_used_excluding(state: &AdmissionState, excluded_action_id: &str) -> Budget {
    let mut used = state.session_budget_used.clone();
    let Some(reserved) = state
        .pending_runtime_budget_by_action
        .get(excluded_action_id)
    else {
        return used;
    };
    if let (Some(used_calls), Some(reserved_calls)) =
        (&mut used.max_tool_calls, reserved.max_tool_calls)
    {
        *used_calls = used_calls.saturating_sub(reserved_calls);
        if *used_calls == 0 {
            used.max_tool_calls = None;
        }
    }
    if let (Some(used_wall_ms), Some(reserved_wall_ms)) =
        (&mut used.max_wall_ms, reserved.max_wall_ms)
    {
        *used_wall_ms = used_wall_ms.saturating_sub(reserved_wall_ms);
        if *used_wall_ms == 0 {
            used.max_wall_ms = None;
        }
    }
    used
}

fn payment_reserved_by_mandate_excluding(
    state: &AdmissionState,
    excluded_action_id: &str,
) -> BTreeMap<String, u64> {
    let mut reserved = state.payment_reserved_by_mandate.clone();
    let Some(decision) = state.latest_decisions.get(excluded_action_id) else {
        return reserved;
    };
    if !reserves_payment_capacity(decision.result.clone()) {
        return reserved;
    }
    let Some(proposal) = state.proposals.get(excluded_action_id) else {
        return reserved;
    };
    let Some(intent) = proposal.manifest.payment_intent.as_ref() else {
        return reserved;
    };
    if let Some(entry) = reserved.get_mut(&intent.mandate_id) {
        *entry = entry.saturating_sub(intent.amount_minor_units);
        if *entry == 0 {
            reserved.remove(&intent.mandate_id);
        }
    }
    reserved
}

fn is_payment_manifest(manifest: &ActionManifest) -> bool {
    manifest.action_kind == ActionKind::Spend
        || manifest
            .expected_side_effects
            .contains(&SideEffectClass::Payment)
}

fn journal_event_id(event: &JournalEvent) -> Option<&str> {
    match event {
        JournalEvent::SessionCreated { session } => Some(session.session_id.as_str()),
        JournalEvent::SessionStatusChanged { transition_id, .. } => Some(transition_id.as_str()),
        JournalEvent::CapabilityGranted { grant } => Some(grant.grant_id.as_str()),
        JournalEvent::CapabilityRevoked {
            revocation_handle, ..
        } => Some(revocation_handle.as_str()),
        JournalEvent::PaymentMandateIssued { mandate } => Some(mandate.mandate_id.as_str()),
        JournalEvent::ActionProposed { manifest } => Some(manifest.action_id.as_str()),
        JournalEvent::PolicyDecided { decision } => Some(decision.decision_id.as_str()),
        JournalEvent::ExecutionLeaseIssued { lease } => Some(lease.lease_id.as_str()),
        JournalEvent::ExecutionLeaseHeartbeated { heartbeat } => {
            Some(heartbeat.heartbeat_id.as_str())
        }
        JournalEvent::ExecutionLeaseReconciled { reconciliation } => {
            Some(reconciliation.reconciliation_id.as_str())
        }
        JournalEvent::ApprovalRecorded { approval } => Some(approval.review_id.as_str()),
        JournalEvent::SimulationRecorded { simulation } => Some(simulation.simulation_id.as_str()),
        JournalEvent::ReceiptAppended { receipt } => Some(receipt.receipt_id.as_str()),
        JournalEvent::MemoryWritten { .. } => None,
        JournalEvent::ScenarioEvaluated { scenario, .. } => Some(scenario.scenario_id.as_str()),
        JournalEvent::IncidentAnnotated { incident_id, .. } => Some(incident_id.as_str()),
    }
}

fn validate_approval_evidence(
    state: &AdmissionState,
    approval: &ApprovalEvidence,
) -> DaemonResult<()> {
    if approval.policy_version != DAEMON_POLICY_VERSION {
        return Err(DaemonError::Refused(format!(
            "approval {} uses unsupported policy version {}",
            approval.review_id, approval.policy_version
        )));
    }
    let Some(proposal) = state.proposals.get(&approval.action_id) else {
        return Err(DaemonError::Refused(format!(
            "approval {} references unproposed action {}",
            approval.review_id, approval.action_id
        )));
    };
    if proposal.manifest.digest()? != approval.manifest_hash {
        return Err(DaemonError::Refused(format!(
            "approval {} manifest hash does not match action {}",
            approval.review_id, approval.action_id
        )));
    }
    if !state.grants.contains_key(&approval.grant_id) {
        return Err(DaemonError::Refused(format!(
            "approval {} references unknown grant {}",
            approval.review_id, approval.grant_id
        )));
    }
    if approval.approved_at < proposal.record.created_at {
        return Err(DaemonError::Refused(format!(
            "approval {} predates action proposal {}",
            approval.review_id, approval.action_id
        )));
    }
    Ok(())
}

fn validate_simulation_evidence(
    state: &AdmissionState,
    simulation: &SimulationEvidence,
) -> DaemonResult<()> {
    if simulation.policy_version != DAEMON_POLICY_VERSION {
        return Err(DaemonError::Refused(format!(
            "simulation {} uses unsupported policy version {}",
            simulation.simulation_id, simulation.policy_version
        )));
    }
    let Some(proposal) = state.proposals.get(&simulation.action_id) else {
        return Err(DaemonError::Refused(format!(
            "simulation {} references unproposed action {}",
            simulation.simulation_id, simulation.action_id
        )));
    };
    if proposal.manifest.digest()? != simulation.manifest_hash {
        return Err(DaemonError::Refused(format!(
            "simulation {} manifest hash does not match action {}",
            simulation.simulation_id, simulation.action_id
        )));
    }
    if simulation.passed_at < proposal.record.created_at {
        return Err(DaemonError::Refused(format!(
            "simulation {} predates action proposal {}",
            simulation.simulation_id, simulation.action_id
        )));
    }
    let Some(decision) = state.latest_decisions.get(simulation.action_id.as_str()) else {
        return Err(DaemonError::Refused(format!(
            "simulation {} references action {} without a policy decision",
            simulation.simulation_id, simulation.action_id
        )));
    };
    if decision.result != DecisionResult::NeedsSimulation {
        return Err(DaemonError::Refused(format!(
            "simulation {} references action {} without a latest NeedsSimulation decision",
            simulation.simulation_id, simulation.action_id
        )));
    }
    let Some(required_simulation) = &decision.required_simulation else {
        return Err(DaemonError::Refused(format!(
            "simulation {} references action {} whose decision has no simulation requirement",
            simulation.simulation_id, simulation.action_id
        )));
    };
    if &simulation.scenario_id != required_simulation {
        return Err(DaemonError::Refused(format!(
            "simulation {} scenario {} does not match required simulation {}",
            simulation.simulation_id, simulation.scenario_id, required_simulation
        )));
    }
    Ok(())
}

fn validate_grant_authority(state: &AdmissionState, grant: &CapabilityGrant) -> DaemonResult<()> {
    if let Some(parent_id) = &grant.parent_grant_id {
        let Some(parent) = state.grants.get(parent_id) else {
            return Err(DaemonError::Refused(format!(
                "grant {} parent {} has not been issued",
                grant.grant_id, parent_id
            )));
        };
        if grant.issuer != parent.holder {
            return Err(DaemonError::Refused(format!(
                "grant {} issuer {} does not hold parent {}",
                grant.grant_id, grant.issuer, parent.grant_id
            )));
        }
        if parent.delegation == DelegationMode::None {
            return Err(DaemonError::Refused(format!(
                "grant {} parent {} is not delegable",
                grant.grant_id, parent.grant_id
            )));
        }
        if state.revoked_handles.contains(&parent.revocation_handle) || parent.revoked {
            return Err(DaemonError::Refused(format!(
                "grant {} parent {} is revoked",
                grant.grant_id, parent.grant_id
            )));
        }
        if parent.delegation == DelegationMode::AttenuatedOnly
            && !grant_is_attenuated(parent, grant)
        {
            return Err(DaemonError::Refused(format!(
                "grant {} does not attenuate parent {}",
                grant.grant_id, parent.grant_id
            )));
        }
        if !grant_within_parent(parent, grant) {
            return Err(DaemonError::Refused(format!(
                "grant {} broadens parent {}",
                grant.grant_id, parent.grant_id
            )));
        }
    } else {
        if !state
            .session
            .initial_capability_ids
            .contains(&grant.grant_id)
        {
            return Err(DaemonError::Refused(format!(
                "root grant {} was not declared in session genesis",
                grant.grant_id
            )));
        }
        if grant.issuer != state.session.created_by {
            return Err(DaemonError::Refused(format!(
                "root grant {} issuer {} does not match session creator {}",
                grant.grant_id, grant.issuer, state.session.created_by
            )));
        }
    }
    Ok(())
}

fn grant_effectively_active(
    grant: &CapabilityGrant,
    now: DateTime<Utc>,
    revoked_handles: &BTreeSet<String>,
    grants_by_id: &BTreeMap<&str, &CapabilityGrant>,
) -> bool {
    let mut current = grant;
    let mut seen = BTreeSet::new();
    loop {
        if !current.is_active_at(now) || revoked_handles.contains(&current.revocation_handle) {
            return false;
        }
        let Some(parent_id) = current.parent_grant_id.as_deref() else {
            return true;
        };
        if !seen.insert(current.grant_id.as_str()) {
            return false;
        }
        let Some(parent) = grants_by_id.get(parent_id) else {
            return false;
        };
        current = *parent;
    }
}

fn normalize_grant_file_authority(mut grant: CapabilityGrant) -> DaemonResult<CapabilityGrant> {
    if grant.scope.selector.resource_kind == ResourceKind::FilePath
        && grant.scope.selector.resource_id != "*"
    {
        grant.scope.selector.resource_id = canonical_existing_file_authority_or_lexical(
            "resource-id",
            &grant.scope.selector.resource_id,
        )?;
    }
    let mut normalized_prefixes = BTreeSet::new();
    for prefix in &grant.constraints.path_prefixes {
        normalized_prefixes.insert(canonical_existing_file_authority("path-prefix", prefix)?);
    }
    grant.constraints.path_prefixes = normalized_prefixes;
    Ok(grant)
}

fn canonical_existing_file_authority_or_lexical(field: &str, value: &str) -> DaemonResult<String> {
    validate_absolute_lexical_file_authority(field, value)?;
    match fs::canonicalize(Path::new(value)) {
        Ok(canonical) => Ok(canonical.display().to_string()),
        Err(err) if err.kind() == std::io::ErrorKind::NotFound => Ok(value.to_string()),
        Err(err) => Err(DaemonError::Refused(format!(
            "file grant {field} {value:?} cannot be canonicalized: {err}"
        ))),
    }
}

fn canonical_existing_file_authority(field: &str, value: &str) -> DaemonResult<String> {
    validate_absolute_lexical_file_authority(field, value)?;
    fs::canonicalize(Path::new(value))
        .map(|canonical| canonical.display().to_string())
        .map_err(|err| {
            DaemonError::Refused(format!(
                "file grant {field} {value:?} cannot be canonicalized: {err}"
            ))
        })
}

fn validate_absolute_lexical_file_authority(field: &str, value: &str) -> DaemonResult<()> {
    let path = Path::new(value);
    if !path.is_absolute()
        || path.components().any(|component| {
            matches!(
                component,
                std::path::Component::CurDir
                    | std::path::Component::ParentDir
                    | std::path::Component::Prefix(_)
            )
        })
    {
        return Err(DaemonError::Refused(format!(
            "file grant {field} {value:?} must be an absolute canonical path"
        )));
    }
    Ok(())
}

fn grant_is_attenuated(parent: &CapabilityGrant, grant: &CapabilityGrant) -> bool {
    grant.expires_at < parent.expires_at
        || grant.scope != parent.scope
        || grant.denied_actions != parent.denied_actions
        || grant.constraints != parent.constraints
        || grant.delegation != parent.delegation
}

fn grant_within_parent(parent: &CapabilityGrant, grant: &CapabilityGrant) -> bool {
    grant.expires_at <= parent.expires_at
        && scope_within_parent(&parent.scope, &grant.scope)
        && grant.denied_actions.is_superset(&parent.denied_actions)
        && optional_ord_le(grant.constraints.max_risk, parent.constraints.max_risk)
        && optional_ord_le(
            grant.constraints.max_data_class,
            parent.constraints.max_data_class,
        )
        && grant
            .constraints
            .budget
            .fits_within(&parent.constraints.budget)
        && restriction_set_within_parent(
            &parent.constraints.network_allowlist,
            &grant.constraints.network_allowlist,
        )
        && restriction_set_within_parent(
            &parent.constraints.path_prefixes,
            &grant.constraints.path_prefixes,
        )
        && delegation_within_parent(parent.delegation.clone(), grant.delegation.clone())
}

fn scope_within_parent(parent: &CapabilityScope, grant: &CapabilityScope) -> bool {
    parent.selector.matches(&grant.selector) && grant.actions.is_subset(&parent.actions)
}

fn optional_ord_le<T: Ord>(child: Option<T>, parent: Option<T>) -> bool {
    match (child, parent) {
        (Some(child), Some(parent)) => child <= parent,
        (Some(_), None) => true,
        (None, None) => true,
        (None, Some(_)) => false,
    }
}

fn restriction_set_within_parent(parent: &BTreeSet<String>, child: &BTreeSet<String>) -> bool {
    parent.is_empty() || (!child.is_empty() && child.is_subset(parent))
}

fn delegation_within_parent(parent: DelegationMode, child: DelegationMode) -> bool {
    match parent {
        DelegationMode::None => child == DelegationMode::None,
        DelegationMode::AttenuatedOnly => {
            matches!(child, DelegationMode::None | DelegationMode::AttenuatedOnly)
        }
        DelegationMode::SameScope => true,
    }
}
