//! Deterministic session lifecycle runtime for beaterOS (backlog slice 3).
//!
//! This crate is the `beater-osd` session state machine described in
//! `final.md` §7.2 (Agent Session), §10.1 (kernel daemon — "keep it small"),
//! §10.2 (Capability Service — "bind grants to sessions and principals"), and
//! §12.1 (`AgentSession`). It builds *on* the [`beater_os_core`] contracts and
//! journal; it does not redefine them. A [`Session`] pairs one core
//! [`AgentSession`] with its own hash-chained [`InMemoryJournal`] and enforces a
//! strict, model-free lifecycle: create → active, pause, resume, cancel.
//!
//! # Before Coding (SOTA systems checklist — `CLAUDE.md`)
//!
//! - **Critical path**: the lifecycle transition and grant-bind functions. Each
//!   is O(1) plus one journal append (one `sha2` hash over the record). There is
//!   no non-critical background path; the crate is a synchronous library.
//! - **Allocation / copy / syscall**: bounded. A transition clones the session
//!   snapshot once (to journal the new authoritative state) and appends one
//!   record. No syscalls, no I/O, no async. The journal is an append-only `Vec`;
//!   this crate never spawns an unbounded queue and never retries — a failed
//!   append is surfaced, not buffered.
//! - **Queue / retry bounds**: none. There is no queue and no retry loop; growth
//!   is exactly one journal record per accepted operation, driven by the caller.
//! - **Failure mode under overload**: fail-closed. Every illegal transition and
//!   every ill-formed grant bind returns a typed [`SessionError`] and leaves the
//!   session and its journal untouched. The runtime never panics and never
//!   silently degrades into an unsafe state (no ambient authority).
//! - **Security boundary + required evidence**: the state machine *is* the
//!   boundary. Authority (a bound grant) is refused on any non-active session,
//!   and a grant whose session/principal does not match is refused. Evidence:
//!   every accepted operation appends a [`JournalEvent`] to the tamper-evident
//!   chain **before** the new status becomes observable, and the chain still
//!   [`InMemoryJournal::verify_chain`]s clean afterward.
//! - **Language choice**: pure safe Rust. No `unsafe`, C, or assembly; the
//!   workspace forbids `unsafe_code`. Deterministic logic with no platform
//!   surface, so Rust is the obvious fit.
//! - **macOS impact**: none. No platform-specific mechanism, no I/O; identical
//!   behavior on macOS / Apple Silicon and elsewhere.
//! - **Local verification**: `cargo test -p beater-os-session`.
//!
//! # Journal-before-side-effects invariant
//!
//! For every transition and grant bind the runtime appends the journal record
//! first; only if the append succeeds does it mutate the observable
//! [`Session::status`] / bound-grant set. If the append fails, the operation
//! returns an error with no state change. Core exposes no
//! `SessionPaused`/`SessionResumed`/`SessionCanceled` event, so lifecycle
//! transitions are journaled as [`JournalEvent::SessionCreated`] carrying the
//! session snapshot at its *new* status — the closest existing variant; see the
//! `TODO(slice-3 follow-up)` in [`Session::transition`].

mod error;

use std::collections::BTreeSet;

use beater_os_core::{
    AgentSession, CapabilityGrant, InMemoryJournal, JournalEvent, JournalRecord, SessionStatus,
};
use chrono::{DateTime, Utc};

pub use crate::error::{SessionError, Transition};

/// Result alias for session runtime operations.
pub type SessionResult<T> = Result<T, SessionError>;

/// A live agent session: one [`AgentSession`] contract plus its own append-only,
/// hash-chained journal and the set of capability grants bound to it.
///
/// The wrapper owns the authoritative status. Callers construct the session
/// template (an [`AgentSession`]) from a goal and hand it to [`Session::create`];
/// from then on the runtime is the only writer of the status field, and every
/// write is journaled.
#[derive(Clone, Debug)]
pub struct Session {
    session: AgentSession,
    journal: InMemoryJournal,
    bound_grant_ids: BTreeSet<String>,
}

impl Session {
    /// Create a live session from a goal template.
    ///
    /// The incoming [`AgentSession`] is a template; `create` establishes a fresh
    /// runtime with an empty journal, forces the status to
    /// [`SessionStatus::Running`] (active), and appends a
    /// [`JournalEvent::SessionCreated`] as the genesis record (`final.md` §10.4
    /// "Session created"). Journaling happens before the session is returned, so
    /// no observable active session exists without a matching journal entry.
    pub fn create(mut session: AgentSession, now: DateTime<Utc>) -> SessionResult<Self> {
        session.status = SessionStatus::Running;
        let mut journal = InMemoryJournal::new();
        journal.append(
            JournalEvent::SessionCreated {
                session: session.clone(),
            },
            now,
        )?;
        Ok(Self {
            session,
            journal,
            bound_grant_ids: BTreeSet::new(),
        })
    }

    /// Pause an active session: `Running -> Paused`. Rejected from any other
    /// status.
    pub fn pause(&mut self, now: DateTime<Utc>) -> SessionResult<JournalRecord> {
        self.transition(Transition::Pause, now)
    }

    /// Resume a paused session: `Paused -> Running`. Rejected from any other
    /// status.
    pub fn resume(&mut self, now: DateTime<Utc>) -> SessionResult<JournalRecord> {
        self.transition(Transition::Resume, now)
    }

    /// Cancel a session into the terminal [`SessionStatus::Canceled`] state,
    /// legal from `Running` or `Paused`. A second cancel (or any transition out
    /// of a terminal state) is rejected as an illegal transition — cancellation
    /// is not idempotent; it is a one-shot, fail-closed terminal step.
    pub fn cancel(&mut self, now: DateTime<Utc>) -> SessionResult<JournalRecord> {
        self.transition(Transition::Cancel, now)
    }

    /// Bind a capability grant to this session and principal (`final.md` §10.2).
    ///
    /// Fail-closed refusals, checked in order:
    /// 1. the session must be active ([`SessionStatus::Running`]) — no authority
    ///    accrues to a paused or terminal session (§26 no ambient authority);
    /// 2. the grant must name this session;
    /// 3. the grant's holder must be this session's principal (its `agent_id`);
    /// 4. the grant must be active (not revoked or expired) at `now` (§26
    ///    revocation).
    ///
    /// On success it journals a [`JournalEvent::CapabilityGranted`] (before the
    /// grant is recorded as bound) and records the grant id.
    pub fn bind_grant(
        &mut self,
        grant: CapabilityGrant,
        now: DateTime<Utc>,
    ) -> SessionResult<JournalRecord> {
        if self.session.status != SessionStatus::Running {
            return Err(SessionError::SessionNotActive {
                status: self.session.status.clone(),
            });
        }
        if grant.session_id != self.session.session_id {
            return Err(SessionError::GrantSessionMismatch {
                grant_id: grant.grant_id,
                grant_session_id: grant.session_id,
                session_id: self.session.session_id.clone(),
            });
        }
        if grant.holder != self.session.agent_id {
            return Err(SessionError::GrantPrincipalMismatch {
                grant_id: grant.grant_id,
                holder: grant.holder,
                principal: self.session.agent_id.clone(),
            });
        }
        if !grant.is_active_at(now) {
            return Err(SessionError::GrantInactive {
                grant_id: grant.grant_id,
                now: now.to_rfc3339(),
            });
        }
        if self.bound_grant_ids.contains(&grant.grant_id) {
            return Err(SessionError::GrantAlreadyBound {
                grant_id: grant.grant_id,
            });
        }
        let grant_id = grant.grant_id.clone();
        let record = self
            .journal
            .append(JournalEvent::CapabilityGranted { grant }, now)?;
        self.bound_grant_ids.insert(grant_id);
        Ok(record)
    }

    /// The session's current authoritative status.
    pub fn status(&self) -> &SessionStatus {
        &self.session.status
    }

    /// The underlying [`AgentSession`] contract.
    pub fn agent_session(&self) -> &AgentSession {
        &self.session
    }

    /// The session identifier.
    pub fn session_id(&self) -> &str {
        &self.session.session_id
    }

    /// The append-only, hash-chained journal for this session.
    pub fn journal(&self) -> &InMemoryJournal {
        &self.journal
    }

    /// The set of capability grant ids currently bound to this session.
    pub fn bound_grant_ids(&self) -> &BTreeSet<String> {
        &self.bound_grant_ids
    }

    /// Apply a lifecycle transition: validate it (fail-closed), journal the new
    /// authoritative session state, then flip the observable status.
    fn transition(
        &mut self,
        transition: Transition,
        now: DateTime<Utc>,
    ) -> SessionResult<JournalRecord> {
        let next = next_status(transition, &self.session.status)?;
        let mut snapshot = self.session.clone();
        snapshot.status = next.clone();
        // TODO(slice-3 follow-up): core exposes no SessionPaused/SessionResumed/
        // SessionCanceled (or a generic SessionStatusChanged) JournalEvent, so a
        // lifecycle transition is recorded via the closest existing variant,
        // SessionCreated, carrying the session snapshot at its new status. The
        // full status is preserved and the chain replays cleanly; a dedicated
        // transition event would be more legible. Do not add it to core here.
        let record = self
            .journal
            .append(JournalEvent::SessionCreated { session: snapshot }, now)?;
        self.session.status = next;
        Ok(record)
    }
}

/// The strict lifecycle state machine.
///
/// Only the pairings enumerated here are legal; every other `(transition,
/// status)` combination — including transitions out of terminal states and a
/// second cancel — falls through to the fail-closed [`SessionError::IllegalTransition`].
fn next_status(transition: Transition, from: &SessionStatus) -> SessionResult<SessionStatus> {
    let next = match (transition, from) {
        (Transition::Pause, SessionStatus::Running) => SessionStatus::Paused,
        (Transition::Resume, SessionStatus::Paused) => SessionStatus::Running,
        (Transition::Cancel, SessionStatus::Running | SessionStatus::Paused) => {
            SessionStatus::Canceled
        }
        _ => {
            return Err(SessionError::IllegalTransition {
                transition,
                from: from.clone(),
            });
        }
    };
    Ok(next)
}
