//! End-to-end tests for beater-os-audit.
//!
//! These build coherent journals through the real `beater-os-core` API (so the
//! hash chain and manifest binding are genuine) and then exercise the audit
//! crate: independent verification, trace rendering, metrics, and bundle export.
//! They also prove the independent checks catch integrity gaps that the core
//! chain verifier alone does not (missing session, missing grant, unexplained
//! denial, tampered hash).

use std::collections::BTreeSet;

use beater_os_core::{
    ActionKind, ActionManifest, ApprovalRequirement, BeaterOsError, Budget, CapabilityGrant,
    CapabilityReceipt, CapabilityReceiptInput, CapabilityScope, CapabilitySelector, DecisionResult,
    DelegationMode, GrantConstraints, InMemoryJournal, JournalEvent, JournalSnapshot, ModelPolicy,
    PolicyDecision, ReceiptLedger, ResourceKind, RiskClass, SessionStatus,
};
use chrono::{DateTime, Utc};

use beater_os_audit::{build_bundle, compute_metrics, render_trace, verify_snapshot};

fn ts(secs: i64) -> DateTime<Utc> {
    DateTime::from_timestamp(secs, 0).unwrap_or_else(Utc::now)
}

fn session(id: &str) -> beater_os_core::AgentSession {
    beater_os_core::AgentSession {
        session_id: id.to_string(),
        created_at: ts(1_000),
        created_by: "user:alice".to_string(),
        agent_id: "agent:coder".to_string(),
        workspace_id: "ws:demo".to_string(),
        goal: "read a file in the workspace".to_string(),
        constraints: Vec::new(),
        policy_profile: "default".to_string(),
        initial_capability_ids: BTreeSet::new(),
        budget: Budget::default(),
        model_policy: ModelPolicy::default(),
        memory_scope: None,
        journal_root: "root".to_string(),
        status: SessionStatus::Running,
    }
}

fn grant(id: &str, session_id: &str, holder: &str) -> CapabilityGrant {
    CapabilityGrant {
        grant_id: id.to_string(),
        issuer: "user:alice".to_string(),
        holder: holder.to_string(),
        session_id: session_id.to_string(),
        parent_grant_id: None,
        scope: CapabilityScope {
            selector: CapabilitySelector {
                resource_kind: ResourceKind::FilePath,
                resource_id: "/ws/demo".to_string(),
            },
            actions: BTreeSet::from([ActionKind::Read]),
        },
        denied_actions: BTreeSet::new(),
        constraints: GrantConstraints::default(),
        expires_at: ts(2_000_000),
        delegation: DelegationMode::None,
        approval: ApprovalRequirement::default(),
        revocation_handle: "rev-1".to_string(),
        policy_version: "v1".to_string(),
        reason: "read workspace file".to_string(),
        revoked: false,
    }
}

fn manifest(action_id: &str, session_id: &str, tool: &str, grants: &[&str]) -> ActionManifest {
    ActionManifest {
        action_id: action_id.to_string(),
        session_id: session_id.to_string(),
        tool_id: tool.to_string(),
        action_kind: ActionKind::Read,
        target: CapabilitySelector {
            resource_kind: ResourceKind::FilePath,
            resource_id: "/ws/demo/file.txt".to_string(),
        },
        resolved_target: None,
        inputs_digest: "digest:inputs".to_string(),
        inputs_summary: "read /ws/demo/file.txt".to_string(),
        expected_outputs: Vec::new(),
        expected_side_effects: BTreeSet::new(),
        required_grants: grants.iter().map(|g| g.to_string()).collect(),
        requested_budget: Budget::default(),
        risk_class: RiskClass::Low,
        data_classes: BTreeSet::new(),
        taint: BTreeSet::new(),
        idempotency_key: None,
        payment_intent: None,
        compensation_plan: None,
        human_explanation: "read a workspace file".to_string(),
    }
}

fn decision(
    id: &str,
    manifest: &ActionManifest,
    result: DecisionResult,
    explanation: &str,
) -> Result<PolicyDecision, BeaterOsError> {
    Ok(PolicyDecision {
        decision_id: id.to_string(),
        action_id: manifest.action_id.clone(),
        manifest_hash: manifest.digest()?,
        policy_version: "v1".to_string(),
        result,
        matched_rules: vec!["admitted_by_capability_policy".to_string()],
        explanation: explanation.to_string(),
        required_review: None,
        required_simulation: None,
        created_at: ts(1_002),
    })
}

// Build a receipt through ReceiptLedger so it carries a valid, hash-linked
// `receipt_hash` (the core journal verifier recomputes it during admission).
fn receipt(id: &str, manifest: &ActionManifest) -> Result<CapabilityReceipt, BeaterOsError> {
    let mut ledger = ReceiptLedger::new();
    ledger.append(CapabilityReceiptInput {
        receipt_id: Some(id.to_string()),
        action_id: manifest.action_id.clone(),
        tool_id: manifest.tool_id.clone(),
        target: manifest.target.clone(),
        started_at: ts(1_003),
        finished_at: ts(1_004),
        status: "ok".to_string(),
        input_digest: manifest.inputs_digest.clone(),
        output_digest: "digest:output".to_string(),
        side_effect_summary: "read completed".to_string(),
        side_effects: Vec::new(),
        external_ids: Vec::new(),
        artifact_refs: Vec::new(),
    })
}

/// A full, valid session → grant → action → decision → receipt trace.
fn valid_snapshot() -> Result<JournalSnapshot, BeaterOsError> {
    let mut journal = InMemoryJournal::new();
    journal.append(
        JournalEvent::SessionCreated {
            session: session("S1"),
        },
        ts(1_000),
    )?;
    journal.append(
        JournalEvent::CapabilityGranted {
            grant: grant("G1", "S1", "agent:coder"),
        },
        ts(1_001),
    )?;
    let m = manifest("A1", "S1", "tool:fs", &["G1"]);
    journal.append(
        JournalEvent::ActionProposed {
            manifest: Box::new(m.clone()),
        },
        ts(1_002),
    )?;
    journal.append(
        JournalEvent::PolicyDecided {
            decision: decision("D1", &m, DecisionResult::Allowed, "admitted by grant G1")?,
        },
        ts(1_002),
    )?;
    journal.append(
        JournalEvent::ReceiptAppended {
            receipt: receipt("R1", &m)?,
        },
        ts(1_004),
    )?;
    Ok(journal.snapshot())
}

#[test]
fn valid_trace_passes_every_independent_check() -> Result<(), BeaterOsError> {
    let snapshot = valid_snapshot()?;
    let report = verify_snapshot(&snapshot);
    assert!(
        report.ok,
        "expected pass, failures: {:?}",
        report.failures().collect::<Vec<_>>()
    );
    assert_eq!(report.records, 5);
    assert_eq!(report.checks.len(), 8);
    assert_eq!(report.failures().count(), 0);
    Ok(())
}

#[test]
fn metrics_report_full_coverage_for_valid_trace() -> Result<(), BeaterOsError> {
    let snapshot = valid_snapshot()?;
    let metrics = compute_metrics(&snapshot);
    assert_eq!(metrics.sessions, 1);
    assert_eq!(metrics.grants, 1);
    assert_eq!(metrics.actions_proposed, 1);
    assert_eq!(metrics.decisions, 1);
    assert_eq!(metrics.allowed_actions, 1);
    assert_eq!(metrics.receipts, 1);
    assert!(metrics.decision_coverage.is_complete());
    assert!(metrics.receipt_coverage.is_complete());
    assert!(metrics.denial_explanation_coverage.is_complete());
    Ok(())
}

#[test]
fn trace_render_and_bundle_are_coherent() -> Result<(), BeaterOsError> {
    let snapshot = valid_snapshot()?;
    let rendered = render_trace(&snapshot);
    assert!(rendered.contains("session=S1"));
    assert!(rendered.contains("action=A1"));
    assert!(rendered.contains("decision=D1"));
    assert!(rendered.contains("receipt=R1"));

    let bundle = build_bundle(&snapshot);
    assert_eq!(bundle.records, 5);
    assert!(bundle.report.ok);
    assert_eq!(bundle.record_digests.len(), 5);
    // The bundle carries hashes and kinds but not raw event payloads.
    let json = beater_os_audit::bundle_to_json(&bundle)?;
    assert!(json.contains("session_created"));
    assert!(!json.contains("read /ws/demo/file.txt")); // inputs_summary is not exported
    Ok(())
}

#[test]
fn detects_grant_used_before_it_was_issued() -> Result<(), BeaterOsError> {
    // A manifest requires G-missing that was never granted. The core chain
    // verifier accepts this (no decision/receipt), but the audit crate must not.
    let mut journal = InMemoryJournal::new();
    journal.append(
        JournalEvent::SessionCreated {
            session: session("S1"),
        },
        ts(1_000),
    )?;
    journal.append(
        JournalEvent::ActionProposed {
            manifest: Box::new(manifest("A1", "S1", "tool:fs", &["G-missing"])),
        },
        ts(1_001),
    )?;
    let snapshot = journal.snapshot();
    let report = verify_snapshot(&snapshot);
    assert!(!report.ok);
    let failed: BTreeSet<&str> = report.failures().map(|c| c.check.as_str()).collect();
    assert!(failed.contains("grant_references"));
    // The core cryptographic chain accepts this journal; only the independent
    // layer catches it. Assert that explicitly so the claim can't silently rot.
    assert!(!failed.contains("cryptographic_chain"));
    Ok(())
}

#[test]
fn detects_grant_for_unknown_session() -> Result<(), BeaterOsError> {
    let mut journal = InMemoryJournal::new();
    journal.append(
        JournalEvent::CapabilityGranted {
            grant: grant("G1", "S-unknown", "agent:coder"),
        },
        ts(1_000),
    )?;
    let snapshot = journal.snapshot();
    let report = verify_snapshot(&snapshot);
    assert!(!report.ok);
    let failed: BTreeSet<&str> = report.failures().map(|c| c.check.as_str()).collect();
    assert!(failed.contains("referential_sessions"));
    assert!(!failed.contains("cryptographic_chain"));
    Ok(())
}

#[test]
fn detects_unexplained_denial() -> Result<(), BeaterOsError> {
    let mut journal = InMemoryJournal::new();
    journal.append(
        JournalEvent::SessionCreated {
            session: session("S1"),
        },
        ts(1_000),
    )?;
    journal.append(
        JournalEvent::CapabilityGranted {
            grant: grant("G1", "S1", "agent:coder"),
        },
        ts(1_001),
    )?;
    let m = manifest("A1", "S1", "tool:fs", &["G1"]);
    journal.append(
        JournalEvent::ActionProposed {
            manifest: Box::new(m.clone()),
        },
        ts(1_002),
    )?;
    journal.append(
        JournalEvent::PolicyDecided {
            decision: decision("D1", &m, DecisionResult::Denied, "   ")?,
        },
        ts(1_002),
    )?;
    let snapshot = journal.snapshot();
    let report = verify_snapshot(&snapshot);
    assert!(!report.ok);
    let failed: BTreeSet<&str> = report.failures().map(|c| c.check.as_str()).collect();
    assert!(failed.contains("denial_explained"));
    assert!(!failed.contains("cryptographic_chain"));
    Ok(())
}

#[test]
fn detects_tampered_record_hash() -> Result<(), BeaterOsError> {
    let mut snapshot = valid_snapshot()?;
    // Corrupt the content hash of the third record. This breaks both the core
    // cryptographic chain and the independent linkage check.
    if let Some(record) = snapshot.records.get_mut(2) {
        record.hash = "f".repeat(64);
    }
    let report = verify_snapshot(&snapshot);
    assert!(!report.ok);
    let failed: BTreeSet<&str> = report.failures().map(|c| c.check.as_str()).collect();
    assert!(failed.contains("cryptographic_chain"));
    assert!(failed.contains("hash_linkage"));
    Ok(())
}

#[test]
fn independent_recompute_detects_terminal_record_tamper() -> Result<(), BeaterOsError> {
    // Tamper a hashed field (`created_at`) of the LAST record while leaving its
    // stored `hash` and `prev_hash` intact. There is no successor record, so
    // `hash_linkage` cannot catch it — only an independent content-hash recompute
    // can. This is the blind spot that trusting `record.hash` + delegating to
    // core alone used to paper over.
    let mut snapshot = valid_snapshot()?;
    if let Some(last) = snapshot.records.last_mut() {
        last.created_at = ts(9_999);
    }

    let report = verify_snapshot(&snapshot);
    assert!(!report.ok);
    let failed: BTreeSet<&str> = report.failures().map(|c| c.check.as_str()).collect();
    // The independent recompute catches the terminal-record content tamper ...
    assert!(failed.contains("cryptographic_chain"));
    // ... while prev-hash linkage does not (no successor; stored hash unchanged).
    assert!(!failed.contains("hash_linkage"));
    // Assert the recompute branch specifically fired (not merely that the check
    // name failed): the detail must name the independent recompute. This is what
    // proves the independent path — not linkage or a delegated verifier — is
    // what caught the tamper.
    let detail = report
        .failures()
        .find(|c| c.check == "cryptographic_chain")
        .map(|c| c.detail.clone())
        .unwrap_or_default();
    assert!(
        detail.contains("independently recomputed"),
        "expected the independent recompute to fire, got: {detail}"
    );
    Ok(())
}

#[test]
fn detects_use_of_revoked_grant() -> Result<(), BeaterOsError> {
    // A grant is issued already revoked, then an action uses it. The core journal
    // verifier does not re-check grant validity; the audit's grant_validity must.
    let mut journal = InMemoryJournal::new();
    journal.append(
        JournalEvent::SessionCreated {
            session: session("S1"),
        },
        ts(1_000),
    )?;
    let mut g = grant("G1", "S1", "agent:coder");
    g.revoked = true;
    journal.append(JournalEvent::CapabilityGranted { grant: g }, ts(1_001))?;
    journal.append(
        JournalEvent::ActionProposed {
            manifest: Box::new(manifest("A1", "S1", "tool:fs", &["G1"])),
        },
        ts(1_002),
    )?;
    let snapshot = journal.snapshot();
    let report = verify_snapshot(&snapshot);
    assert!(!report.ok);
    let failed: BTreeSet<&str> = report.failures().map(|c| c.check.as_str()).collect();
    assert!(failed.contains("grant_validity"));
    assert!(!failed.contains("cryptographic_chain"));
    Ok(())
}

#[test]
fn detects_use_of_expired_grant() -> Result<(), BeaterOsError> {
    let mut journal = InMemoryJournal::new();
    journal.append(
        JournalEvent::SessionCreated {
            session: session("S1"),
        },
        ts(1_000),
    )?;
    let mut g = grant("G1", "S1", "agent:coder");
    g.expires_at = ts(1_001); // expires before the action at ts(1_002) uses it
    journal.append(JournalEvent::CapabilityGranted { grant: g }, ts(1_000))?;
    journal.append(
        JournalEvent::ActionProposed {
            manifest: Box::new(manifest("A1", "S1", "tool:fs", &["G1"])),
        },
        ts(1_002),
    )?;
    let snapshot = journal.snapshot();
    let report = verify_snapshot(&snapshot);
    assert!(!report.ok);
    let failed: BTreeSet<&str> = report.failures().map(|c| c.check.as_str()).collect();
    assert!(failed.contains("grant_validity"));
    assert!(!failed.contains("cryptographic_chain"));
    Ok(())
}
