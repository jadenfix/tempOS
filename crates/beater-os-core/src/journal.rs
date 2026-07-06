use std::collections::BTreeMap;

use chrono::{DateTime, Utc};
use serde::{Deserialize, Serialize};

use crate::contracts::{
    ActionManifest, AgentSession, CapabilityGrant, DecisionResult, MemoryRecord, PolicyDecision,
    ScenarioManifest,
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
    CapabilityGranted {
        grant: CapabilityGrant,
    },
    ActionProposed {
        manifest: Box<ActionManifest>,
    },
    PolicyDecided {
        decision: PolicyDecision,
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
        let mut proposed_actions: BTreeMap<String, ActionManifest> = BTreeMap::new();
        let mut allowed_decisions: BTreeMap<String, HashValue> = BTreeMap::new();
        let mut latest_decision_by_action: BTreeMap<String, DecisionResult> = BTreeMap::new();
        let mut receipt_chain = Vec::new();
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
            verify_event_causality(
                record,
                &mut proposed_actions,
                &mut allowed_decisions,
                &mut latest_decision_by_action,
                &mut receipt_chain,
            )?;
            prev_hash = record.hash.clone();
        }
        Ok(JournalVerificationReport {
            records: self.records.len(),
            root_hash: prev_hash,
        })
    }
}

fn verify_event_causality(
    record: &JournalRecord,
    proposed_actions: &mut BTreeMap<String, ActionManifest>,
    allowed_decisions: &mut BTreeMap<String, HashValue>,
    latest_decision_by_action: &mut BTreeMap<String, DecisionResult>,
    receipt_chain: &mut Vec<CapabilityReceipt>,
) -> BeaterOsResult<()> {
    match &record.event {
        JournalEvent::ActionProposed { manifest } => {
            if proposed_actions
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
            let Some(manifest) = proposed_actions.get(&decision.action_id) else {
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
            latest_decision_by_action.insert(decision.action_id.clone(), decision.result.clone());
            if decision.result == DecisionResult::Allowed {
                allowed_decisions
                    .insert(decision.action_id.clone(), decision.manifest_hash.clone());
            } else {
                allowed_decisions.remove(&decision.action_id);
            }
        }
        JournalEvent::ReceiptAppended { receipt } => {
            let Some(manifest) = proposed_actions.get(&receipt.action_id) else {
                return causality_error(
                    record.seq,
                    format!(
                        "receipt {} references action {} before it was proposed",
                        receipt.receipt_id, receipt.action_id
                    ),
                );
            };
            let Some(allowed_manifest_hash) = allowed_decisions.get(&receipt.action_id) else {
                let latest = latest_decision_by_action
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
            receipt_chain.push(receipt.clone());
            ReceiptLedger::from_receipts(receipt_chain.clone()).verify_chain()?;
        }
        JournalEvent::SessionCreated { .. }
        | JournalEvent::CapabilityGranted { .. }
        | JournalEvent::MemoryWritten { .. }
        | JournalEvent::ScenarioEvaluated { .. }
        | JournalEvent::IncidentAnnotated { .. } => {}
    }
    Ok(())
}

fn causality_error<T>(seq: u64, reason: String) -> BeaterOsResult<T> {
    Err(BeaterOsError::JournalCausality { seq, reason })
}
