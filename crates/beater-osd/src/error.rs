use thiserror::Error;

/// Result alias for daemon runtime operations.
pub type DaemonResult<T> = Result<T, DaemonError>;

/// Fail-closed errors surfaced by the local daemon runtime.
#[derive(Debug, Error)]
pub enum DaemonError {
    #[error("io error: {0}")]
    Io(#[from] std::io::Error),
    #[error("json error: {0}")]
    Json(#[from] serde_json::Error),
    #[error("core error: {0}")]
    Core(#[from] beater_os_core::BeaterOsError),
    #[error("registry error: {0}")]
    Registry(#[from] beater_os_tool_registry::RegistryError),
    #[error("invalid value for {field}: {value}")]
    Invalid { field: String, value: String },
    #[error("session not found: {0}")]
    SessionNotFound(String),
    #[error("session already exists: {0}")]
    SessionExists(String),
    #[error("timed out acquiring single-writer lock for session {0}")]
    LockTimeout(String),
    #[error("refused: {0}")]
    Refused(String),
}

impl DaemonError {
    pub fn invalid(field: impl Into<String>, value: impl Into<String>) -> Self {
        Self::Invalid {
            field: field.into(),
            value: value.into(),
        }
    }
}
