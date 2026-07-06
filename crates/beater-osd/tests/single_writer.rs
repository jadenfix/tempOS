#![allow(clippy::expect_used, clippy::unwrap_used)]

use std::collections::BTreeSet;
use std::fs;
use std::path::PathBuf;
use std::sync::{Arc, Barrier};
use std::thread;
use std::time::{Duration, Instant};

use beater_os_core::{
    ActionKind, ActionManifest, AgentSession, Budget, CapabilityGrant, CapabilityReceiptInput,
    CapabilityScope, CapabilitySelector, DecisionResult, DelegationMode, GrantConstraints,
    JournalEvent, ResourceKind, RiskClass, SessionStatus, SideEffectClass,
};
use beater_osd::{DaemonError, Store, StoreOptions};
use chrono::{TimeDelta, Utc};
use uuid::Uuid;

struct TempDir {
    path: PathBuf,
}

impl TempDir {
    fn new(tag: &str) -> Self {
        let path = std::env::temp_dir().join(format!("beater-osd-{tag}-{}", Uuid::new_v4()));
        fs::create_dir_all(&path).unwrap();
        Self { path }
    }
}

impl Drop for TempDir {
    fn drop(&mut self) {
        let _ = fs::remove_dir_all(&self.path);
    }
}

fn session(root: &TempDir, id: &str) -> AgentSession {
    AgentSession {
        session_id: id.to_string(),
        created_at: Utc::now(),
        created_by: "human:owner".to_string(),
        agent_id: "agent:runtime".to_string(),
        workspace_id: "workspace:repo".to_string(),
        goal: "serialize journal writers".to_string(),
        constraints: Vec::new(),
        policy_profile: "default".to_string(),
        initial_capability_ids: (0..32).map(|index| format!("grant-{index}")).collect(),
        budget: Budget::default(),
        model_policy: Default::default(),
        memory_scope: None,
        journal_root: root.path.display().to_string(),
        status: SessionStatus::Running,
    }
}

fn grant(session_id: &str, index: usize) -> CapabilityGrant {
    CapabilityGrant {
        grant_id: format!("grant-{index}"),
        issuer: "human:owner".to_string(),
        holder: "agent:runtime".to_string(),
        session_id: session_id.to_string(),
        parent_grant_id: None,
        scope: CapabilityScope {
            selector: CapabilitySelector {
                resource_kind: ResourceKind::FilePath,
                resource_id: "/workspace/out".to_string(),
            },
            actions: BTreeSet::from([ActionKind::Read, ActionKind::Write]),
        },
        denied_actions: BTreeSet::new(),
        constraints: GrantConstraints::default(),
        expires_at: Utc::now() + TimeDelta::hours(1),
        delegation: DelegationMode::None,
        approval: Default::default(),
        revocation_handle: format!("revoke-{index}"),
        policy_version: "beateros-policy-v0".to_string(),
        reason: "test grant".to_string(),
        revoked: false,
    }
}

fn receipt_input(action_id: &str) -> CapabilityReceiptInput {
    CapabilityReceiptInput {
        receipt_id: Some(format!("receipt-{action_id}")),
        action_id: action_id.to_string(),
        tool_id: "tool:test".to_string(),
        target: CapabilitySelector {
            resource_kind: ResourceKind::FilePath,
            resource_id: "/workspace/out".to_string(),
        },
        started_at: Utc::now(),
        finished_at: Utc::now(),
        status: "ok".to_string(),
        input_digest: "input-digest".to_string(),
        output_digest: "output-digest".to_string(),
        side_effect_summary: "test receipt".to_string(),
        side_effects: Vec::new(),
        external_ids: Vec::new(),
        artifact_refs: Vec::new(),
    }
}

fn manifest(session_id: &str, action_id: &str) -> ActionManifest {
    ActionManifest {
        action_id: action_id.to_string(),
        session_id: session_id.to_string(),
        tool_id: "tool:test".to_string(),
        action_kind: ActionKind::Write,
        target: CapabilitySelector {
            resource_kind: ResourceKind::FilePath,
            resource_id: "/workspace/out".to_string(),
        },
        resolved_target: Some(CapabilitySelector {
            resource_kind: ResourceKind::FilePath,
            resource_id: "/workspace/out".to_string(),
        }),
        inputs_digest: "input-digest".to_string(),
        inputs_summary: "write output".to_string(),
        expected_outputs: Vec::new(),
        expected_side_effects: BTreeSet::from([SideEffectClass::LocalWrite]),
        required_grants: BTreeSet::from(["grant-0".to_string()]),
        requested_budget: Budget::default(),
        risk_class: RiskClass::Low,
        data_classes: BTreeSet::new(),
        taint: BTreeSet::new(),
        idempotency_key: Some(format!("idem-{action_id}")),
        payment_intent: None,
        compensation_plan: None,
        human_explanation: "test action".to_string(),
    }
}

fn append_allowed_action(store: &Store, session_id: &str, action_id: &str) {
    let manifest = manifest(session_id, action_id);
    store
        .issue_grant(session_id, grant(session_id, 0), Utc::now())
        .unwrap();
    let outcome = store.admit_action(session_id, manifest).unwrap();
    assert_eq!(outcome.decision.result, DecisionResult::Allowed);
}

#[test]
fn concurrent_appends_serialize_without_forking_the_chain() {
    let root = TempDir::new("single-writer");
    let store = Arc::new(Store::open(&root.path).unwrap());
    let session_id = "sess_concurrent";
    store.create_session(&session(&root, session_id)).unwrap();

    let writers = 16;
    let barrier = Arc::new(Barrier::new(writers));
    let mut handles = Vec::new();
    for index in 0..writers {
        let store = Arc::clone(&store);
        let barrier = Arc::clone(&barrier);
        handles.push(thread::spawn(move || {
            barrier.wait();
            store.issue_grant(session_id, grant(session_id, index), Utc::now())
        }));
    }

    for handle in handles {
        handle
            .join()
            .expect("writer thread should not panic")
            .unwrap();
    }

    let journal = store.load_journal(session_id).unwrap();
    let report = journal.verify_chain().unwrap();
    assert_eq!(report.records, writers + 1);

    let projection = store.project(session_id).unwrap();
    assert_eq!(projection.grants.len(), writers);
}

#[test]
fn held_session_lock_times_out_fail_closed() {
    let root = TempDir::new("timeout");
    let store = Store::open_with_options(
        &root.path,
        StoreOptions {
            lock_timeout: Duration::from_millis(10),
            lock_poll_interval: Duration::from_millis(1),
            ..StoreOptions::default()
        },
    )
    .unwrap();
    let session_id = "sess_locked";
    let lock_path = root
        .path
        .join("sessions")
        .join(format!("{session_id}.lock"));
    fs::create_dir(&lock_path).unwrap();

    let result = store.append_event(
        session_id,
        JournalEvent::IncidentAnnotated {
            incident_id: "incident-lock".to_string(),
            note: "blocked on held lock".to_string(),
        },
        Utc::now(),
    );

    assert!(matches!(result, Err(DaemonError::LockTimeout(id)) if id == session_id));
}

#[test]
fn held_session_lock_timeout_is_not_extended_by_poll_interval() {
    let root = TempDir::new("timeout-bound");
    let store = Store::open_with_options(
        &root.path,
        StoreOptions {
            lock_timeout: Duration::from_millis(10),
            lock_poll_interval: Duration::from_secs(60),
            ..StoreOptions::default()
        },
    )
    .unwrap();
    let session_id = "sess_oversized_poll";
    let lock_path = root
        .path
        .join("sessions")
        .join(format!("{session_id}.lock"));
    fs::create_dir(&lock_path).unwrap();

    let started = Instant::now();
    let result = store.append_event(
        session_id,
        JournalEvent::IncidentAnnotated {
            incident_id: "incident-lock-bound".to_string(),
            note: "blocked on held lock".to_string(),
        },
        Utc::now(),
    );

    assert!(matches!(result, Err(DaemonError::LockTimeout(id)) if id == session_id));
    assert!(
        started.elapsed() < Duration::from_millis(250),
        "lock wait exceeded the configured timeout bound"
    );
}

#[test]
fn receipt_ledger_is_projected_from_journal_not_receipt_cache() {
    let root = TempDir::new("receipt-source");
    let store = Store::open(&root.path).unwrap();
    let session_id = "sess_receipts";
    store.create_session(&session(&root, session_id)).unwrap();
    append_allowed_action(&store, session_id, "act-1");

    let receipt = store
        .append_receipt(session_id, receipt_input("act-1"), Utc::now())
        .unwrap();
    let receipt_cache = root
        .path
        .join("sessions")
        .join(session_id)
        .join("receipts.jsonl");
    fs::write(&receipt_cache, "not valid json\n").unwrap();

    let ledger = store.load_receipts(session_id).unwrap();
    assert_eq!(ledger.receipts().len(), 1);
    assert_eq!(ledger.receipts()[0].receipt_id, receipt.receipt_id);
}

#[test]
fn raw_receipt_events_are_refused_by_public_append_event() {
    let root = TempDir::new("raw-receipt");
    let store = Store::open(&root.path).unwrap();
    let session_id = "sess_raw_receipt";
    store.create_session(&session(&root, session_id)).unwrap();
    append_allowed_action(&store, session_id, "act-1");
    let receipt = store
        .append_receipt(session_id, receipt_input("act-1"), Utc::now())
        .unwrap();

    let result = store.append_event(
        session_id,
        JournalEvent::ReceiptAppended { receipt },
        Utc::now(),
    );

    assert!(
        matches!(result, Err(DaemonError::Refused(message)) if message.contains("append_receipt"))
    );
}

#[test]
fn empty_partial_session_is_recoverable_during_create() {
    let root = TempDir::new("partial-create");
    let store = Store::open(&root.path).unwrap();
    let session_id = "sess_partial";
    let session_dir = root.path.join("sessions").join(session_id);
    fs::create_dir_all(&session_dir).unwrap();
    fs::write(session_dir.join("journal.jsonl"), "").unwrap();
    fs::write(session_dir.join("receipts.jsonl"), "").unwrap();

    store.create_session(&session(&root, session_id)).unwrap();

    let journal = store.load_journal(session_id).unwrap();
    assert_eq!(journal.records().len(), 1);
    assert!(matches!(
        &journal.records()[0].event,
        JournalEvent::SessionCreated { session } if session.session_id == session_id
    ));
}
