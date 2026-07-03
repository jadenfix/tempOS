use thiserror::Error;

pub type BeaterOsResult<T> = Result<T, BeaterOsError>;

#[derive(Debug, Error)]
pub enum BeaterOsError {
    #[error("json serialization failed: {0}")]
    Json(#[from] serde_json::Error),
    #[error("journal chain violation at seq {seq}: expected prev_hash {expected}, found {found}")]
    JournalPrevHash {
        seq: u64,
        expected: String,
        found: String,
    },
    #[error("journal hash violation at seq {seq}: expected hash {expected}, found {found}")]
    JournalHash {
        seq: u64,
        expected: String,
        found: String,
    },
    #[error("journal seq violation: expected seq {expected}, found {found}")]
    JournalSeq { expected: u64, found: u64 },
    #[error("receipt chain violation at seq {seq}: expected prev_hash {expected}, found {found}")]
    ReceiptPrevHash {
        seq: u64,
        expected: String,
        found: String,
    },
    #[error("receipt hash violation at seq {seq}: expected hash {expected}, found {found}")]
    ReceiptHash {
        seq: u64,
        expected: String,
        found: String,
    },
    #[error("receipt seq violation: expected seq {expected}, found {found}")]
    ReceiptSeq { expected: u64, found: u64 },
}
