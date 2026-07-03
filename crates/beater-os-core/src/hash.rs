use serde::Serialize;
use sha2::{Digest, Sha256};

use crate::error::BeaterOsResult;

pub type HashValue = String;

pub const GENESIS_HASH: &str = "0000000000000000000000000000000000000000000000000000000000000000";

pub fn hash_json<T: Serialize>(value: &T) -> BeaterOsResult<HashValue> {
    let bytes = serde_json::to_vec(value)?;
    let mut hasher = Sha256::new();
    hasher.update(bytes);
    Ok(hex::encode(hasher.finalize()))
}
