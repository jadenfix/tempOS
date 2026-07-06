use std::collections::{BTreeMap, BTreeSet};

use chrono::{DateTime, Utc};
use serde::{Deserialize, Serialize};

use crate::contracts::{
    ActionManifest, AgentSession, ApprovalEvidence, CapabilityGrant, DecisionResult, MemoryRecord,
    PaymentMandate, PolicyDecision, ScenarioManifest, SessionStatus, SimulationEvidence,
};
use crate::error::{BeaterOsError, BeaterOsResult};
use crate::hash::{GENESIS_HASH, HashValue, hash_json};
use crate::receipt::{CapabilityReceipt, ReceiptLedger};

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
    PaymentMandateIssued {
        mandate: PaymentMandate,
    },
    ActionProposed {
        manifest: Box<ActionManifest>,
    },
    PolicyDecided {
        decision: PolicyDecision,
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
    allowed_decisions: BTreeMap<String, HashValue>,
    latest_decision_by_action: BTreeMap<String, DecisionResult>,
    review_ids: BTreeMap<String, ()>,
    simulation_ids: BTreeMap<String, ()>,
    receipt_chain: Vec<CapabilityReceipt>,
    prior_event_ids: BTreeSet<String>,
    transition_ids: BTreeSet<String>,
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
            state
                .issued_grants
                .insert(grant.grant_id.clone(), grant.clone());
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
                state
                    .allowed_decisions
                    .insert(decision.action_id.clone(), decision.manifest_hash.clone());
            } else {
                state.allowed_decisions.remove(&decision.action_id);
            }
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
            state.receipt_chain.push(receipt.clone());
            ReceiptLedger::from_receipts(state.receipt_chain.clone()).verify_chain()?;
        }
        JournalEvent::MemoryWritten { memory } => {
            validate_memory_source(
                memory,
                record.seq,
                state.prior_event_ids.iter().map(String::as_str),
            )?;
        }
        JournalEvent::PaymentMandateIssued { .. }
        | JournalEvent::ScenarioEvaluated { .. }
        | JournalEvent::IncidentAnnotated { .. } => {}
    }
    if let Some(event_id) = primary_event_id(record) {
        if !state.prior_event_ids.insert(event_id.to_string()) {
            return causality_error(
                record.seq,
                format!("journal event id {event_id} was recorded more than once"),
            );
        }
    }
    Ok(())
}

fn primary_event_id(record: &JournalRecord) -> Option<&str> {
    match &record.event {
        JournalEvent::SessionCreated { session } => Some(session.session_id.as_str()),
        JournalEvent::SessionStatusChanged { transition_id, .. } => Some(transition_id.as_str()),
        JournalEvent::CapabilityGranted { grant } => Some(grant.grant_id.as_str()),
        JournalEvent::PaymentMandateIssued { mandate } => Some(mandate.mandate_id.as_str()),
        JournalEvent::ActionProposed { manifest } => Some(manifest.action_id.as_str()),
        JournalEvent::PolicyDecided { decision } => Some(decision.decision_id.as_str()),
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
