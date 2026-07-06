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
    ActionManifest, AdmissionContext, AgentSession, ApprovalEvidence, CapabilityGrant,
    CapabilityReceipt, CapabilityReceiptInput, CapabilityScope, DecisionResult, DelegationMode,
    InMemoryJournal, JournalEvent, JournalRecord, PaymentMandate, PolicyDecision, PolicyEngine,
    ReceiptLedger, ResourceKind, RiskClass, SessionStatus, SideEffectClass, SimulationEvidence,
    ToolManifest,
};
use chrono::{DateTime, Utc};

pub use crate::error::{DaemonError, DaemonResult};

const SESSIONS_DIR: &str = "sessions";
const JOURNAL_FILE: &str = "journal.jsonl";
const RECEIPTS_FILE: &str = "receipts.jsonl";
const LOCK_SUFFIX: &str = ".lock";
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
    pub mandates: Vec<PaymentMandate>,
    pub manifests: Vec<ActionManifest>,
    pub decisions: Vec<PolicyDecision>,
    pub approvals: Vec<ApprovalEvidence>,
    pub simulations: Vec<SimulationEvidence>,
    pub receipts: Vec<CapabilityReceipt>,
}

impl SessionProjection {
    /// Grants that are active at `now`, projected from daemon-owned journal state.
    pub fn active_grants(&self, now: DateTime<Utc>) -> Vec<CapabilityGrant> {
        self.grants
            .iter()
            .filter(|grant| grant.is_active_at(now))
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
}

/// Result of a daemon-owned policy admission transaction.
#[derive(Debug, Clone)]
pub struct AdmissionOutcome {
    pub proposal_record: JournalRecord,
    pub decision_record: JournalRecord,
    pub decision: PolicyDecision,
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
            let projection = self.project_unlocked(session_id)?;
            let from = projection.session.status.clone();
            let to = next_session_status(transition, &from)?;
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
            validate_grant_authority(&admission_state, &grant)?;
            let record = journal.append(JournalEvent::CapabilityGranted { grant }, created_at)?;
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

            let now = Utc::now();
            let ctx = AdmissionContext {
                now,
                actor_id: admission_state.session.agent_id,
                session_id: admission_state.session.session_id,
                policy_version: DAEMON_POLICY_VERSION.to_string(),
                grants: admission_state.grants.into_values().collect(),
                approvals: admission_state.approvals,
                simulations: admission_state.simulations,
                mandates: admission_state.mandates.into_values().collect(),
                revoked_handles: BTreeSet::new(),
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
            self.write_journal_records_unlocked(session_id, records_to_write.iter())?;
            Ok(AdmissionOutcome {
                proposal_record,
                decision_record,
                decision,
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
        self.with_session_lock(session_id, || {
            let projection = self.project_unlocked(session_id)?;
            ensure_session_running(&projection.session)?;
            let mut ledger = self.receipt_ledger_from_journal_unlocked(session_id)?;
            let receipt = ledger.append(input)?;
            self.append_event_unlocked(
                session_id,
                JournalEvent::ReceiptAppended {
                    receipt: receipt.clone(),
                },
                created_at,
            )?;
            Ok(receipt)
        })
    }

    /// Run one execution callback while holding the per-session daemon lock,
    /// then append the returned receipt input before releasing the lock.
    ///
    /// This is the minimal runtime lease for local tool execution: lifecycle
    /// transitions and competing daemon writes cannot interleave between the
    /// running-session check and durable receipt append.
    pub fn execute_and_append_receipt<T, E>(
        &self,
        session_id: &str,
        created_at: DateTime<Utc>,
        execute: impl FnOnce(&SessionProjection) -> Result<(CapabilityReceiptInput, T), E>,
    ) -> Result<(CapabilityReceipt, T), E>
    where
        E: From<DaemonError>,
    {
        let _lock = self.acquire_session_lock(session_id).map_err(E::from)?;
        let projection = self.project_unlocked(session_id).map_err(E::from)?;
        ensure_session_running(&projection.session).map_err(E::from)?;
        let (input, outcome) = execute(&projection)?;
        let mut ledger = self
            .receipt_ledger_from_journal_unlocked(session_id)
            .map_err(E::from)?;
        let receipt = ledger
            .append(input)
            .map_err(DaemonError::from)
            .map_err(E::from)?;
        self.append_event_unlocked(
            session_id,
            JournalEvent::ReceiptAppended {
                receipt: receipt.clone(),
            },
            created_at,
        )
        .map_err(E::from)?;
        Ok((receipt, outcome))
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

    fn project_unlocked(&self, session_id: &str) -> DaemonResult<SessionProjection> {
        let journal = self.load_journal_unlocked(session_id)?;
        let mut session = None;
        let mut grants = Vec::new();
        let mut mandates = Vec::new();
        let mut manifests = Vec::new();
        let mut decisions = Vec::new();
        let mut approvals = Vec::new();
        let mut simulations = Vec::new();
        let mut receipts = Vec::new();
        for record in journal.records() {
            match &record.event {
                JournalEvent::SessionCreated { session: created } => {
                    session = Some(created.clone());
                }
                JournalEvent::SessionStatusChanged { session_id, to, .. } => {
                    let Some(projected) = session.as_mut() else {
                        return Err(DaemonError::Refused(format!(
                            "session transition for {session_id} appears before SessionCreated"
                        )));
                    };
                    projected.status = to.clone();
                }
                JournalEvent::CapabilityGranted { grant } => grants.push(grant.clone()),
                JournalEvent::PaymentMandateIssued { mandate } => mandates.push(mandate.clone()),
                JournalEvent::ActionProposed { manifest } => {
                    manifests.push(manifest.as_ref().clone())
                }
                JournalEvent::PolicyDecided { decision } => decisions.push(decision.clone()),
                JournalEvent::ApprovalRecorded { approval } => approvals.push(approval.clone()),
                JournalEvent::SimulationRecorded { simulation } => {
                    simulations.push(simulation.clone())
                }
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
            mandates,
            manifests,
            decisions,
            approvals,
            simulations,
            receipts,
        })
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

    fn acquire_session_lock(&self, session_id: &str) -> DaemonResult<SessionLock> {
        let path = self.lock_path(session_id)?;
        let deadline = Instant::now()
            .checked_add(self.options.lock_timeout)
            .unwrap_or_else(Instant::now);
        loop {
            match fs::create_dir(&path) {
                Ok(()) => return Ok(SessionLock { path }),
                Err(err) if err.kind() == std::io::ErrorKind::AlreadyExists => {
                    let now = Instant::now();
                    if now >= deadline {
                        return Err(DaemonError::LockTimeout(session_id.to_string()));
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
    mandates: BTreeMap<String, PaymentMandate>,
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
    let mut mandates = BTreeMap::new();
    let mut proposals = BTreeMap::new();
    let mut latest_decisions = BTreeMap::new();
    let mut approvals = Vec::new();
    let mut simulations = Vec::new();
    for record in journal.records() {
        match &record.event {
            JournalEvent::SessionCreated { session: created } => {
                session = Some(created.clone());
            }
            JournalEvent::SessionStatusChanged { session_id, to, .. } => {
                let Some(projected) = session.as_mut() else {
                    return Err(DaemonError::Refused(format!(
                        "session transition for {session_id} appears before SessionCreated"
                    )));
                };
                projected.status = to.clone();
            }
            JournalEvent::CapabilityGranted { grant } => {
                grants.insert(grant.grant_id.clone(), grant.clone());
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
            JournalEvent::ApprovalRecorded { approval } => approvals.push(approval.clone()),
            JournalEvent::SimulationRecorded { simulation } => simulations.push(simulation.clone()),
            JournalEvent::ReceiptAppended { .. }
            | JournalEvent::MemoryWritten { .. }
            | JournalEvent::ScenarioEvaluated { .. }
            | JournalEvent::IncidentAnnotated { .. } => {}
        }
    }
    let session = session.ok_or_else(|| {
        DaemonError::Refused(format!(
            "session {session_id} journal has no SessionCreated event"
        ))
    })?;
    Ok(AdmissionState {
        session,
        grants,
        mandates,
        proposals,
        latest_decisions,
        approvals,
        simulations,
    })
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
