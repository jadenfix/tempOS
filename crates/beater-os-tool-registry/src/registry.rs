use std::collections::{BTreeMap, BTreeSet};

use beater_os_core::{
    BeaterOsResult, CapabilityScope, HashValue, RiskClass, SideEffectClass, ToolManifest, hash_json,
};
use chrono::{DateTime, Utc};
use serde::{Deserialize, Serialize};

use crate::error::{RegistryError, RegistryResult};

/// Test status tracked per tool version (`final.md` §10.14: "track test status").
#[derive(Clone, Copy, Debug, Default, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum TestStatus {
    /// Not yet evaluated. Treated as not-passing when the registry requires tests.
    #[default]
    Untested,
    Passing,
    Failing,
}

impl TestStatus {
    pub fn is_passing(self) -> bool {
        matches!(self, TestStatus::Passing)
    }
}

/// Trust state of a registered tool version. Only `Trusted` tools resolve.
#[derive(Clone, Debug, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum ToolTrust {
    Trusted,
    Quarantined { reason: String },
    Revoked { reason: String },
}

/// A signature attestation over a tool's content digest.
///
/// This crate does **not** invent cryptography (`final.md` §22.7). It records
/// the signer's identity and the digest the signature covers, and enforces a
/// *trust policy* over them: the publisher must be in the registry's trusted
/// set and the signed digest must equal the tool's actual content digest.
/// Verifying the `signature` bytes against a real key belongs to a crypto layer
/// (§13.12) that can populate `verified` before registration.
#[derive(Clone, Debug, PartialEq, Eq, Serialize, Deserialize)]
pub struct ToolSignature {
    pub publisher: String,
    pub key_id: String,
    /// The security digest this signature attests to.
    ///
    /// This digest binds the tool schema/artifact digest together with the
    /// manifest fields that affect authority: publisher, version, transport,
    /// required capabilities, side effects, risk class, and sandbox floor.
    pub content_digest: HashValue,
    /// Opaque signature material, verified out of band by a crypto layer.
    pub signature: String,
    /// Whether the crypto layer verified `signature` bytes against `key_id`.
    /// Defaults false so deserialized partial records fail closed.
    #[serde(default)]
    pub verified: bool,
}

/// A tool version registered in the registry: the core `ToolManifest` plus the
/// registry metadata needed to trust, pin, test-gate, and revoke it.
#[derive(Clone, Debug, PartialEq, Eq, Serialize, Deserialize)]
pub struct RegisteredTool {
    pub manifest: ToolManifest,
    /// Digest of the tool's pinned schema/artifact (see [`content_digest`]).
    pub content_digest: HashValue,
    #[serde(default)]
    pub signature: Option<ToolSignature>,
    #[serde(default)]
    pub test_status: TestStatus,
    pub trust: ToolTrust,
    pub registered_at: DateTime<Utc>,
    #[serde(default)]
    pub notes: String,
}

/// A per-tool version+digest pin (schema pinning §13.6, version rollback §13.10).
///
/// When a pin exists for a tool, only the pinned version *and* digest resolve —
/// this is how a workspace rolls back to a known-good tool and rejects a
/// silently-updated schema.
#[derive(Clone, Debug, PartialEq, Eq, Serialize, Deserialize)]
pub struct ToolPin {
    pub version: String,
    pub content_digest: HashValue,
}

/// Registry admission policy. Distrust-by-default (`final.md` §13.6).
///
/// The `#[serde(default)]` is deliberately on the **container**, not the fields:
/// any field missing from a policy JSON inherits the safe [`Default`] below
/// (signatures required, risk capped at `High`, sandbox floor `High`), rather
/// than the field type's own default (which would be `false`/`None` — an
/// ambient-trust fail-open). A partially specified or forward-migrated policy
/// therefore stays fail-closed; disabling a control must be an explicit,
/// auditable value (e.g. `"require_signature": false`, `"max_risk": null`).
#[derive(Clone, Debug, PartialEq, Eq, Serialize, Deserialize)]
#[serde(default)]
pub struct RegistryPolicy {
    /// Publishers whose signatures the registry will trust.
    pub trusted_publishers: BTreeSet<String>,
    /// Every tool must carry a signature from a trusted publisher.
    pub require_signature: bool,
    /// A tool resolves only if its tests are passing.
    pub require_passing_tests: bool,
    /// No tool above this risk class may be registered or resolved.
    pub max_risk: Option<RiskClass>,
    /// Tools at or above this risk class must declare `sandbox_required` (§13.8).
    pub require_sandbox_at_or_above: Option<RiskClass>,
}

impl Default for RegistryPolicy {
    /// A safe-by-default policy: unsigned tools are rejected, `Critical` tools
    /// are refused outright, and `High`+ tools must be sandboxed. Callers loosen
    /// this deliberately and auditably, never by omission.
    fn default() -> Self {
        Self {
            trusted_publishers: BTreeSet::new(),
            require_signature: true,
            require_passing_tests: false,
            max_risk: Some(RiskClass::High),
            require_sandbox_at_or_above: Some(RiskClass::High),
        }
    }
}

/// An append-only audit event emitted by registry mutations. Serializable so a
/// caller can persist it to the kernel journal; the registry keeps its own copy
/// too, so the registry is self-auditing without depending on the core journal.
#[derive(Clone, Debug, PartialEq, Eq, Serialize, Deserialize)]
#[serde(tag = "kind", rename_all = "snake_case")]
pub enum RegistryEvent {
    Registered {
        tool_id: String,
        version: String,
        content_digest: HashValue,
        publisher: String,
    },
    Pinned {
        tool_id: String,
        version: String,
        content_digest: HashValue,
    },
    Quarantined {
        tool_id: String,
        version: String,
        reason: String,
    },
    Revoked {
        tool_id: String,
        version: String,
        reason: String,
    },
    WorkspaceAllowlistSet {
        workspace_id: String,
        tool_ids: Vec<String>,
    },
}

/// A request to resolve a tool for use.
#[derive(Clone, Debug, PartialEq, Eq)]
pub struct ResolveRequest {
    pub tool_id: String,
    pub version: String,
    /// If set, the caller's own view of the tool digest must match the
    /// registered one — catches a tampered call site.
    pub expected_digest: Option<HashValue>,
    /// If set, the tool must be on that workspace's allowlist (when one exists).
    pub workspace_id: Option<String>,
}

impl ResolveRequest {
    pub fn new(tool_id: impl Into<String>, version: impl Into<String>) -> Self {
        Self {
            tool_id: tool_id.into(),
            version: version.into(),
            expected_digest: None,
            workspace_id: None,
        }
    }

    pub fn in_workspace(mut self, workspace_id: impl Into<String>) -> Self {
        self.workspace_id = Some(workspace_id.into());
        self
    }

    pub fn expecting_digest(mut self, digest: impl Into<HashValue>) -> Self {
        self.expected_digest = Some(digest.into());
        self
    }
}

/// The trustworthy tool registry.
///
/// Registration and resolution both fail closed against [`RegistryPolicy`].
/// The registry owns no ambient trust: a tool that was never registered, or was
/// quarantined/revoked, is never resolvable, and a pinned tool resolves only at
/// its pinned version and digest.
///
/// Performance note (per `docs/sota-systems-engineering.md`): lookups are
/// `O(log n)` over `BTreeMap`s and there is no IO, lock, or syscall on the
/// resolve hot path; a runtime front-end owns concurrency and persistence.
/// When signatures are required, resolve also recomputes the signed security
/// preimage so deserialized or policy-shifted registries still fail closed.
#[derive(Clone, Debug, Default, Serialize, Deserialize)]
pub struct ToolRegistry {
    policy: RegistryPolicy,
    /// tool_id -> version -> registered tool.
    tools: BTreeMap<String, BTreeMap<String, RegisteredTool>>,
    /// tool_id -> pin.
    pins: BTreeMap<String, ToolPin>,
    /// workspace_id -> allowed tool_ids.
    workspace_allowlists: BTreeMap<String, BTreeSet<String>>,
    /// Append-only audit log of registry mutations.
    events: Vec<RegistryEvent>,
}

impl ToolRegistry {
    pub fn new(policy: RegistryPolicy) -> Self {
        Self {
            policy,
            ..Default::default()
        }
    }

    pub fn policy(&self) -> &RegistryPolicy {
        &self.policy
    }

    pub fn events(&self) -> &[RegistryEvent] {
        &self.events
    }

    /// Register a tool version. Fails closed if the tool violates the static
    /// policy checks (signature, publisher trust, risk ceiling, sandbox floor),
    /// or if the version is already registered.
    pub fn register(&mut self, tool: RegisteredTool) -> RegistryResult<()> {
        let tool_id = tool.manifest.tool_id.clone();
        let version = tool.manifest.version.clone();

        if self
            .tools
            .get(&tool_id)
            .is_some_and(|versions| versions.contains_key(&version))
        {
            return Err(RegistryError::AlreadyRegistered { tool_id, version });
        }

        self.check_static_policy(&tool)?;

        let publisher = tool
            .signature
            .as_ref()
            .map(|signature| signature.publisher.clone())
            .unwrap_or_else(|| tool.manifest.publisher.clone());
        let content_digest = tool.content_digest.clone();
        self.tools
            .entry(tool_id.clone())
            .or_default()
            .insert(version.clone(), tool);
        self.events.push(RegistryEvent::Registered {
            tool_id,
            version,
            content_digest,
            publisher,
        });
        Ok(())
    }

    /// Pin a tool to an exact version and digest. Later resolutions of the tool
    /// must match both. The pin target must already be registered.
    pub fn pin(&mut self, tool_id: &str, version: &str) -> RegistryResult<()> {
        let tool = self.get(tool_id, version)?;
        let pin = ToolPin {
            version: version.to_string(),
            content_digest: tool.content_digest.clone(),
        };
        self.events.push(RegistryEvent::Pinned {
            tool_id: tool_id.to_string(),
            version: version.to_string(),
            content_digest: pin.content_digest.clone(),
        });
        self.pins.insert(tool_id.to_string(), pin);
        Ok(())
    }

    /// Quarantine a tool version: it stops resolving until trust is restored.
    pub fn quarantine(&mut self, tool_id: &str, version: &str, reason: &str) -> RegistryResult<()> {
        self.set_trust(
            tool_id,
            version,
            ToolTrust::Quarantined {
                reason: reason.to_string(),
            },
        )?;
        self.events.push(RegistryEvent::Quarantined {
            tool_id: tool_id.to_string(),
            version: version.to_string(),
            reason: reason.to_string(),
        });
        Ok(())
    }

    /// Revoke a tool version permanently.
    pub fn revoke(&mut self, tool_id: &str, version: &str, reason: &str) -> RegistryResult<()> {
        self.set_trust(
            tool_id,
            version,
            ToolTrust::Revoked {
                reason: reason.to_string(),
            },
        )?;
        self.events.push(RegistryEvent::Revoked {
            tool_id: tool_id.to_string(),
            version: version.to_string(),
            reason: reason.to_string(),
        });
        Ok(())
    }

    /// Set (replace) the tool allowlist for a workspace. When a workspace has an
    /// allowlist, only its tool ids resolve there.
    pub fn set_workspace_allowlist(
        &mut self,
        workspace_id: &str,
        tool_ids: impl IntoIterator<Item = String>,
    ) {
        let set: BTreeSet<String> = tool_ids.into_iter().collect();
        self.events.push(RegistryEvent::WorkspaceAllowlistSet {
            workspace_id: workspace_id.to_string(),
            tool_ids: set.iter().cloned().collect(),
        });
        self.workspace_allowlists
            .insert(workspace_id.to_string(), set);
    }

    /// Return whether a workspace has an explicit allowlist configured.
    ///
    /// `resolve` intentionally preserves an unrestricted global registry view
    /// for non-runtime callers. Runtime boundaries that execute tools should
    /// use this to require explicit workspace scoping before resolving.
    pub fn has_workspace_allowlist(&self, workspace_id: &str) -> bool {
        self.workspace_allowlists.contains_key(workspace_id)
    }

    /// Look up a registered tool without applying resolution policy.
    pub fn get(&self, tool_id: &str, version: &str) -> RegistryResult<&RegisteredTool> {
        self.tools
            .get(tool_id)
            .and_then(|versions| versions.get(version))
            .ok_or_else(|| RegistryError::Unregistered {
                tool_id: tool_id.to_string(),
                version: version.to_string(),
            })
    }

    /// Resolve a tool for use, applying every fail-closed check. Returns the
    /// registered tool only if it is registered, pin-conformant, trusted,
    /// digest-consistent, signed (if required), tested (if required), within the
    /// risk ceiling, sandboxed (if required), and workspace-allowed.
    ///
    /// Scoping note: a request with `workspace_id: None` is **not**
    /// workspace-scoped and skips the per-workspace allowlist. Callers that
    /// require scoping (e.g. the tool gateway) must always supply a workspace;
    /// omitting it yields the unrestricted global view of registered tools.
    pub fn resolve(&self, request: &ResolveRequest) -> RegistryResult<&RegisteredTool> {
        let tool = self.get(&request.tool_id, &request.version)?;

        // Version + schema pin (rollback / schema pinning).
        if let Some(pin) = self.pins.get(&request.tool_id) {
            if pin.version != request.version {
                return Err(RegistryError::VersionNotPinned {
                    tool_id: request.tool_id.clone(),
                    pinned: pin.version.clone(),
                    requested: request.version.clone(),
                });
            }
            if pin.content_digest != tool.content_digest {
                return Err(RegistryError::DigestMismatch {
                    tool_id: request.tool_id.clone(),
                    version: request.version.clone(),
                    expected: pin.content_digest.clone(),
                    found: tool.content_digest.clone(),
                });
            }
        }

        // Trust state.
        match &tool.trust {
            ToolTrust::Trusted => {}
            ToolTrust::Quarantined { reason } => {
                return Err(RegistryError::Quarantined {
                    tool_id: request.tool_id.clone(),
                    version: request.version.clone(),
                    reason: reason.clone(),
                });
            }
            ToolTrust::Revoked { reason } => {
                return Err(RegistryError::Revoked {
                    tool_id: request.tool_id.clone(),
                    version: request.version.clone(),
                    reason: reason.clone(),
                });
            }
        }

        // Caller-supplied digest must match the registered one (tamper check).
        if let Some(expected) = &request.expected_digest
            && expected != &tool.content_digest
        {
            return Err(RegistryError::DigestMismatch {
                tool_id: request.tool_id.clone(),
                version: request.version.clone(),
                expected: expected.clone(),
                found: tool.content_digest.clone(),
            });
        }

        // Static policy (re-checked at resolve so a tightened policy applies to
        // already-registered tools).
        self.check_static_policy(tool)?;

        if self.policy.require_passing_tests && !tool.test_status.is_passing() {
            return Err(RegistryError::TestsNotPassing {
                tool_id: request.tool_id.clone(),
                version: request.version.clone(),
            });
        }

        // Per-workspace allowlist (distrust-by-default within a scoped workspace).
        if let Some(workspace_id) = &request.workspace_id
            && let Some(allowed) = self.workspace_allowlists.get(workspace_id)
            && !allowed.contains(&request.tool_id)
        {
            return Err(RegistryError::WorkspaceNotAllowed {
                tool_id: request.tool_id.clone(),
                workspace: workspace_id.clone(),
            });
        }

        Ok(tool)
    }

    /// Static, trust-and-shape checks applied at both registration and resolve.
    fn check_static_policy(&self, tool: &RegisteredTool) -> RegistryResult<()> {
        let tool_id = &tool.manifest.tool_id;
        let version = &tool.manifest.version;

        if let Some(ceiling) = self.policy.max_risk
            && tool.manifest.risk_class > ceiling
        {
            return Err(RegistryError::RiskCeilingExceeded {
                tool_id: tool_id.clone(),
                version: version.clone(),
                risk: tool.manifest.risk_class,
                ceiling,
            });
        }

        if let Some(floor) = self.policy.require_sandbox_at_or_above
            && tool.manifest.risk_class >= floor
            && !tool.manifest.sandbox_required
        {
            return Err(RegistryError::SandboxRequired {
                tool_id: tool_id.clone(),
                version: version.clone(),
                risk: tool.manifest.risk_class,
                floor,
            });
        }

        if self.policy.require_signature {
            let Some(signature) = &tool.signature else {
                return Err(RegistryError::MissingSignature {
                    tool_id: tool_id.clone(),
                    version: version.clone(),
                });
            };
            // Trust is anchored on the SIGNATURE's publisher, never
            // `manifest.publisher`: the manifest is untrusted metadata and may
            // name a different (or spoofed) publisher. Only the signer's
            // identity — bound to the content digest below — is authoritative.
            if !self
                .policy
                .trusted_publishers
                .contains(&signature.publisher)
            {
                return Err(RegistryError::UntrustedPublisher {
                    tool_id: tool_id.clone(),
                    version: version.clone(),
                    publisher: signature.publisher.clone(),
                });
            }
            if !signature.verified {
                return Err(RegistryError::UnverifiedSignature {
                    tool_id: tool_id.clone(),
                    version: version.clone(),
                    key_id: signature.key_id.clone(),
                });
            }
            if signature.publisher != tool.manifest.publisher {
                return Err(RegistryError::SignaturePublisherMismatch {
                    tool_id: tool_id.clone(),
                    version: version.clone(),
                    signature_publisher: signature.publisher.clone(),
                    manifest_publisher: tool.manifest.publisher.clone(),
                });
            }
            let expected_signed_digest =
                tool_signature_digest(&tool.manifest, &tool.content_digest).map_err(|err| {
                    RegistryError::SignaturePreimageDigestFailed {
                        tool_id: tool_id.clone(),
                        version: version.clone(),
                        reason: err.to_string(),
                    }
                })?;
            if signature.content_digest != expected_signed_digest {
                return Err(RegistryError::SignatureDigestMismatch {
                    tool_id: tool_id.clone(),
                    version: version.clone(),
                    signed: signature.content_digest.clone(),
                    actual: expected_signed_digest,
                });
            }
        }

        Ok(())
    }

    fn set_trust(&mut self, tool_id: &str, version: &str, trust: ToolTrust) -> RegistryResult<()> {
        let tool = self
            .tools
            .get_mut(tool_id)
            .and_then(|versions| versions.get_mut(version))
            .ok_or_else(|| RegistryError::Unregistered {
                tool_id: tool_id.to_string(),
                version: version.to_string(),
            })?;
        tool.trust = trust;
        Ok(())
    }
}

/// Compute the content digest of a tool's schema/artifact bytes, so callers and
/// the registry agree on one digest function. Reuses the core SHA-256 hashing.
///
/// The `Result` is surfaced rather than hidden behind an empty-string sentinel:
/// a digest is a security value, and "hashing failed" must never quietly become
/// a digest that could compare equal to another failure. (In practice hashing a
/// string cannot fail.)
pub fn content_digest(schema: &str) -> BeaterOsResult<HashValue> {
    hash_json(&schema)
}

#[derive(Serialize)]
struct ToolSignaturePreimage<'a> {
    schema_version: &'static str,
    content_digest: &'a HashValue,
    tool_id: &'a str,
    publisher: &'a str,
    version: &'a str,
    transport: &'a str,
    required_capabilities: &'a Vec<CapabilityScope>,
    side_effects: &'a BTreeSet<SideEffectClass>,
    risk_class: RiskClass,
    sandbox_required: bool,
}

/// Digest that a tool signature must cover.
///
/// This is intentionally stronger than [`content_digest`]: the raw
/// schema/artifact digest remains the pinning value, while the signature digest
/// also binds all manifest fields that change authority or sandbox policy.
pub fn tool_signature_digest(
    manifest: &ToolManifest,
    content_digest: &HashValue,
) -> BeaterOsResult<HashValue> {
    hash_json(&ToolSignaturePreimage {
        schema_version: "beateros-tool-signature-v1",
        content_digest,
        tool_id: &manifest.tool_id,
        publisher: &manifest.publisher,
        version: &manifest.version,
        transport: &manifest.transport,
        required_capabilities: &manifest.required_capabilities,
        side_effects: &manifest.side_effects,
        risk_class: manifest.risk_class,
        sandbox_required: manifest.sandbox_required,
    })
}
