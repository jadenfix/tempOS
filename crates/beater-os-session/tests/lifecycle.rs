//! Behavioral tests for the session lifecycle runtime.
//!
//! Coverage is deliberately negative-heavy: every illegal transition and every
//! ill-formed grant bind must be *rejected* and must leave the session and its
//! journal untouched, and the hash chain must stay clean throughout.
//!
//! Test code unwraps `Option`/`Result` freely to keep assertions terse; the
//! library crate itself uses no `unwrap`/`expect` (workspace lints deny them).
#![allow(clippy::unwrap_used)]

use std::collections::BTreeSet;

use beater_os_core::{
    ActionKind, AgentSession, Budget, CapabilityGrant, CapabilityScope, CapabilitySelector,
    DelegationMode, GrantConstraints, JournalEvent, ModelPolicy, ResourceKind, SessionStatus,
};
use beater_os_session::{Session, SessionError, Transition};
use chrono::{DateTime, Utc};

const AGENT: &str = "agent-1";
const SESSION_ID: &str = "session-1";

fn at(secs: i64) -> DateTime<Utc> {
    DateTime::from_timestamp(secs, 0).unwrap()
}

fn session_template() -> AgentSession {
    AgentSession {
        session_id: SESSION_ID.to_string(),
        created_at: at(1_000),
        created_by: "owner@example.com".to_string(),
        agent_id: AGENT.to_string(),
        workspace_id: "workspace-1".to_string(),
        goal: "refactor the parser".to_string(),
        constraints: Vec::new(),
        policy_profile: "default".to_string(),
        initial_capability_ids: BTreeSet::new(),
        budget: Budget::default(),
        model_policy: ModelPolicy::default(),
        memory_scope: None,
        journal_root: "genesis".to_string(),
        status: SessionStatus::Created,
    }
}

/// A grant that matches `session_id`/`holder` and is active well past `expires`.
fn grant_for(grant_id: &str, session_id: &str, holder: &str) -> CapabilityGrant {
    let mut actions = BTreeSet::new();
    actions.insert(ActionKind::Read);
    CapabilityGrant {
        grant_id: grant_id.to_string(),
        issuer: "beater-osd".to_string(),
        holder: holder.to_string(),
        session_id: session_id.to_string(),
        scope: CapabilityScope {
            selector: CapabilitySelector {
                resource_kind: ResourceKind::FilePath,
                resource_id: "/repo".to_string(),
            },
            actions,
        },
        denied_actions: BTreeSet::new(),
        constraints: GrantConstraints::default(),
        expires_at: at(1_000_000),
        delegation: DelegationMode::None,
        approval: Default::default(),
        revocation_handle: "revoke-1".to_string(),
        policy_version: "v1".to_string(),
        reason: "read repo".to_string(),
        revoked: false,
        parent_grant_id: None,
    }
}

fn active_session() -> Session {
    Session::create(session_template(), at(1_001)).unwrap()
}

/// Count how many journal events of each kind exist, for ordering assertions.
fn event_kinds(session: &Session) -> Vec<&'static str> {
    session
        .journal()
        .records()
        .iter()
        .map(|record| match record.event {
            JournalEvent::SessionCreated { .. } => "session_created",
            JournalEvent::SessionStatusChanged { .. } => "session_status_changed",
            JournalEvent::CapabilityGranted { .. } => "capability_granted",
            _ => "other",
        })
        .collect()
}

// --- create -----------------------------------------------------------------

#[test]
fn create_yields_active_session_with_genesis_journal() {
    let session = active_session();
    assert_eq!(*session.status(), SessionStatus::Running);
    assert_eq!(session.session_id(), SESSION_ID);
    assert_eq!(session.journal().records().len(), 1);
    assert_eq!(event_kinds(&session), vec!["session_created"]);
    // The journaled snapshot carries the active status, not the template's.
    if let JournalEvent::SessionCreated { session: snap } = &session.journal().records()[0].event {
        assert_eq!(snap.status, SessionStatus::Running);
    } else {
        panic!("first event must be session_created");
    }
    assert!(session.journal().verify_chain().is_ok());
}

// --- legal transition paths -------------------------------------------------

#[test]
fn full_legal_lifecycle_is_journaled_in_order_and_verifies() {
    let mut session = active_session();
    session.pause(at(1_002)).unwrap();
    assert_eq!(*session.status(), SessionStatus::Paused);
    session.resume(at(1_003)).unwrap();
    assert_eq!(*session.status(), SessionStatus::Running);
    session.cancel(at(1_004)).unwrap();
    assert_eq!(*session.status(), SessionStatus::Canceled);

    // One genesis + three explicit transition records.
    assert_eq!(
        event_kinds(&session),
        vec![
            "session_created",
            "session_status_changed",
            "session_status_changed",
            "session_status_changed"
        ]
    );
    // The genesis records the initial running status; each transition records
    // the next status the session held after that step.
    let statuses: Vec<SessionStatus> = session
        .journal()
        .records()
        .iter()
        .filter_map(|record| match &record.event {
            JournalEvent::SessionCreated { session } => Some(session.status.clone()),
            JournalEvent::SessionStatusChanged { to, .. } => Some(to.clone()),
            _ => None,
        })
        .collect();
    assert_eq!(
        statuses,
        vec![
            SessionStatus::Running,
            SessionStatus::Paused,
            SessionStatus::Running,
            SessionStatus::Canceled,
        ]
    );
    // Seqs are contiguous and the chain is intact.
    let seqs: Vec<u64> = session.journal().records().iter().map(|r| r.seq).collect();
    assert_eq!(seqs, vec![0, 1, 2, 3]);
    assert!(session.journal().verify_chain().is_ok());
}

#[test]
fn cancel_from_paused_is_legal() {
    let mut session = active_session();
    session.pause(at(1_002)).unwrap();
    session.cancel(at(1_003)).unwrap();
    assert_eq!(*session.status(), SessionStatus::Canceled);
    assert!(session.journal().verify_chain().is_ok());
}

// --- every illegal transition is rejected, fail-closed ----------------------

#[test]
fn pause_from_paused_is_rejected() {
    let mut session = active_session();
    session.pause(at(1_002)).unwrap();
    let before = session.journal().records().len();
    let err = session.pause(at(1_003)).unwrap_err();
    assert!(matches!(
        err,
        SessionError::IllegalTransition {
            transition: Transition::Pause,
            from: SessionStatus::Paused
        }
    ));
    // Fail-closed: status and journal are untouched.
    assert_eq!(*session.status(), SessionStatus::Paused);
    assert_eq!(session.journal().records().len(), before);
    assert!(session.journal().verify_chain().is_ok());
}

#[test]
fn resume_from_running_is_rejected() {
    let mut session = active_session();
    let err = session.resume(at(1_002)).unwrap_err();
    assert!(matches!(
        err,
        SessionError::IllegalTransition {
            transition: Transition::Resume,
            from: SessionStatus::Running
        }
    ));
    assert_eq!(*session.status(), SessionStatus::Running);
    assert_eq!(session.journal().records().len(), 1);
}

#[test]
fn pause_from_terminal_is_rejected() {
    let mut session = active_session();
    session.cancel(at(1_002)).unwrap();
    let err = session.pause(at(1_003)).unwrap_err();
    assert!(matches!(
        err,
        SessionError::IllegalTransition {
            transition: Transition::Pause,
            from: SessionStatus::Canceled
        }
    ));
    assert_eq!(*session.status(), SessionStatus::Canceled);
}

#[test]
fn resume_from_terminal_is_rejected() {
    let mut session = active_session();
    session.cancel(at(1_002)).unwrap();
    let err = session.resume(at(1_003)).unwrap_err();
    assert!(matches!(
        err,
        SessionError::IllegalTransition {
            transition: Transition::Resume,
            from: SessionStatus::Canceled
        }
    ));
    assert_eq!(*session.status(), SessionStatus::Canceled);
}

#[test]
fn double_cancel_is_rejected_and_leaves_journal_clean() {
    let mut session = active_session();
    session.cancel(at(1_002)).unwrap();
    let after_first = session.journal().records().len();
    let err = session.cancel(at(1_003)).unwrap_err();
    assert!(matches!(
        err,
        SessionError::IllegalTransition {
            transition: Transition::Cancel,
            from: SessionStatus::Canceled
        }
    ));
    // Cancellation is one-shot, not idempotent: no extra record, chain intact.
    assert_eq!(*session.status(), SessionStatus::Canceled);
    assert_eq!(session.journal().records().len(), after_first);
    assert!(session.journal().verify_chain().is_ok());
}

// --- grant binding: happy path ----------------------------------------------

#[test]
fn bind_grant_success_is_journaled_and_recorded() {
    let mut session = active_session();
    let record = session
        .bind_grant(grant_for("grant-1", SESSION_ID, AGENT), at(1_002))
        .unwrap();
    assert!(matches!(
        record.event,
        JournalEvent::CapabilityGranted { .. }
    ));
    assert!(session.bound_grant_ids().contains("grant-1"));
    assert_eq!(
        event_kinds(&session),
        vec!["session_created", "capability_granted"]
    );
    assert!(session.journal().verify_chain().is_ok());
}

#[test]
fn rebinding_same_grant_is_rejected_and_journal_unchanged() {
    let mut session = active_session();
    session
        .bind_grant(grant_for("grant-1", SESSION_ID, AGENT), at(1_002))
        .unwrap();
    let len_after_first = session.journal().records().len();
    // A second bind of the same grant id must fail closed, not silently dedupe,
    // so the journal never carries a duplicate CapabilityGranted.
    let err = session
        .bind_grant(grant_for("grant-1", SESSION_ID, AGENT), at(1_003))
        .unwrap_err();
    assert!(matches!(err, SessionError::GrantAlreadyBound { .. }));
    assert_eq!(session.journal().records().len(), len_after_first);
    assert_eq!(session.bound_grant_ids().len(), 1);
    assert!(session.journal().verify_chain().is_ok());
}

// --- grant binding: fail-closed rejection cases -----------------------------

#[test]
fn bind_grant_wrong_session_is_rejected() {
    let mut session = active_session();
    let err = session
        .bind_grant(grant_for("grant-x", "other-session", AGENT), at(1_002))
        .unwrap_err();
    assert!(matches!(err, SessionError::GrantSessionMismatch { .. }));
    assert!(session.bound_grant_ids().is_empty());
    assert_eq!(session.journal().records().len(), 1);
    assert!(session.journal().verify_chain().is_ok());
}

#[test]
fn bind_grant_wrong_principal_is_rejected() {
    let mut session = active_session();
    let err = session
        .bind_grant(grant_for("grant-x", SESSION_ID, "other-agent"), at(1_002))
        .unwrap_err();
    assert!(matches!(err, SessionError::GrantPrincipalMismatch { .. }));
    assert!(session.bound_grant_ids().is_empty());
    assert_eq!(session.journal().records().len(), 1);
}

#[test]
fn bind_grant_on_paused_session_is_rejected() {
    let mut session = active_session();
    session.pause(at(1_002)).unwrap();
    let before = session.journal().records().len();
    let err = session
        .bind_grant(grant_for("grant-1", SESSION_ID, AGENT), at(1_003))
        .unwrap_err();
    assert!(matches!(
        err,
        SessionError::SessionNotActive {
            status: SessionStatus::Paused
        }
    ));
    assert!(session.bound_grant_ids().is_empty());
    assert_eq!(session.journal().records().len(), before);
}

#[test]
fn bind_grant_on_terminal_session_is_rejected() {
    let mut session = active_session();
    session.cancel(at(1_002)).unwrap();
    let err = session
        .bind_grant(grant_for("grant-1", SESSION_ID, AGENT), at(1_003))
        .unwrap_err();
    assert!(matches!(
        err,
        SessionError::SessionNotActive {
            status: SessionStatus::Canceled
        }
    ));
    assert!(session.bound_grant_ids().is_empty());
}

#[test]
fn bind_revoked_grant_is_rejected() {
    let mut session = active_session();
    let mut grant = grant_for("grant-1", SESSION_ID, AGENT);
    grant.revoked = true;
    let err = session.bind_grant(grant, at(1_002)).unwrap_err();
    assert!(matches!(err, SessionError::GrantInactive { .. }));
    assert!(session.bound_grant_ids().is_empty());
    assert_eq!(session.journal().records().len(), 1);
}

#[test]
fn bind_expired_grant_is_rejected() {
    let mut session = active_session();
    let mut grant = grant_for("grant-1", SESSION_ID, AGENT);
    grant.expires_at = at(500); // already past at bind time
    let err = session.bind_grant(grant, at(1_002)).unwrap_err();
    assert!(matches!(err, SessionError::GrantInactive { .. }));
    assert!(session.bound_grant_ids().is_empty());
}

// --- integrated journal ordering across the whole surface -------------------

#[test]
fn interleaved_grant_and_lifecycle_preserve_order_and_chain() {
    let mut session = active_session();
    session
        .bind_grant(grant_for("grant-1", SESSION_ID, AGENT), at(1_002))
        .unwrap();
    session.pause(at(1_003)).unwrap();
    session.resume(at(1_004)).unwrap();
    session
        .bind_grant(grant_for("grant-2", SESSION_ID, AGENT), at(1_005))
        .unwrap();
    session.cancel(at(1_006)).unwrap();

    assert_eq!(
        event_kinds(&session),
        vec![
            "session_created",        // create
            "capability_granted",     // grant-1
            "session_status_changed", // pause
            "session_status_changed", // resume
            "capability_granted",     // grant-2
            "session_status_changed", // cancel
        ]
    );
    let mut expected_grants = BTreeSet::new();
    expected_grants.insert("grant-1".to_string());
    expected_grants.insert("grant-2".to_string());
    assert_eq!(session.bound_grant_ids(), &expected_grants);

    let report = session.journal().verify_chain().unwrap();
    assert_eq!(report.records, 6);
}
