use std::collections::{BTreeMap, BTreeSet};

use chrono::{DateTime, TimeDelta, Utc};
use serde::{Deserialize, Serialize};

use crate::contracts::{
    ActionKind, ActionManifest, AgentSession, ApprovalEvidence, CapabilityGrant, DecisionResult,
    ExecutionLease, ExecutionLeaseHeartbeat, ExecutionLeaseReconciliation, MemoryRecord,
    PaymentIntent, PaymentMandate, PolicyDecision, ScenarioManifest, SessionStatus,
    SideEffectClass, SimulationEvidence,
};
use crate::error::{BeaterOsError, BeaterOsResult};
use crate::hash::{GENESIS_HASH, HashValue, hash_json};
use crate::receipt::{
    CapabilityReceipt, PaymentReceiptEvidence, PaymentSettlementStatus, ReceiptLedger,
};

const EXECUTION_LEASE_OVERHEAD_GRACE_MS: u64 = 2_000;

#[derive(Clone, Debug, PartialEq, Eq, Serialize, Deserialize)]
#[serde(tag = "kind", rename_all = "snake_case")]
pub enum JournalEvent {
    SessionCreated {
        session: AgentSession,
    },
    SessionStatusChanged {
        transition_id: String,
        session_id: String,
        from: SessionStatus,
        to: SessionStatus,
    },
    CapabilityGranted {
        grant: CapabilityGrant,
    },
    CapabilityRevoked {
        grant_id: String,
        revocation_handle: String,
        revoked_by: String,
        reason: String,
    },
    PaymentMandateIssued {
        mandate: PaymentMandate,
    },
    ActionProposed {
        manifest: Box<ActionManifest>,
    },
    PolicyDecided {
        decision: PolicyDecision,
    },
    ExecutionLeaseIssued {
        lease: ExecutionLease,
    },
    ExecutionLeaseHeartbeated {
        heartbeat: ExecutionLeaseHeartbeat,
    },
    ExecutionLeaseReconciled {
        reconciliation: ExecutionLeaseReconciliation,
    },
    ApprovalRecorded {
        approval: ApprovalEvidence,
    },
    SimulationRecorded {
        simulation: SimulationEvidence,
    },
    ReceiptAppended {
        receipt: CapabilityReceipt,
    },
    MemoryWritten {
        memory: MemoryRecord,
    },
    ScenarioEvaluated {
        scenario: ScenarioManifest,
        passed: bool,
    },
    IncidentAnnotated {
        incident_id: String,
        note: String,
    },
}

#[derive(Clone, Debug, PartialEq, Eq, Serialize, Deserialize)]
pub struct JournalRecord {
    pub seq: u64,
    pub created_at: DateTime<Utc>,
    pub event: JournalEvent,
    pub prev_hash: HashValue,
    pub hash: HashValue,
}

#[derive(Serialize)]
struct JournalHashView<'a> {
    seq: u64,
    created_at: &'a DateTime<Utc>,
    event: &'a JournalEvent,
    prev_hash: &'a HashValue,
}

impl<'a> From<&'a JournalRecord> for JournalHashView<'a> {
    fn from(record: &'a JournalRecord) -> Self {
        Self {
            seq: record.seq,
            created_at: &record.created_at,
            event: &record.event,
            prev_hash: &record.prev_hash,
        }
    }
}

#[derive(Clone, Debug, Default, PartialEq, Eq, Serialize, Deserialize)]
pub struct JournalSnapshot {
    pub records: Vec<JournalRecord>,
}

#[derive(Clone, Debug, PartialEq, Eq, Serialize, Deserialize)]
pub struct JournalVerificationReport {
    pub records: usize,
    pub root_hash: HashValue,
}

#[derive(Clone, Debug, Default)]
pub struct InMemoryJournal {
    records: Vec<JournalRecord>,
}

impl InMemoryJournal {
    pub fn new() -> Self {
        Self {
            records: Vec::new(),
        }
    }

    pub fn from_records(records: Vec<JournalRecord>) -> Self {
        Self { records }
    }

    pub fn append(
        &mut self,
        event: JournalEvent,
        created_at: DateTime<Utc>,
    ) -> BeaterOsResult<JournalRecord> {
        let seq = self.records.len() as u64;
        if let JournalEvent::MemoryWritten { memory } = &event {
            let known_event_ids = self.records.iter().filter_map(primary_event_id);
            validate_memory_source(memory, seq, known_event_ids)?;
        }
        let prev_hash = self
            .records
            .last()
            .map(|record| record.hash.clone())
            .unwrap_or_else(|| GENESIS_HASH.to_string());
        let mut record = JournalRecord {
            seq,
            created_at,
            event,
            prev_hash,
            hash: String::new(),
        };
        record.hash = hash_json(&JournalHashView::from(&record))?;
        self.records.push(record.clone());
        Ok(record)
    }

    pub fn records(&self) -> &[JournalRecord] {
        &self.records
    }

    pub fn snapshot(&self) -> JournalSnapshot {
        JournalSnapshot {
            records: self.records.clone(),
        }
    }

    pub fn root_hash(&self) -> HashValue {
        self.records
            .last()
            .map(|record| record.hash.clone())
            .unwrap_or_else(|| GENESIS_HASH.to_string())
    }

    pub fn verify_chain(&self) -> BeaterOsResult<JournalVerificationReport> {
        let mut prev_hash = GENESIS_HASH.to_string();
        let mut causality = CausalityState::default();
        for (idx, record) in self.records.iter().enumerate() {
            let expected_seq = idx as u64;
            if record.seq != expected_seq {
                return Err(BeaterOsError::JournalSeq {
                    expected: expected_seq,
                    found: record.seq,
                });
            }
            if record.prev_hash != prev_hash {
                return Err(BeaterOsError::JournalPrevHash {
                    seq: record.seq,
                    expected: prev_hash,
                    found: record.prev_hash.clone(),
                });
            }
            let expected_hash = hash_json(&JournalHashView::from(record))?;
            if record.hash != expected_hash {
                return Err(BeaterOsError::JournalHash {
                    seq: record.seq,
                    expected: expected_hash,
                    found: record.hash.clone(),
                });
            }
            verify_event_causality(record, &mut causality)?;
            prev_hash = record.hash.clone();
        }
        Ok(JournalVerificationReport {
            records: self.records.len(),
            root_hash: prev_hash,
        })
    }
}

#[derive(Default)]
struct CausalityState {
    session_statuses: BTreeMap<String, SessionStatus>,
    proposed_actions: BTreeMap<String, ActionManifest>,
    issued_grants: BTreeMap<String, CapabilityGrant>,
    issued_mandates: BTreeMap<String, PaymentMandate>,
    allowed_decisions: BTreeMap<String, HashValue>,
    allowed_decision_ids: BTreeMap<String, String>,
    latest_decision_by_action: BTreeMap<String, DecisionResult>,
    open_execution_leases: BTreeMap<String, ExecutionLease>,
    reconciled_execution_actions: BTreeMap<String, String>,
    receipted_actions: BTreeSet<String>,
    review_ids: BTreeMap<String, ()>,
    simulation_ids: BTreeMap<String, ()>,
    receipt_chain: Vec<CapabilityReceipt>,
    prior_event_ids: BTreeSet<String>,
    transition_ids: BTreeSet<String>,
    issued_revocation_handles: BTreeSet<String>,
    revoked_handles: BTreeSet<String>,
}

fn verify_event_causality(
    record: &JournalRecord,
    state: &mut CausalityState,
) -> BeaterOsResult<()> {
    match &record.event {
        JournalEvent::SessionCreated { session } => {
            if state
                .session_statuses
                .insert(session.session_id.clone(), session.status.clone())
                .is_some()
            {
                return causality_error(
                    record.seq,
                    format!("session {} was created more than once", session.session_id),
                );
            }
        }
        JournalEvent::SessionStatusChanged {
            transition_id,
            session_id,
            from,
            to,
        } => {
            if transition_id.trim().is_empty() {
                return causality_error(record.seq, "session transition id is empty".to_string());
            }
            if !state.transition_ids.insert(transition_id.clone()) {
                return causality_error(
                    record.seq,
                    format!("session transition {transition_id} was recorded more than once"),
                );
            }
            let Some(current) = state.session_statuses.get(session_id) else {
                return causality_error(
                    record.seq,
                    format!(
                        "session transition {transition_id} references unknown session {session_id}"
                    ),
                );
            };
            if current != from {
                return causality_error(
                    record.seq,
                    format!(
                        "session transition {transition_id} from {from:?} does not match current status {current:?}"
                    ),
                );
            }
            if !valid_session_transition(from, to) {
                return causality_error(
                    record.seq,
                    format!("illegal session transition {transition_id}: {from:?} -> {to:?}"),
                );
            }
            state
                .session_statuses
                .insert(session_id.clone(), to.clone());
        }
        JournalEvent::CapabilityGranted { grant } => {
            if grant.revocation_handle == grant.grant_id {
                return causality_error(
                    record.seq,
                    format!(
                        "grant {} revocation handle must not equal the grant event id",
                        grant.grant_id
                    ),
                );
            }
            if state.prior_event_ids.contains(&grant.revocation_handle) {
                return causality_error(
                    record.seq,
                    format!(
                        "grant {} revocation handle {} collides with a prior journal event id",
                        grant.grant_id, grant.revocation_handle
                    ),
                );
            }
            if !state
                .issued_revocation_handles
                .insert(grant.revocation_handle.clone())
            {
                return causality_error(
                    record.seq,
                    format!(
                        "grant {} revocation handle {} was already issued",
                        grant.grant_id, grant.revocation_handle
                    ),
                );
            }
            state
                .issued_grants
                .insert(grant.grant_id.clone(), grant.clone());
        }
        JournalEvent::CapabilityRevoked {
            grant_id,
            revocation_handle,
            revoked_by,
            reason,
        } => {
            if revoked_by.trim().is_empty() {
                return causality_error(record.seq, "revocation actor is empty".to_string());
            }
            if reason.trim().is_empty() {
                return causality_error(record.seq, "revocation reason is empty".to_string());
            }
            let Some(grant) = state.issued_grants.get(grant_id) else {
                return causality_error(
                    record.seq,
                    format!("revocation references grant {grant_id} before it was issued"),
                );
            };
            if grant.revocation_handle != *revocation_handle {
                return causality_error(
                    record.seq,
                    format!(
                        "revocation for grant {grant_id} uses handle {revocation_handle}, expected {}",
                        grant.revocation_handle
                    ),
                );
            }
            if !state.revoked_handles.insert(revocation_handle.clone()) {
                return causality_error(
                    record.seq,
                    format!("revocation handle {revocation_handle} was recorded more than once"),
                );
            }
        }
        JournalEvent::ActionProposed { manifest } => {
            if state
                .proposed_actions
                .insert(manifest.action_id.clone(), manifest.as_ref().clone())
                .is_some()
            {
                return causality_error(
                    record.seq,
                    format!("action {} was proposed more than once", manifest.action_id),
                );
            }
        }
        JournalEvent::PolicyDecided { decision } => {
            let Some(manifest) = state.proposed_actions.get(&decision.action_id) else {
                return causality_error(
                    record.seq,
                    format!(
                        "policy decision {} references action {} before it was proposed",
                        decision.decision_id, decision.action_id
                    ),
                );
            };
            let manifest_hash = manifest.digest()?;
            if decision.manifest_hash != manifest_hash {
                return causality_error(
                    record.seq,
                    format!(
                        "policy decision {} manifest hash does not match action {}",
                        decision.decision_id, decision.action_id
                    ),
                );
            }
            state
                .latest_decision_by_action
                .insert(decision.action_id.clone(), decision.result.clone());
            if decision.result == DecisionResult::Allowed {
                for required in &manifest.required_grants {
                    let Some(grant) = state.issued_grants.get(required) else {
                        continue;
                    };
                    if grant.revoked || state.revoked_handles.contains(&grant.revocation_handle) {
                        return causality_error(
                            record.seq,
                            format!(
                                "allowed decision {} references revoked grant {}",
                                decision.decision_id, required
                            ),
                        );
                    }
                    if grant.expires_at <= record.created_at {
                        return causality_error(
                            record.seq,
                            format!(
                                "allowed decision {} references expired grant {}",
                                decision.decision_id, required
                            ),
                        );
                    }
                }
                state
                    .allowed_decisions
                    .insert(decision.action_id.clone(), decision.manifest_hash.clone());
                state
                    .allowed_decision_ids
                    .insert(decision.action_id.clone(), decision.decision_id.clone());
            } else {
                state.allowed_decisions.remove(&decision.action_id);
                state.allowed_decision_ids.remove(&decision.action_id);
            }
        }
        JournalEvent::ExecutionLeaseIssued { lease } => {
            validate_execution_lease(record, lease, state)?;
            if state
                .open_execution_leases
                .insert(lease.action_id.clone(), lease.clone())
                .is_some()
            {
                return causality_error(
                    record.seq,
                    format!(
                        "action {} already has an open execution lease",
                        lease.action_id
                    ),
                );
            }
        }
        JournalEvent::ExecutionLeaseHeartbeated { heartbeat } => {
            validate_execution_lease_heartbeat(record, heartbeat, state)?;
            let Some(open_lease) = state.open_execution_leases.get_mut(&heartbeat.action_id) else {
                return causality_error(
                    record.seq,
                    format!(
                        "execution lease heartbeat {} references action {} without an open execution lease",
                        heartbeat.heartbeat_id, heartbeat.action_id
                    ),
                );
            };
            open_lease.expires_at = heartbeat.extended_expires_at;
        }
        JournalEvent::ExecutionLeaseReconciled { reconciliation } => {
            validate_execution_lease_reconciliation(record, reconciliation, state)?;
            state
                .open_execution_leases
                .remove(&reconciliation.action_id);
            state.reconciled_execution_actions.insert(
                reconciliation.action_id.clone(),
                reconciliation.reconciliation_id.clone(),
            );
        }
        JournalEvent::ApprovalRecorded { approval } => {
            if state
                .review_ids
                .insert(approval.review_id.clone(), ())
                .is_some()
            {
                return causality_error(
                    record.seq,
                    format!(
                        "approval {} was recorded more than once",
                        approval.review_id
                    ),
                );
            }
            let Some(manifest) = state.proposed_actions.get(&approval.action_id) else {
                return causality_error(
                    record.seq,
                    format!(
                        "approval {} references action {} before it was proposed",
                        approval.review_id, approval.action_id
                    ),
                );
            };
            if manifest.digest()? != approval.manifest_hash {
                return causality_error(
                    record.seq,
                    format!(
                        "approval {} manifest hash does not match action {}",
                        approval.review_id, approval.action_id
                    ),
                );
            }
            if !state.issued_grants.contains_key(&approval.grant_id) {
                return causality_error(
                    record.seq,
                    format!(
                        "approval {} references grant {} before it was issued",
                        approval.review_id, approval.grant_id
                    ),
                );
            }
            if state.latest_decision_by_action.get(&approval.action_id)
                != Some(&DecisionResult::NeedsApproval)
            {
                return causality_error(
                    record.seq,
                    format!(
                        "approval {} references action {} without a latest NeedsApproval decision",
                        approval.review_id, approval.action_id
                    ),
                );
            }
            if approval.approved_at > record.created_at {
                return causality_error(
                    record.seq,
                    format!("approval {} is future-dated", approval.review_id),
                );
            }
        }
        JournalEvent::SimulationRecorded { simulation } => {
            if state
                .simulation_ids
                .insert(simulation.simulation_id.clone(), ())
                .is_some()
            {
                return causality_error(
                    record.seq,
                    format!(
                        "simulation {} was recorded more than once",
                        simulation.simulation_id
                    ),
                );
            }
            let Some(manifest) = state.proposed_actions.get(&simulation.action_id) else {
                return causality_error(
                    record.seq,
                    format!(
                        "simulation {} references action {} before it was proposed",
                        simulation.simulation_id, simulation.action_id
                    ),
                );
            };
            if manifest.digest()? != simulation.manifest_hash {
                return causality_error(
                    record.seq,
                    format!(
                        "simulation {} manifest hash does not match action {}",
                        simulation.simulation_id, simulation.action_id
                    ),
                );
            }
            if state.latest_decision_by_action.get(&simulation.action_id)
                != Some(&DecisionResult::NeedsSimulation)
            {
                return causality_error(
                    record.seq,
                    format!(
                        "simulation {} references action {} without a latest NeedsSimulation decision",
                        simulation.simulation_id, simulation.action_id
                    ),
                );
            }
            if simulation.passed_at > record.created_at {
                return causality_error(
                    record.seq,
                    format!("simulation {} is future-dated", simulation.simulation_id),
                );
            }
        }
        JournalEvent::ReceiptAppended { receipt } => {
            let Some(manifest) = state.proposed_actions.get(&receipt.action_id) else {
                return causality_error(
                    record.seq,
                    format!(
                        "receipt {} references action {} before it was proposed",
                        receipt.receipt_id, receipt.action_id
                    ),
                );
            };
            if let Some(reconciliation_id) =
                state.reconciled_execution_actions.get(&receipt.action_id)
            {
                return causality_error(
                    record.seq,
                    format!(
                        "receipt {} references action {} after execution lease reconciliation {}",
                        receipt.receipt_id, receipt.action_id, reconciliation_id
                    ),
                );
            }
            let Some(allowed_manifest_hash) = state.allowed_decisions.get(&receipt.action_id)
            else {
                let latest = state
                    .latest_decision_by_action
                    .get(&receipt.action_id)
                    .map(|result| format!("{result:?}"))
                    .unwrap_or_else(|| "missing".to_string());
                return causality_error(
                    record.seq,
                    format!(
                        "receipt {} references action {} without a prior allowed decision (latest decision: {})",
                        receipt.receipt_id, receipt.action_id, latest
                    ),
                );
            };
            if &manifest.digest()? != allowed_manifest_hash {
                return causality_error(
                    record.seq,
                    format!(
                        "receipt {} references action {} whose allowed decision hash is stale",
                        receipt.receipt_id, manifest.action_id
                    ),
                );
            }
            if receipt.tool_id != manifest.tool_id {
                return causality_error(
                    record.seq,
                    format!(
                        "receipt {} tool {} does not match action {} tool {}",
                        receipt.receipt_id, receipt.tool_id, manifest.action_id, manifest.tool_id
                    ),
                );
            }
            if receipt.input_digest != manifest.inputs_digest {
                return causality_error(
                    record.seq,
                    format!(
                        "receipt {} input digest does not match action {} input digest",
                        receipt.receipt_id, manifest.action_id
                    ),
                );
            }
            let expected_target = manifest
                .resolved_target
                .as_ref()
                .unwrap_or(&manifest.target);
            if &receipt.target != expected_target {
                return causality_error(
                    record.seq,
                    format!(
                        "receipt {} target does not match action {} target",
                        receipt.receipt_id, manifest.action_id
                    ),
                );
            }
            if receipt
                .side_effects
                .iter()
                .any(|effect| !manifest.expected_side_effects.contains(effect))
            {
                return causality_error(
                    record.seq,
                    format!(
                        "receipt {} contains side effects not declared by action {}",
                        receipt.receipt_id, manifest.action_id
                    ),
                );
            }
            let consumed_execution_lease =
                if let Some(lease) = state.open_execution_leases.remove(&receipt.action_id) {
                    if lease.tool_id != receipt.tool_id {
                        return causality_error(
                            record.seq,
                            format!(
                                "receipt {} tool {} does not match execution lease {} tool {}",
                                receipt.receipt_id, receipt.tool_id, lease.lease_id, lease.tool_id
                            ),
                        );
                    }
                    if lease.target != receipt.target {
                        return causality_error(
                            record.seq,
                            format!(
                                "receipt {} target does not match execution lease {} target",
                                receipt.receipt_id, lease.lease_id
                            ),
                        );
                    }
                    if receipt.started_at < lease.leased_at {
                        return causality_error(
                            record.seq,
                            format!(
                                "receipt {} started before execution lease {} was issued",
                                receipt.receipt_id, lease.lease_id
                            ),
                        );
                    }
                    if receipt.finished_at > lease.expires_at {
                        return causality_error(
                            record.seq,
                            format!(
                                "receipt {} finished after execution lease {} expired",
                                receipt.receipt_id, lease.lease_id
                            ),
                        );
                    }
                    if record.created_at > lease.expires_at {
                        return causality_error(
                            record.seq,
                            format!(
                                "receipt {} was journaled after execution lease {} expired",
                                receipt.receipt_id, lease.lease_id
                            ),
                        );
                    }
                    if receipt.finished_at > record.created_at {
                        return causality_error(
                            record.seq,
                            format!(
                                "receipt {} finished after it was journaled",
                                receipt.receipt_id
                            ),
                        );
                    }
                    true
                } else {
                    false
                };
            if manifest.action_kind == ActionKind::Execute && !consumed_execution_lease {
                return causality_error(
                    record.seq,
                    format!(
                        "receipt {} references execute action {} without an open execution lease",
                        receipt.receipt_id, receipt.action_id
                    ),
                );
            }
            validate_payment_receipt(record.seq, record.created_at, manifest, receipt, state)?;
            state.receipted_actions.insert(receipt.action_id.clone());
            state.receipt_chain.push(receipt.clone());
            ReceiptLedger::from_receipts(state.receipt_chain.clone()).verify_chain()?;
        }
        JournalEvent::PaymentMandateIssued { mandate } => {
            if state.issued_mandates.contains_key(&mandate.mandate_id) {
                return causality_error(
                    record.seq,
                    format!(
                        "payment mandate {} was issued more than once",
                        mandate.mandate_id
                    ),
                );
            }
            validate_payment_mandate_event(record.seq, record.created_at, mandate, state)?;
            state
                .issued_mandates
                .insert(mandate.mandate_id.clone(), mandate.clone());
        }
        JournalEvent::MemoryWritten { memory } => {
            validate_memory_source(
                memory,
                record.seq,
                state.prior_event_ids.iter().map(String::as_str),
            )?;
        }
        JournalEvent::ScenarioEvaluated { .. } | JournalEvent::IncidentAnnotated { .. } => {}
    }
    if let Some(event_id) = primary_event_id(record)
        && !state.prior_event_ids.insert(event_id.to_string())
    {
        return causality_error(
            record.seq,
            format!("journal event id {event_id} was recorded more than once"),
        );
    }
    Ok(())
}

fn validate_execution_lease(
    record: &JournalRecord,
    lease: &ExecutionLease,
    state: &CausalityState,
) -> BeaterOsResult<()> {
    for (field, value) in [
        ("lease_id", lease.lease_id.as_str()),
        ("session_id", lease.session_id.as_str()),
        ("action_id", lease.action_id.as_str()),
        ("manifest_hash", lease.manifest_hash.as_str()),
        ("decision_id", lease.decision_id.as_str()),
        ("tool_id", lease.tool_id.as_str()),
        ("tool_ref", lease.tool_ref.as_str()),
    ] {
        if value.trim().is_empty() {
            return causality_error(
                record.seq,
                format!("execution lease {} has empty {field}", lease.lease_id),
            );
        }
    }
    match state.session_statuses.get(&lease.session_id) {
        Some(SessionStatus::Running) => {}
        Some(status) => {
            return causality_error(
                record.seq,
                format!(
                    "execution lease {} was issued while session {} was {status:?}",
                    lease.lease_id, lease.session_id
                ),
            );
        }
        None => {
            return causality_error(
                record.seq,
                format!(
                    "execution lease {} references missing session {}",
                    lease.lease_id, lease.session_id
                ),
            );
        }
    }
    let Some(manifest) = state.proposed_actions.get(&lease.action_id) else {
        return causality_error(
            record.seq,
            format!(
                "execution lease {} references action {} before it was proposed",
                lease.lease_id, lease.action_id
            ),
        );
    };
    if manifest.session_id != lease.session_id {
        return causality_error(
            record.seq,
            format!(
                "execution lease {} session {} does not match action {} session {}",
                lease.lease_id, lease.session_id, manifest.action_id, manifest.session_id
            ),
        );
    }
    if manifest.digest()? != lease.manifest_hash {
        return causality_error(
            record.seq,
            format!(
                "execution lease {} manifest hash does not match action {}",
                lease.lease_id, manifest.action_id
            ),
        );
    }
    if state.latest_decision_by_action.get(&lease.action_id) != Some(&DecisionResult::Allowed) {
        return causality_error(
            record.seq,
            format!(
                "execution lease {} references action {} without a latest Allowed decision",
                lease.lease_id, lease.action_id
            ),
        );
    }
    if state.receipted_actions.contains(&lease.action_id) {
        return causality_error(
            record.seq,
            format!(
                "execution lease {} references already-receipted action {}",
                lease.lease_id, lease.action_id
            ),
        );
    }
    if let Some(reconciliation_id) = state.reconciled_execution_actions.get(&lease.action_id) {
        return causality_error(
            record.seq,
            format!(
                "execution lease {} references already-reconciled action {} via {}",
                lease.lease_id, lease.action_id, reconciliation_id
            ),
        );
    }
    if state.allowed_decision_ids.get(&lease.action_id) != Some(&lease.decision_id) {
        return causality_error(
            record.seq,
            format!(
                "execution lease {} decision id is not the latest allowed decision for action {}",
                lease.lease_id, lease.action_id
            ),
        );
    }
    if lease.tool_id != manifest.tool_id {
        return causality_error(
            record.seq,
            format!(
                "execution lease {} tool {} does not match action {} tool {}",
                lease.lease_id, lease.tool_id, manifest.action_id, manifest.tool_id
            ),
        );
    }
    let expected_target = manifest
        .resolved_target
        .as_ref()
        .unwrap_or(&manifest.target);
    if &lease.target != expected_target {
        return causality_error(
            record.seq,
            format!(
                "execution lease {} target does not match action {} target",
                lease.lease_id, manifest.action_id
            ),
        );
    }
    if lease.required_grants != manifest.required_grants {
        return causality_error(
            record.seq,
            format!(
                "execution lease {} required grants do not match action {}",
                lease.lease_id, manifest.action_id
            ),
        );
    }
    if lease.requested_budget != manifest.requested_budget {
        return causality_error(
            record.seq,
            format!(
                "execution lease {} requested budget does not match action {}",
                lease.lease_id, manifest.action_id
            ),
        );
    }
    if lease.leased_at > record.created_at {
        return causality_error(
            record.seq,
            format!("execution lease {} is future-dated", lease.lease_id),
        );
    }
    if lease.expires_at <= record.created_at {
        return causality_error(
            record.seq,
            format!(
                "execution lease {} expires before it can execute",
                lease.lease_id
            ),
        );
    }
    for required in &manifest.required_grants {
        let Some(grant) = state.issued_grants.get(required) else {
            return causality_error(
                record.seq,
                format!(
                    "execution lease {} references missing grant {}",
                    lease.lease_id, required
                ),
            );
        };
        if grant.revoked || state.revoked_handles.contains(&grant.revocation_handle) {
            return causality_error(
                record.seq,
                format!(
                    "execution lease {} references revoked grant {}",
                    lease.lease_id, required
                ),
            );
        }
        if grant.expires_at <= record.created_at {
            return causality_error(
                record.seq,
                format!(
                    "execution lease {} references expired grant {}",
                    lease.lease_id, required
                ),
            );
        }
    }
    Ok(())
}

fn validate_execution_lease_reconciliation(
    record: &JournalRecord,
    reconciliation: &ExecutionLeaseReconciliation,
    state: &CausalityState,
) -> BeaterOsResult<()> {
    for (field, value) in [
        (
            "reconciliation_id",
            reconciliation.reconciliation_id.as_str(),
        ),
        ("lease_id", reconciliation.lease_id.as_str()),
        ("session_id", reconciliation.session_id.as_str()),
        ("action_id", reconciliation.action_id.as_str()),
        ("manifest_hash", reconciliation.manifest_hash.as_str()),
        ("decision_id", reconciliation.decision_id.as_str()),
        ("reconciled_by", reconciliation.reconciled_by.as_str()),
        ("reason", reconciliation.reason.as_str()),
    ] {
        if value.trim().is_empty() {
            return causality_error(
                record.seq,
                format!(
                    "execution lease reconciliation {} has empty {field}",
                    reconciliation.reconciliation_id
                ),
            );
        }
    }
    if reconciliation
        .evidence_refs
        .iter()
        .any(|evidence| evidence.trim().is_empty())
    {
        return causality_error(
            record.seq,
            format!(
                "execution lease reconciliation {} has empty evidence ref",
                reconciliation.reconciliation_id
            ),
        );
    }
    if reconciliation.reconciled_at > record.created_at {
        return causality_error(
            record.seq,
            format!(
                "execution lease reconciliation {} is future-dated",
                reconciliation.reconciliation_id
            ),
        );
    }
    if state.receipted_actions.contains(&reconciliation.action_id) {
        return causality_error(
            record.seq,
            format!(
                "execution lease reconciliation {} references already-receipted action {}",
                reconciliation.reconciliation_id, reconciliation.action_id
            ),
        );
    }
    if state
        .reconciled_execution_actions
        .contains_key(&reconciliation.action_id)
    {
        return causality_error(
            record.seq,
            format!(
                "execution lease reconciliation {} references already-reconciled action {}",
                reconciliation.reconciliation_id, reconciliation.action_id
            ),
        );
    }
    let Some(lease) = state.open_execution_leases.get(&reconciliation.action_id) else {
        return causality_error(
            record.seq,
            format!(
                "execution lease reconciliation {} references action {} without an open execution lease",
                reconciliation.reconciliation_id, reconciliation.action_id
            ),
        );
    };
    if reconciliation.lease_id != lease.lease_id {
        return causality_error(
            record.seq,
            format!(
                "execution lease reconciliation {} lease {} does not match open lease {}",
                reconciliation.reconciliation_id, reconciliation.lease_id, lease.lease_id
            ),
        );
    }
    if reconciliation.session_id != lease.session_id
        || reconciliation.action_id != lease.action_id
        || reconciliation.manifest_hash != lease.manifest_hash
        || reconciliation.decision_id != lease.decision_id
    {
        return causality_error(
            record.seq,
            format!(
                "execution lease reconciliation {} does not match open lease {} authority",
                reconciliation.reconciliation_id, lease.lease_id
            ),
        );
    }
    if reconciliation.reconciled_at < lease.expires_at {
        return causality_error(
            record.seq,
            format!(
                "execution lease reconciliation {} occurred before lease {} expired",
                reconciliation.reconciliation_id, lease.lease_id
            ),
        );
    }
    if record.created_at < lease.expires_at {
        return causality_error(
            record.seq,
            format!(
                "execution lease reconciliation {} was journaled before lease {} expired",
                reconciliation.reconciliation_id, lease.lease_id
            ),
        );
    }
    Ok(())
}

fn validate_payment_mandate_event(
    seq: u64,
    created_at: chrono::DateTime<Utc>,
    mandate: &PaymentMandate,
    state: &CausalityState,
) -> BeaterOsResult<()> {
    match state.session_statuses.get(&mandate.session_id) {
        Some(SessionStatus::Running) => {}
        Some(status) => {
            return causality_error(
                seq,
                format!(
                    "payment mandate {} was issued while session {} was {status:?}",
                    mandate.mandate_id, mandate.session_id
                ),
            );
        }
        None => {
            return causality_error(
                seq,
                format!(
                    "payment mandate {} references missing session {}",
                    mandate.mandate_id, mandate.session_id
                ),
            );
        }
    }
    for (field, value) in [
        ("mandate_id", mandate.mandate_id.as_str()),
        ("issuer", mandate.issuer.as_str()),
        ("holder", mandate.holder.as_str()),
        ("session_id", mandate.session_id.as_str()),
        ("rail", mandate.rail.as_str()),
        ("asset", mandate.asset.as_str()),
        ("counterparty_policy", mandate.counterparty_policy.as_str()),
        ("purpose", mandate.purpose.as_str()),
        ("idempotency_key", mandate.idempotency_key.as_str()),
        ("receipt_requirement", mandate.receipt_requirement.as_str()),
    ] {
        if value.trim().is_empty() {
            return causality_error(
                seq,
                format!("payment mandate {} has empty {field}", mandate.mandate_id),
            );
        }
    }
    if mandate.max_minor_units == 0 {
        return causality_error(
            seq,
            format!(
                "payment mandate {} max_minor_units must be positive",
                mandate.mandate_id
            ),
        );
    }
    if mandate.approval_threshold_minor_units > mandate.max_minor_units {
        return causality_error(
            seq,
            format!(
                "payment mandate {} approval threshold exceeds ceiling",
                mandate.mandate_id
            ),
        );
    }
    if mandate.expires_at <= created_at {
        return causality_error(
            seq,
            format!(
                "payment mandate {} expires_at must be after issuance",
                mandate.mandate_id
            ),
        );
    }
    if mandate.receipt_requirement != "required" {
        return causality_error(
            seq,
            format!(
                "payment mandate {} has unsupported receipt_requirement {:?}",
                mandate.mandate_id, mandate.receipt_requirement
            ),
        );
    }
    if mandate.allowed_adapter_ids.is_empty()
        || mandate
            .allowed_adapter_ids
            .iter()
            .any(|adapter| adapter.trim().is_empty())
    {
        return causality_error(
            seq,
            format!(
                "payment mandate {} requires explicit allowed_adapter_ids",
                mandate.mandate_id
            ),
        );
    }
    if mandate.allowed_envelope_formats.is_empty()
        || mandate
            .allowed_envelope_formats
            .iter()
            .any(|format| format.trim().is_empty())
    {
        return causality_error(
            seq,
            format!(
                "payment mandate {} requires explicit allowed_envelope_formats",
                mandate.mandate_id
            ),
        );
    }
    if !valid_counterparty_policy(&mandate.counterparty_policy) {
        return causality_error(
            seq,
            format!(
                "payment mandate {} has invalid counterparty_policy",
                mandate.mandate_id
            ),
        );
    }
    Ok(())
}

fn validate_payment_receipt(
    seq: u64,
    created_at: chrono::DateTime<Utc>,
    manifest: &ActionManifest,
    receipt: &CapabilityReceipt,
    state: &CausalityState,
) -> BeaterOsResult<()> {
    if !is_payment_manifest(manifest) {
        if receipt.payment_receipt.is_some() {
            return causality_error(
                seq,
                format!(
                    "receipt {} carries payment receipt evidence for non-payment action {}",
                    receipt.receipt_id, manifest.action_id
                ),
            );
        }
        return Ok(());
    }

    if !receipt.side_effects.contains(&SideEffectClass::Payment) {
        return causality_error(
            seq,
            format!(
                "payment receipt {} must report the payment side effect declared by action {}",
                receipt.receipt_id, manifest.action_id
            ),
        );
    }

    let Some(intent) = &manifest.payment_intent else {
        return causality_error(
            seq,
            format!(
                "payment receipt {} references action {} without a payment_intent",
                receipt.receipt_id, manifest.action_id
            ),
        );
    };
    let Some(mandate) = state.issued_mandates.get(&intent.mandate_id) else {
        return causality_error(
            seq,
            format!(
                "payment receipt {} references mandate {} before it was issued",
                receipt.receipt_id, intent.mandate_id
            ),
        );
    };
    match mandate.receipt_requirement.as_str() {
        "required" => {}
        other => {
            return causality_error(
                seq,
                format!(
                    "payment mandate {} has unsupported receipt_requirement {other:?}",
                    mandate.mandate_id
                ),
            );
        }
    }

    let Some(evidence) = &receipt.payment_receipt else {
        return causality_error(
            seq,
            format!(
                "payment receipt {} is missing required typed payment evidence",
                receipt.receipt_id
            ),
        );
    };
    validate_payment_receipt_evidence(seq, created_at, manifest, intent, mandate, evidence)
}

fn validate_payment_receipt_evidence(
    seq: u64,
    created_at: chrono::DateTime<Utc>,
    manifest: &ActionManifest,
    intent: &PaymentIntent,
    mandate: &PaymentMandate,
    evidence: &PaymentReceiptEvidence,
) -> BeaterOsResult<()> {
    let manifest_hash = manifest.digest()?;
    if evidence.manifest_hash != manifest_hash {
        return causality_error(
            seq,
            format!(
                "payment receipt evidence manifest_hash does not match action {}",
                manifest.action_id
            ),
        );
    }
    if evidence.mandate_id != intent.mandate_id
        || evidence.rail != intent.rail
        || evidence.adapter_id != intent.adapter_id
        || evidence.adapter_version != intent.adapter_version
        || evidence.asset != intent.asset
        || evidence.amount_minor_units != intent.amount_minor_units
        || evidence.counterparty_ref != intent.counterparty_ref
        || evidence.counterparty_binding_hash != intent.counterparty_binding_hash
        || evidence.purpose != intent.purpose
        || evidence.payment_idempotency_key != intent.payment_idempotency_key
        || evidence.envelope_format != intent.envelope_format
        || evidence.envelope_hash != intent.envelope_hash
    {
        return causality_error(
            seq,
            format!(
                "payment receipt evidence does not match payment_intent for action {}",
                manifest.action_id
            ),
        );
    }
    if evidence.rail != mandate.rail
        || evidence.asset != mandate.asset
        || evidence.purpose != mandate.purpose
        || evidence.payment_idempotency_key != mandate.idempotency_key
    {
        return causality_error(
            seq,
            format!(
                "payment receipt evidence does not match mandate {}",
                mandate.mandate_id
            ),
        );
    }
    if !is_hex_64(&evidence.rail_receipt_hash) {
        return causality_error(
            seq,
            "payment receipt evidence rail_receipt_hash must be lowercase 32-byte hex".to_string(),
        );
    }
    match evidence.settlement_status {
        PaymentSettlementStatus::Settled if evidence.settled_at.is_none() => {
            return causality_error(
                seq,
                "payment receipt evidence settled status requires settled_at".to_string(),
            );
        }
        PaymentSettlementStatus::Submitted
        | PaymentSettlementStatus::Failed
        | PaymentSettlementStatus::Canceled
            if evidence.settled_at.is_some() =>
        {
            return causality_error(
                seq,
                "payment receipt evidence settled_at is only valid for settled status".to_string(),
            );
        }
        _ => {}
    }
    if let Some(settled_at) = evidence.settled_at
        && settled_at > created_at
    {
        return causality_error(
            seq,
            "payment receipt evidence settled_at is future-dated".to_string(),
        );
    }
    Ok(())
}

fn is_payment_manifest(manifest: &ActionManifest) -> bool {
    manifest.action_kind == crate::contracts::ActionKind::Spend
        || manifest
            .expected_side_effects
            .contains(&SideEffectClass::Payment)
}

fn is_hex_64(value: &str) -> bool {
    value.len() == 64
        && value
            .as_bytes()
            .iter()
            .all(|byte| byte.is_ascii_hexdigit() && !byte.is_ascii_uppercase())
}

fn valid_counterparty_policy(policy: &str) -> bool {
    if policy == "any" {
        return true;
    }
    if let Some(value) = policy.strip_prefix("exact:") {
        return !value.is_empty();
    }
    if let Some(value) = policy.strip_prefix("prefix:") {
        return !value.is_empty();
    }
    if let Some(value) = policy.strip_prefix("hash:") {
        return is_hex_64(value);
    }
    if let Some(value) = policy.strip_prefix("allowlist:") {
        let mut entries = value.split(',').map(str::trim).peekable();
        return entries.peek().is_some() && entries.all(|entry| !entry.is_empty());
    }
    false
}

fn validate_execution_lease_heartbeat(
    record: &JournalRecord,
    heartbeat: &ExecutionLeaseHeartbeat,
    state: &CausalityState,
) -> BeaterOsResult<()> {
    if heartbeat.heartbeat_id.trim().is_empty()
        || heartbeat.lease_id.trim().is_empty()
        || heartbeat.session_id.trim().is_empty()
        || heartbeat.action_id.trim().is_empty()
        || heartbeat.observed_by.trim().is_empty()
    {
        return causality_error(
            record.seq,
            "execution lease heartbeat has empty identity field".to_string(),
        );
    }
    if heartbeat
        .evidence_refs
        .iter()
        .any(|reference| reference.trim().is_empty())
    {
        return causality_error(
            record.seq,
            format!(
                "execution lease heartbeat {} contains an empty evidence reference",
                heartbeat.heartbeat_id
            ),
        );
    }
    if state.receipted_actions.contains(&heartbeat.action_id) {
        return causality_error(
            record.seq,
            format!(
                "execution lease heartbeat {} references already-receipted action {}",
                heartbeat.heartbeat_id, heartbeat.action_id
            ),
        );
    }
    if state
        .reconciled_execution_actions
        .contains_key(&heartbeat.action_id)
    {
        return causality_error(
            record.seq,
            format!(
                "execution lease heartbeat {} references already-reconciled action {}",
                heartbeat.heartbeat_id, heartbeat.action_id
            ),
        );
    }
    let Some(lease) = state.open_execution_leases.get(&heartbeat.action_id) else {
        return causality_error(
            record.seq,
            format!(
                "execution lease heartbeat {} references action {} without an open execution lease",
                heartbeat.heartbeat_id, heartbeat.action_id
            ),
        );
    };
    if heartbeat.lease_id != lease.lease_id {
        return causality_error(
            record.seq,
            format!(
                "execution lease heartbeat {} lease {} does not match open lease {}",
                heartbeat.heartbeat_id, heartbeat.lease_id, lease.lease_id
            ),
        );
    }
    if heartbeat.session_id != lease.session_id
        || heartbeat.action_id != lease.action_id
        || heartbeat.manifest_hash != lease.manifest_hash
        || heartbeat.decision_id != lease.decision_id
    {
        return causality_error(
            record.seq,
            format!(
                "execution lease heartbeat {} does not match open lease {} authority",
                heartbeat.heartbeat_id, lease.lease_id
            ),
        );
    }
    if heartbeat.previous_expires_at != lease.expires_at {
        return causality_error(
            record.seq,
            format!(
                "execution lease heartbeat {} expected previous expiry {}, found {}",
                heartbeat.heartbeat_id, heartbeat.previous_expires_at, lease.expires_at
            ),
        );
    }
    if heartbeat.heartbeat_at >= lease.expires_at || record.created_at >= lease.expires_at {
        return causality_error(
            record.seq,
            format!(
                "execution lease heartbeat {} occurred after lease {} expired",
                heartbeat.heartbeat_id, lease.lease_id
            ),
        );
    }
    if heartbeat.extended_expires_at <= lease.expires_at {
        return causality_error(
            record.seq,
            format!(
                "execution lease heartbeat {} did not extend lease {}",
                heartbeat.heartbeat_id, lease.lease_id
            ),
        );
    }
    let Some(requested_wall_ms) = lease.requested_budget.max_wall_ms else {
        return causality_error(
            record.seq,
            format!(
                "record {}: execution lease heartbeat {} cannot extend lease {} without finite wall budget",
                record.seq, heartbeat.heartbeat_id, lease.lease_id
            ),
        );
    };
    let Some(max_wall_ms) = requested_wall_ms.checked_add(EXECUTION_LEASE_OVERHEAD_GRACE_MS) else {
        return causality_error(
            record.seq,
            format!(
                "record {}: execution lease heartbeat {} wall budget overflowed",
                record.seq, heartbeat.heartbeat_id
            ),
        );
    };
    let Ok(max_wall_delta_ms) = i64::try_from(max_wall_ms) else {
        return causality_error(
            record.seq,
            format!(
                "record {}: execution lease heartbeat {} wall budget cannot fit signed milliseconds",
                record.seq, heartbeat.heartbeat_id
            ),
        );
    };
    let Some(max_expires_at) = lease
        .leased_at
        .checked_add_signed(TimeDelta::milliseconds(max_wall_delta_ms))
    else {
        return causality_error(
            record.seq,
            format!(
                "record {}: execution lease heartbeat {} maximum expiration overflowed",
                record.seq, heartbeat.heartbeat_id
            ),
        );
    };
    if heartbeat.extended_expires_at > max_expires_at {
        return causality_error(
            record.seq,
            format!(
                "execution lease heartbeat {} extends lease {} beyond action wall budget",
                heartbeat.heartbeat_id, lease.lease_id
            ),
        );
    }
    Ok(())
}

fn primary_event_id(record: &JournalRecord) -> Option<&str> {
    match &record.event {
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
        // Memory ids are mutable projection keys: `beater-os-memory` explicitly
        // supports last-writer-wins rewrites for the same memory_id. They are
        // therefore not unambiguous journal event ids for provenance.
        JournalEvent::MemoryWritten { .. } => None,
        JournalEvent::ScenarioEvaluated { scenario, .. } => Some(scenario.scenario_id.as_str()),
        JournalEvent::IncidentAnnotated { incident_id, .. } => Some(incident_id.as_str()),
    }
}

fn valid_session_transition(from: &SessionStatus, to: &SessionStatus) -> bool {
    matches!(
        (from, to),
        (SessionStatus::Running, SessionStatus::Paused)
            | (SessionStatus::Paused, SessionStatus::Running)
            | (SessionStatus::Running, SessionStatus::Canceled)
            | (SessionStatus::Paused, SessionStatus::Canceled)
    )
}

fn validate_memory_source<'a>(
    memory: &MemoryRecord,
    seq: u64,
    known_event_ids: impl Iterator<Item = &'a str>,
) -> BeaterOsResult<()> {
    if memory.source_event_id.trim().is_empty() {
        return causality_error(
            seq,
            format!("memory {} has an empty source_event_id", memory.memory_id),
        );
    }
    if !known_event_ids
        .into_iter()
        .any(|id| id == memory.source_event_id)
    {
        return causality_error(
            seq,
            format!(
                "memory {} references unknown source event {}",
                memory.memory_id, memory.source_event_id
            ),
        );
    }
    Ok(())
}

fn causality_error<T>(seq: u64, reason: String) -> BeaterOsResult<T> {
    Err(BeaterOsError::JournalCausality { seq, reason })
}
