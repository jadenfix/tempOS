use std::fs::{self, File, OpenOptions};
use std::io::Write;
use std::path::{Path, PathBuf};

use beater_os_core::{
    ActionManifest, AgentSession, CapabilityGrant, CapabilityReceipt, CapabilityReceiptInput,
    InMemoryJournal, JournalEvent, JournalRecord, PolicyDecision, ReceiptLedger,
};
use chrono::{DateTime, Utc};

use crate::error::{CliError, CliResult};

const SESSIONS_DIR: &str = "sessions";
const JOURNAL_FILE: &str = "journal.jsonl";
const RECEIPTS_FILE: &str = "receipts.jsonl";

/// A durable, local, append-only store for beaterOS sessions.
///
/// Each session owns a directory containing two newline-delimited JSON logs:
/// `journal.jsonl` (the hash-chained event journal) and `receipts.jsonl`
/// (the hash-chained side-effect receipt ledger). The store never mutates a
/// previously written line; new records are only ever appended. This is the
/// on-disk realization of the "journal before side effects, receipts after"
/// invariants from `final.md`.
///
/// Concurrency: this is a single-user, single-process operator tool. Appends
/// are read-len-then-append and are **not** guarded by a file lock, so two
/// concurrent processes writing the same session could fork a chain. Such a
/// fork is not silent — it fails `journal verify` — but a locking or
/// single-writer runtime (the `beater-osd` slice) should own concurrent access.
pub struct Store {
    root: PathBuf,
}

/// A read model projected from a session's append-only journal.
///
/// This is the event-sourced view the CLI renders and admits actions against.
/// It is rebuilt from the log on every command, so it can never drift from the
/// durable source of truth.
#[derive(Debug, Clone)]
pub struct SessionProjection {
    pub session: AgentSession,
    pub grants: Vec<CapabilityGrant>,
    pub manifests: Vec<ActionManifest>,
    pub decisions: Vec<PolicyDecision>,
    pub receipts: Vec<CapabilityReceipt>,
}

impl SessionProjection {
    /// Grants that are not revoked and have not expired at `now`.
    pub fn active_grants(&self, now: DateTime<Utc>) -> Vec<CapabilityGrant> {
        self.grants
            .iter()
            .filter(|grant| grant.is_active_at(now))
            .cloned()
            .collect()
    }

    /// The most recent policy decision recorded for `action_id`, if any.
    pub fn latest_decision(&self, action_id: &str) -> Option<&PolicyDecision> {
        self.decisions
            .iter()
            .rev()
            .find(|decision| decision.action_id == action_id)
    }

    /// The proposed manifest for `action_id`, if any.
    pub fn manifest(&self, action_id: &str) -> Option<&ActionManifest> {
        self.manifests
            .iter()
            .find(|manifest| manifest.action_id == action_id)
    }
}

impl Store {
    /// Open (creating if necessary) a store rooted at `root`.
    pub fn open(root: impl Into<PathBuf>) -> CliResult<Self> {
        let root = root.into();
        fs::create_dir_all(root.join(SESSIONS_DIR))?;
        Ok(Self { root })
    }

    pub fn root(&self) -> &Path {
        &self.root
    }

    /// Resolve a session's directory, rejecting any id that is not a single safe
    /// path segment. Session ids are attacker-controlled (`--session <id>` on
    /// every command), so this is the chokepoint that prevents path traversal:
    /// `--session ../../pwned` must never escape the store root.
    fn session_dir(&self, session_id: &str) -> CliResult<PathBuf> {
        validate_session_id(session_id)?;
        Ok(self.root.join(SESSIONS_DIR).join(session_id))
    }

    fn journal_path(&self, session_id: &str) -> CliResult<PathBuf> {
        Ok(self.session_dir(session_id)?.join(JOURNAL_FILE))
    }

    fn receipts_path(&self, session_id: &str) -> CliResult<PathBuf> {
        Ok(self.session_dir(session_id)?.join(RECEIPTS_FILE))
    }

    /// Whether a session with `session_id` exists on disk. An invalid id is not a
    /// session, so it reports `false` rather than erroring.
    pub fn session_exists(&self, session_id: &str) -> bool {
        self.journal_path(session_id)
            .map(|path| path.is_file())
            .unwrap_or(false)
    }

    /// List all session ids in the store, sorted.
    pub fn list_sessions(&self) -> CliResult<Vec<String>> {
        let dir = self.root.join(SESSIONS_DIR);
        let mut out = Vec::new();
        if !dir.is_dir() {
            return Ok(out);
        }
        for entry in fs::read_dir(&dir)? {
            let entry = entry?;
            if entry.path().join(JOURNAL_FILE).is_file()
                && let Some(name) = entry.file_name().to_str()
            {
                out.push(name.to_string());
            }
        }
        out.sort();
        Ok(out)
    }

    /// Create a new session and journal its `SessionCreated` event.
    pub fn create_session(&self, session: &AgentSession) -> CliResult<JournalRecord> {
        if self.session_exists(&session.session_id) {
            return Err(CliError::SessionExists(session.session_id.clone()));
        }
        fs::create_dir_all(self.session_dir(&session.session_id)?)?;
        // Create the logs up front so appends and loads have a stable target.
        File::create(self.journal_path(&session.session_id)?)?;
        File::create(self.receipts_path(&session.session_id)?)?;
        self.append_event(
            &session.session_id,
            JournalEvent::SessionCreated {
                session: session.clone(),
            },
            session.created_at,
        )
    }

    /// Load a session's journal into memory, reconstructing the hash chain.
    pub fn load_journal(&self, session_id: &str) -> CliResult<InMemoryJournal> {
        let path = self.journal_path(session_id)?;
        if !path.is_file() {
            return Err(CliError::SessionNotFound(session_id.to_string()));
        }
        let mut records = Vec::new();
        for line in fs::read_to_string(&path)?.lines() {
            let line = line.trim();
            if line.is_empty() {
                continue;
            }
            records.push(serde_json::from_str::<JournalRecord>(line)?);
        }
        let journal = InMemoryJournal::from_records(records);
        // Fail closed on any drift: re-verify the hash chain on every load so the
        // admission path (and every read model) can never operate on tampered or
        // corrupted journal state. This is the security boundary — a hand-edited
        // journal.jsonl must be rejected here, not only under `journal verify`.
        journal.verify_chain()?;
        Ok(journal)
    }

    /// Append an event to a session's journal and persist it.
    ///
    /// The core journal computes the seq and hash chain; the store persists the
    /// returned record verbatim so a reload reproduces an identical chain.
    pub fn append_event(
        &self,
        session_id: &str,
        event: JournalEvent,
        created_at: DateTime<Utc>,
    ) -> CliResult<JournalRecord> {
        if !self.session_exists(session_id) {
            return Err(CliError::SessionNotFound(session_id.to_string()));
        }
        let mut journal = self.load_journal(session_id)?;
        let record = journal.append(event, created_at)?;
        let line = serde_json::to_string(&record)?;
        let mut file = OpenOptions::new()
            .append(true)
            .open(self.journal_path(session_id)?)?;
        writeln!(file, "{line}")?;
        Ok(record)
    }

    /// Load a session's receipt ledger.
    pub fn load_receipts(&self, session_id: &str) -> CliResult<ReceiptLedger> {
        let path = self.receipts_path(session_id)?;
        if !path.is_file() {
            return Ok(ReceiptLedger::new());
        }
        let mut receipts = Vec::new();
        for line in fs::read_to_string(&path)?.lines() {
            let line = line.trim();
            if line.is_empty() {
                continue;
            }
            receipts.push(serde_json::from_str::<CapabilityReceipt>(line)?);
        }
        let ledger = ReceiptLedger::from_receipts(receipts);
        // Fail closed on receipt-chain drift, same rationale as `load_journal`.
        ledger.verify_chain()?;
        Ok(ledger)
    }

    /// Compute the next chained receipt for a session **without** writing it to
    /// disk. The caller is expected to journal the resulting `ReceiptAppended`
    /// event (the source of truth) and then call [`Store::persist_receipt`].
    pub fn stage_receipt(
        &self,
        session_id: &str,
        input: CapabilityReceiptInput,
    ) -> CliResult<CapabilityReceipt> {
        if !self.session_exists(session_id) {
            return Err(CliError::SessionNotFound(session_id.to_string()));
        }
        let mut ledger = self.load_receipts(session_id)?;
        Ok(ledger.append(input)?)
    }

    /// Persist a previously [`staged`](Store::stage_receipt) receipt as the next
    /// line of the session's receipt ledger.
    pub fn persist_receipt(&self, session_id: &str, receipt: &CapabilityReceipt) -> CliResult<()> {
        if !self.session_exists(session_id) {
            return Err(CliError::SessionNotFound(session_id.to_string()));
        }
        let line = serde_json::to_string(receipt)?;
        let mut file = OpenOptions::new()
            .append(true)
            .open(self.receipts_path(session_id)?)?;
        writeln!(file, "{line}")?;
        Ok(())
    }

    /// Rebuild the read model for a session from its journal.
    pub fn project(&self, session_id: &str) -> CliResult<SessionProjection> {
        let journal = self.load_journal(session_id)?;
        let mut session: Option<AgentSession> = None;
        let mut grants = Vec::new();
        let mut manifests = Vec::new();
        let mut decisions = Vec::new();
        let mut receipts = Vec::new();
        for record in journal.records() {
            match &record.event {
                JournalEvent::SessionCreated { session: created } => {
                    session = Some(created.clone());
                }
                JournalEvent::CapabilityGranted { grant } => grants.push(grant.clone()),
                JournalEvent::ActionProposed { manifest } => manifests.push(manifest.clone()),
                JournalEvent::PolicyDecided { decision } => decisions.push(decision.clone()),
                JournalEvent::ReceiptAppended { receipt } => receipts.push(receipt.clone()),
                JournalEvent::MemoryWritten { .. }
                | JournalEvent::ScenarioEvaluated { .. }
                | JournalEvent::IncidentAnnotated { .. } => {}
            }
        }
        let session = session.ok_or_else(|| {
            CliError::Refused(format!(
                "session {session_id} journal has no SessionCreated event"
            ))
        })?;
        Ok(SessionProjection {
            session,
            grants,
            manifests,
            decisions,
            receipts,
        })
    }
}

/// Reject any session id that is not a single safe path segment.
///
/// Session ids reach the store from `--session <id>` on every command, so an
/// unsanitized id is a path-traversal primitive (`../../pwned` would let a
/// command read or write outside the store root). We allow only ASCII
/// alphanumerics, `-`, and `_` — which admits the generated UUIDs and any
/// reasonable operator-chosen id — and reject everything else (path separators,
/// `.`/`..`, whitespace, control bytes) fail-closed.
fn validate_session_id(session_id: &str) -> CliResult<()> {
    let is_safe = !session_id.is_empty()
        && session_id
            .bytes()
            .all(|b| b.is_ascii_alphanumeric() || b == b'-' || b == b'_');
    if is_safe {
        Ok(())
    } else {
        Err(CliError::invalid("session", session_id))
    }
}
