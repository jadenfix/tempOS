use beater_os_core::{HashValue, hash_json};
use chrono::{DateTime, Utc};
use serde::{Deserialize, Serialize};

use crate::claim::{ClaimStatus, SliceClaim};
use crate::error::{CoordinationError, CoordinationResult};
use crate::merge::MergeGateDecision;
use crate::principal::AgentPrincipal;
use crate::review::ReviewAttestation;

/// Genesis hash for the coordination chain (matches `beater-os-core`).
pub const GENESIS_HASH: &str = "0000000000000000000000000000000000000000000000000000000000000000";

/// An append-only event describing one step of multi-agent coordination.
///
/// The coordination ledger is the "communication loop between agents" made
/// durable and tamper-evident (`final.md` 13.11): who claimed what, who
/// reviewed what, and how each merge gate resolved.
#[derive(Clone, Debug, PartialEq, Eq, Serialize, Deserialize)]
#[serde(tag = "kind", rename_all = "snake_case")]
pub enum CoordinationEvent {
    PrincipalRegistered {
        principal: AgentPrincipal,
    },
    SliceClaimed {
        claim: SliceClaim,
    },
    ClaimStatusChanged {
        slice_id: String,
        from: ClaimStatus,
        to: ClaimStatus,
    },
    ClaimReleased {
        slice_id: String,
        reason: String,
    },
    ReviewSubmitted {
        review: ReviewAttestation,
    },
    MergeGateEvaluated {
        decision: MergeGateDecision,
    },
    SliceMerged {
        slice_id: String,
        merger_id: String,
        decision_id: String,
        commit_sha: String,
    },
    ConflictDetected {
        slice_id: String,
        other_slice_id: String,
        detail: String,
    },
}

/// One hash-linked record in the coordination ledger.
#[derive(Clone, Debug, PartialEq, Eq, Serialize, Deserialize)]
pub struct LedgerRecord {
    pub seq: u64,
    pub created_at: DateTime<Utc>,
    pub event: CoordinationEvent,
    pub prev_hash: HashValue,
    pub hash: HashValue,
}

/// Serializable view used to compute a record's hash (excludes `hash` itself).
#[derive(Serialize)]
struct LedgerHashView<'a> {
    seq: u64,
    created_at: &'a DateTime<Utc>,
    event: &'a CoordinationEvent,
    prev_hash: &'a HashValue,
}

impl<'a> From<&'a LedgerRecord> for LedgerHashView<'a> {
    fn from(record: &'a LedgerRecord) -> Self {
        Self {
            seq: record.seq,
            created_at: &record.created_at,
            event: &record.event,
            prev_hash: &record.prev_hash,
        }
    }
}

/// An append-only, hash-chained ledger of coordination events.
#[derive(Clone, Debug, Default, PartialEq, Eq, Serialize, Deserialize)]
pub struct CoordinationLedger {
    records: Vec<LedgerRecord>,
}

impl CoordinationLedger {
    pub fn new() -> Self {
        Self {
            records: Vec::new(),
        }
    }

    /// Append an event, linking it to the prior record's hash.
    pub fn append(
        &mut self,
        event: CoordinationEvent,
        created_at: DateTime<Utc>,
    ) -> CoordinationResult<LedgerRecord> {
        let seq = self.records.len() as u64;
        let prev_hash = self
            .records
            .last()
            .map(|record| record.hash.clone())
            .unwrap_or_else(|| GENESIS_HASH.to_string());
        let mut record = LedgerRecord {
            seq,
            created_at,
            event,
            prev_hash,
            hash: String::new(),
        };
        record.hash = hash_json(&LedgerHashView::from(&record))?;
        self.records.push(record.clone());
        Ok(record)
    }

    pub fn records(&self) -> &[LedgerRecord] {
        &self.records
    }

    /// Current tip hash, or the genesis hash when empty.
    pub fn root_hash(&self) -> HashValue {
        self.records
            .last()
            .map(|record| record.hash.clone())
            .unwrap_or_else(|| GENESIS_HASH.to_string())
    }

    /// Verify sequence continuity and the hash chain end to end.
    pub fn verify_chain(&self) -> CoordinationResult<()> {
        let mut prev_hash = GENESIS_HASH.to_string();
        for (idx, record) in self.records.iter().enumerate() {
            let expected_seq = idx as u64;
            if record.seq != expected_seq {
                return Err(CoordinationError::LedgerChain {
                    seq: record.seq,
                    reason: format!("expected seq {expected_seq}, found {}", record.seq),
                });
            }
            if record.prev_hash != prev_hash {
                return Err(CoordinationError::LedgerChain {
                    seq: record.seq,
                    reason: format!("expected prev_hash {prev_hash}, found {}", record.prev_hash),
                });
            }
            let expected_hash = hash_json(&LedgerHashView::from(record))?;
            if record.hash != expected_hash {
                return Err(CoordinationError::LedgerChain {
                    seq: record.seq,
                    reason: format!("expected hash {expected_hash}, found {}", record.hash),
                });
            }
            prev_hash = record.hash.clone();
        }
        Ok(())
    }
}
