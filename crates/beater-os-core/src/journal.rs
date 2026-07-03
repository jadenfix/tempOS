use chrono::{DateTime, Utc};
use serde::{Deserialize, Serialize};

use crate::contracts::{
    ActionManifest, AgentSession, CapabilityGrant, MemoryRecord, PolicyDecision, ScenarioManifest,
};
use crate::error::{BeaterOsError, BeaterOsResult};
use crate::hash::{GENESIS_HASH, HashValue, hash_json};
use crate::receipt::CapabilityReceipt;

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
        manifest: ActionManifest,
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
            prev_hash = record.hash.clone();
        }
        Ok(JournalVerificationReport {
            records: self.records.len(),
            root_hash: prev_hash,
        })
    }
}
