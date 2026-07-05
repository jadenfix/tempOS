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
    ActionManifest, AdmissionContext, AgentSession, CapabilityGrant, CapabilityReceipt,
    CapabilityReceiptInput, InMemoryJournal, JournalEvent, JournalRecord, PolicyDecision,
    PolicyEngine, ReceiptLedger,
};
use chrono::{DateTime, Utc};

pub use crate::error::{DaemonError, DaemonResult};

const SESSIONS_DIR: &str = "sessions";
const JOURNAL_FILE: &str = "journal.jsonl";
const RECEIPTS_FILE: &str = "receipts.jsonl";
const LOCK_SUFFIX: &str = ".lock";
const DAEMON_POLICY_VERSION: &str = "beateros-policy-v0";

/// Runtime-store configuration.
#[derive(Debug, Clone)]
pub struct StoreOptions {
    /// Maximum time spent trying to acquire a per-session writer lock.
    pub lock_timeout: Duration,
    /// Sleep interval between bounded lock-acquire attempts.
    pub lock_poll_interval: Duration,
}

impl Default for StoreOptions {
    fn default() -> Self {
        Self {
            lock_timeout: Duration::from_secs(2),
            lock_poll_interval: Duration::from_millis(2),
        }
    }
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
    pub manifests: Vec<ActionManifest>,
    pub receipts: Vec<CapabilityReceipt>,
}

/// Result of a daemon-owned policy admission transaction.
#[derive(Debug, Clone)]
pub struct AdmissionOutcome {
    pub proposal_record: JournalRecord,
    pub decision_record: JournalRecord,
    pub decision: PolicyDecision,
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
            JournalEvent::CapabilityGranted { .. } => {
                return Err(DaemonError::Refused(
                    "CapabilityGranted must be written through issue_grant".to_string(),
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
            _ => {}
        }
        self.with_session_lock(session_id, || {
            self.append_event_unlocked(session_id, event, created_at)
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
            let record = journal.append(JournalEvent::CapabilityGranted { grant }, created_at)?;
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
            let existing_proposal = admission_state.proposals.get(manifest.action_id.as_str());
            if admission_state
                .decided_actions
                .contains(manifest.action_id.as_str())
            {
                return Err(DaemonError::Refused(format!(
                    "action {} already has a policy decision",
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
                approvals: Vec::new(),
                simulations: Vec::new(),
                mandates: Vec::new(),
                revoked_handles: BTreeSet::new(),
            };
            let decision = PolicyEngine::new().admit(&manifest, &ctx)?;
            let mut records_to_write = Vec::new();
            let proposal_record = if let Some(proposal) = existing_proposal {
                proposal.record.clone()
            } else {
                let record = journal.append(JournalEvent::ActionProposed { manifest }, now)?;
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
            if !self.session_exists_unlocked(session_id)? {
                return Err(DaemonError::SessionNotFound(session_id.to_string()));
            }
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
        let mut manifests = Vec::new();
        let mut receipts = Vec::new();
        for record in journal.records() {
            match &record.event {
                JournalEvent::SessionCreated { session: created } => {
                    session = Some(created.clone());
                }
                JournalEvent::CapabilityGranted { grant } => grants.push(grant.clone()),
                JournalEvent::ActionProposed { manifest } => manifests.push(manifest.clone()),
                JournalEvent::ReceiptAppended { receipt } => receipts.push(receipt.clone()),
                JournalEvent::PolicyDecided { .. }
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
        Ok(SessionProjection {
            session,
            grants,
            manifests,
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

struct AdmissionState {
    session: AgentSession,
    grants: BTreeMap<String, CapabilityGrant>,
    proposals: BTreeMap<String, ProposedAction>,
    decided_actions: BTreeSet<String>,
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
    let mut proposals = BTreeMap::new();
    let mut decided_actions = BTreeSet::new();
    for record in journal.records() {
        match &record.event {
            JournalEvent::SessionCreated { session: created } => {
                session = Some(created.clone());
            }
            JournalEvent::CapabilityGranted { grant } => {
                grants.insert(grant.grant_id.clone(), grant.clone());
            }
            JournalEvent::ActionProposed { manifest } => {
                proposals.insert(
                    manifest.action_id.clone(),
                    ProposedAction {
                        record: record.clone(),
                        manifest: manifest.clone(),
                    },
                );
            }
            JournalEvent::PolicyDecided { decision } => {
                decided_actions.insert(decision.action_id.clone());
            }
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
        proposals,
        decided_actions,
    })
}
