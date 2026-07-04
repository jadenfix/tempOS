//! `beater-os-tool-registry`: the trustworthy tool registry for the beaterOS
//! agent kernel.
//!
//! `final.md` is emphatic that tools are supply-chain risk: "an agent OS without
//! a trustworthy tool registry will inherit every supply-chain problem in the
//! ecosystem" (§10.14), and "tools are treated as untrusted unless registered,
//! pinned, signed, and policy-approved" (§3.1). This crate is that registry.
//!
//! It builds on the [`ToolManifest`](beater_os_core::ToolManifest) contract in
//! `beater-os-core` and adds the registry metadata and admission logic needed to
//! make tool selection safe:
//!
//! - **Signed manifests** — a tool must carry a signature from a trusted
//!   publisher whose signed digest matches the tool's actual content digest
//!   (§6.9, §13.10). Cryptographic signature *bytes* are verified by a separate
//!   crypto layer; this crate owns the trust policy over identity and digest, and
//!   deliberately invents no cryptography (§22.7).
//! - **Version + schema pinning** — a pinned tool resolves only at its pinned
//!   version and content digest, giving rollback and schema-pinning (§13.6,
//!   §13.10).
//! - **Risk metadata & sandbox floor** — a registry ceiling caps tool risk, and
//!   high-risk tools must declare `sandbox_required` (§13.8).
//! - **Test status** — a tool can be gated on passing tests (§10.14).
//! - **Per-workspace allowlists** — tools are scoped per workspace (§13.6).
//! - **Quarantine & revocation** — either stops a tool resolving (§13.6, §13.10).
//!
//! Everything **fails closed**: [`ToolRegistry::resolve`] returns a tool only if
//! every check passes. Mutations emit an append-only [`RegistryEvent`] audit
//! trail that a caller can also persist to the kernel journal.
//!
//! ```
//! use beater_os_core::{RiskClass, ToolManifest};
//! use beater_os_tool_registry::{
//!     content_digest, RegisteredTool, RegistryPolicy, ResolveRequest, TestStatus, ToolRegistry,
//!     ToolSignature, ToolTrust,
//! };
//! use chrono::Utc;
//! use std::collections::BTreeSet;
//!
//! let mut policy = RegistryPolicy::default();
//! policy.trusted_publishers = BTreeSet::from(["beater.tools".to_string()]);
//!
//! let mut registry = ToolRegistry::new(policy);
//! let schema = r#"{"name":"fs.read","args":["path"]}"#;
//! let digest = content_digest(schema).unwrap();
//!
//! let manifest = ToolManifest {
//!     tool_id: "fs.read".into(),
//!     publisher: "beater.tools".into(),
//!     version: "1.0.0".into(),
//!     transport: "local".into(),
//!     required_capabilities: Vec::new(),
//!     side_effects: BTreeSet::new(),
//!     risk_class: RiskClass::Low,
//!     sandbox_required: false,
//! };
//! registry.register(RegisteredTool {
//!     manifest,
//!     content_digest: digest.clone(),
//!     signature: Some(ToolSignature {
//!         publisher: "beater.tools".into(),
//!         key_id: "k1".into(),
//!         content_digest: digest.clone(),
//!         signature: "sig".into(),
//!     }),
//!     test_status: TestStatus::Passing,
//!     trust: ToolTrust::Trusted,
//!     registered_at: Utc::now(),
//!     notes: String::new(),
//! }).unwrap();
//!
//! assert!(registry.resolve(&ResolveRequest::new("fs.read", "1.0.0")).is_ok());
//! assert!(registry.resolve(&ResolveRequest::new("fs.read", "9.9.9")).is_err());
//! ```

mod error;
mod registry;

pub use error::{RegistryError, RegistryResult};
pub use registry::{
    RegisteredTool, RegistryEvent, RegistryPolicy, ResolveRequest, TestStatus, ToolPin,
    ToolRegistry, ToolSignature, ToolTrust, content_digest,
};
