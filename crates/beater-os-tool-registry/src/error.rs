use thiserror::Error;

use beater_os_core::RiskClass;

/// Result alias for tool-registry operations.
pub type RegistryResult<T> = Result<T, RegistryError>;

/// Why a registration or resolution was refused.
///
/// Every variant is a *closed door*: the registry never resolves a tool it is
/// unsure about. Unknown, unpinned-mismatch, untrusted, untested (when
/// required), over-risk, unsandboxed-high-risk, quarantined, and revoked tools
/// all fail closed, per `final.md` §6.9, §10.14, §13.6, §13.10, and §26.
#[derive(Debug, Clone, PartialEq, Eq, Error)]
pub enum RegistryError {
    #[error("tool {tool_id}@{version} is not registered")]
    Unregistered { tool_id: String, version: String },

    #[error("tool {tool_id} is pinned to version {pinned}, but {requested} was requested")]
    VersionNotPinned {
        tool_id: String,
        pinned: String,
        requested: String,
    },

    #[error("content digest mismatch for {tool_id}@{version}: expected {expected}, found {found}")]
    DigestMismatch {
        tool_id: String,
        version: String,
        expected: String,
        found: String,
    },

    #[error("tool {tool_id}@{version} is quarantined: {reason}")]
    Quarantined {
        tool_id: String,
        version: String,
        reason: String,
    },

    #[error("tool {tool_id}@{version} is revoked: {reason}")]
    Revoked {
        tool_id: String,
        version: String,
        reason: String,
    },

    #[error("tool {tool_id}@{version} has no signature but the registry requires one")]
    MissingSignature { tool_id: String, version: String },

    #[error("publisher {publisher} of {tool_id}@{version} is not in the trusted-publisher set")]
    UntrustedPublisher {
        tool_id: String,
        version: String,
        publisher: String,
    },

    #[error("signature for {tool_id}@{version} covers digest {signed}, not the tool's {actual}")]
    SignatureDigestMismatch {
        tool_id: String,
        version: String,
        signed: String,
        actual: String,
    },

    #[error("tool {tool_id}@{version} is not marked test-passing but the registry requires it")]
    TestsNotPassing { tool_id: String, version: String },

    #[error("tool {tool_id}@{version} risk {risk:?} exceeds the registry ceiling {ceiling:?}")]
    RiskCeilingExceeded {
        tool_id: String,
        version: String,
        risk: RiskClass,
        ceiling: RiskClass,
    },

    #[error(
        "tool {tool_id}@{version} at risk {risk:?} must declare sandbox_required (floor {floor:?})"
    )]
    SandboxRequired {
        tool_id: String,
        version: String,
        risk: RiskClass,
        floor: RiskClass,
    },

    #[error("tool {tool_id} is not on the allowlist for workspace {workspace}")]
    WorkspaceNotAllowed { tool_id: String, workspace: String },

    #[error("tool {tool_id}@{version} is already registered; re-registration must be explicit")]
    AlreadyRegistered { tool_id: String, version: String },
}
