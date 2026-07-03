use chrono::{DateTime, Utc};
use serde::{Deserialize, Serialize};
use uuid::Uuid;

use crate::contracts::SideEffectClass;
use crate::error::{BeaterOsError, BeaterOsResult};
use crate::hash::{GENESIS_HASH, HashValue, hash_json};

#[derive(Clone, Debug, PartialEq, Eq, Serialize, Deserialize)]
pub struct CapabilityReceiptInput {
    pub receipt_id: Option<String>,
    pub action_id: String,
    pub tool_id: String,
    pub started_at: DateTime<Utc>,
    pub finished_at: DateTime<Utc>,
    pub status: String,
    pub input_digest: String,
    pub output_digest: String,
    pub side_effect_summary: String,
    #[serde(default)]
    pub side_effects: Vec<SideEffectClass>,
    #[serde(default)]
    pub external_ids: Vec<String>,
    #[serde(default)]
    pub artifact_refs: Vec<String>,
}

#[derive(Clone, Debug, PartialEq, Eq, Serialize, Deserialize)]
pub struct CapabilityReceipt {
    pub receipt_id: String,
    pub seq: u64,
    pub action_id: String,
    pub tool_id: String,
    pub started_at: DateTime<Utc>,
    pub finished_at: DateTime<Utc>,
    pub status: String,
    pub input_digest: String,
    pub output_digest: String,
    pub side_effect_summary: String,
    #[serde(default)]
    pub side_effects: Vec<SideEffectClass>,
    #[serde(default)]
    pub external_ids: Vec<String>,
    #[serde(default)]
    pub artifact_refs: Vec<String>,
    pub prev_receipt_hash: HashValue,
    pub receipt_hash: HashValue,
}

#[derive(Serialize)]
struct ReceiptHashView<'a> {
    receipt_id: &'a str,
    seq: u64,
    action_id: &'a str,
    tool_id: &'a str,
    started_at: &'a DateTime<Utc>,
    finished_at: &'a DateTime<Utc>,
    status: &'a str,
    input_digest: &'a str,
    output_digest: &'a str,
    side_effect_summary: &'a str,
    side_effects: &'a [SideEffectClass],
    external_ids: &'a [String],
    artifact_refs: &'a [String],
    prev_receipt_hash: &'a HashValue,
}

impl<'a> From<&'a CapabilityReceipt> for ReceiptHashView<'a> {
    fn from(receipt: &'a CapabilityReceipt) -> Self {
        Self {
            receipt_id: &receipt.receipt_id,
            seq: receipt.seq,
            action_id: &receipt.action_id,
            tool_id: &receipt.tool_id,
            started_at: &receipt.started_at,
            finished_at: &receipt.finished_at,
            status: &receipt.status,
            input_digest: &receipt.input_digest,
            output_digest: &receipt.output_digest,
            side_effect_summary: &receipt.side_effect_summary,
            side_effects: &receipt.side_effects,
            external_ids: &receipt.external_ids,
            artifact_refs: &receipt.artifact_refs,
            prev_receipt_hash: &receipt.prev_receipt_hash,
        }
    }
}

#[derive(Clone, Debug, Default)]
pub struct ReceiptLedger {
    receipts: Vec<CapabilityReceipt>,
}

impl ReceiptLedger {
    pub fn new() -> Self {
        Self {
            receipts: Vec::new(),
        }
    }

    pub fn from_receipts(receipts: Vec<CapabilityReceipt>) -> Self {
        Self { receipts }
    }

    pub fn append(&mut self, input: CapabilityReceiptInput) -> BeaterOsResult<CapabilityReceipt> {
        let seq = self.receipts.len() as u64;
        let prev_receipt_hash = self
            .receipts
            .last()
            .map(|receipt| receipt.receipt_hash.clone())
            .unwrap_or_else(|| GENESIS_HASH.to_string());
        let mut receipt = CapabilityReceipt {
            receipt_id: input
                .receipt_id
                .unwrap_or_else(|| Uuid::new_v4().to_string()),
            seq,
            action_id: input.action_id,
            tool_id: input.tool_id,
            started_at: input.started_at,
            finished_at: input.finished_at,
            status: input.status,
            input_digest: input.input_digest,
            output_digest: input.output_digest,
            side_effect_summary: input.side_effect_summary,
            side_effects: input.side_effects,
            external_ids: input.external_ids,
            artifact_refs: input.artifact_refs,
            prev_receipt_hash,
            receipt_hash: String::new(),
        };
        receipt.receipt_hash = hash_json(&ReceiptHashView::from(&receipt))?;
        self.receipts.push(receipt.clone());
        Ok(receipt)
    }

    pub fn receipts(&self) -> &[CapabilityReceipt] {
        &self.receipts
    }

    pub fn root_hash(&self) -> HashValue {
        self.receipts
            .last()
            .map(|receipt| receipt.receipt_hash.clone())
            .unwrap_or_else(|| GENESIS_HASH.to_string())
    }

    pub fn verify_chain(&self) -> BeaterOsResult<()> {
        let mut prev_hash = GENESIS_HASH.to_string();
        for (idx, receipt) in self.receipts.iter().enumerate() {
            let expected_seq = idx as u64;
            if receipt.seq != expected_seq {
                return Err(BeaterOsError::ReceiptSeq {
                    expected: expected_seq,
                    found: receipt.seq,
                });
            }
            if receipt.prev_receipt_hash != prev_hash {
                return Err(BeaterOsError::ReceiptPrevHash {
                    seq: receipt.seq,
                    expected: prev_hash,
                    found: receipt.prev_receipt_hash.clone(),
                });
            }
            let expected_hash = hash_json(&ReceiptHashView::from(receipt))?;
            if receipt.receipt_hash != expected_hash {
                return Err(BeaterOsError::ReceiptHash {
                    seq: receipt.seq,
                    expected: expected_hash,
                    found: receipt.receipt_hash.clone(),
                });
            }
            prev_hash = receipt.receipt_hash.clone();
        }
        Ok(())
    }
}
