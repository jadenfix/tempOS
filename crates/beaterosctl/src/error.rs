use thiserror::Error;

/// Result alias for CLI operations.
pub type CliResult<T> = Result<T, CliError>;

/// Errors surfaced by `beaterosctl`.
///
/// Every failure fails closed: the CLI never invents authority or silently
/// falls back to a permissive default when input is missing or invalid.
#[derive(Debug, Error)]
pub enum CliError {
    #[error("io error: {0}")]
    Io(#[from] std::io::Error),
    #[error("json error: {0}")]
    Json(#[from] serde_json::Error),
    #[error("core error: {0}")]
    Core(#[from] beater_os_core::BeaterOsError),
    /// The hosted runtime store refused or failed the operation.
    #[error("runtime error: {0}")]
    Runtime(#[from] beater_osd::DaemonError),
    /// The sandbox execution lane refused or failed (fail closed).
    #[error("sandbox error: {0}")]
    Sandbox(#[from] beater_os_sandbox::SandboxError),
    /// The tool gateway refused or failed (fail closed).
    #[error("gateway error: {0}")]
    Gateway(#[from] beater_os_tool_gateway::GatewayError),
    /// The trustworthy tool registry refused or failed (fail closed).
    #[error("registry error: {0}")]
    Registry(#[from] beater_os_tool_registry::RegistryError),
    /// The command line could not be understood.
    #[error("usage: {0}")]
    Usage(String),
    /// A required flag was not provided.
    #[error("missing required flag --{0}")]
    MissingFlag(String),
    /// A flag value could not be parsed into the expected type.
    #[error("invalid value for --{field}: {value}")]
    Invalid { field: String, value: String },
    /// A referenced session does not exist in the store.
    #[error("session not found: {0}")]
    SessionNotFound(String),
    /// A session id collides with one that already exists.
    #[error("session already exists: {0}")]
    SessionExists(String),
    /// The requested operation was refused because a safety invariant would
    /// otherwise be violated (fail closed).
    #[error("refused: {0}")]
    Refused(String),
}

impl CliError {
    pub fn invalid(field: impl Into<String>, value: impl Into<String>) -> Self {
        CliError::Invalid {
            field: field.into(),
            value: value.into(),
        }
    }
}
