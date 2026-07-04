//! Behavioral tests for the tool registry. Each fail-closed path from
//! `final.md` §6.9/§10.14/§13.6/§13.10/§26 is exercised end to end.

#![allow(clippy::expect_used, clippy::unwrap_used)]

use std::collections::BTreeSet;

use beater_os_core::{RiskClass, ToolManifest};
use beater_os_tool_registry::{
    RegisteredTool, RegistryError, RegistryPolicy, ResolveRequest, TestStatus, ToolRegistry,
    ToolSignature, ToolTrust, content_digest,
};
use chrono::Utc;

const PUBLISHER: &str = "beater.tools";

fn manifest(tool_id: &str, version: &str, risk: RiskClass, sandbox: bool) -> ToolManifest {
    ToolManifest {
        tool_id: tool_id.to_string(),
        publisher: PUBLISHER.to_string(),
        version: version.to_string(),
        transport: "local".to_string(),
        required_capabilities: Vec::new(),
        side_effects: BTreeSet::new(),
        risk_class: risk,
        sandbox_required: sandbox,
    }
}

/// A registered tool, signed by the trusted publisher, tests passing, trusted.
fn signed_tool(tool_id: &str, version: &str, risk: RiskClass, sandbox: bool) -> RegisteredTool {
    let digest = content_digest(&format!("{tool_id}@{version}")).expect("digest");
    RegisteredTool {
        manifest: manifest(tool_id, version, risk, sandbox),
        content_digest: digest.clone(),
        signature: Some(ToolSignature {
            publisher: PUBLISHER.to_string(),
            key_id: "k1".to_string(),
            content_digest: digest,
            signature: "sig".to_string(),
        }),
        test_status: TestStatus::Passing,
        trust: ToolTrust::Trusted,
        registered_at: Utc::now(),
        notes: String::new(),
    }
}

fn permissive_registry() -> ToolRegistry {
    let policy = RegistryPolicy {
        trusted_publishers: BTreeSet::from([PUBLISHER.to_string()]),
        require_signature: true,
        require_passing_tests: false,
        max_risk: Some(RiskClass::Critical),
        require_sandbox_at_or_above: None,
    };
    ToolRegistry::new(policy)
}

#[test]
fn registered_signed_tool_resolves() {
    let mut reg = permissive_registry();
    reg.register(signed_tool("fs.read", "1.0.0", RiskClass::Low, false))
        .expect("register");
    let resolved = reg
        .resolve(&ResolveRequest::new("fs.read", "1.0.0"))
        .expect("resolve");
    assert_eq!(resolved.manifest.tool_id, "fs.read");
}

#[test]
fn unregistered_tool_fails_closed() {
    let reg = permissive_registry();
    let err = reg
        .resolve(&ResolveRequest::new("ghost", "1.0.0"))
        .expect_err("unregistered must fail");
    assert!(matches!(err, RegistryError::Unregistered { .. }), "{err}");
}

#[test]
fn unknown_version_fails_closed() {
    let mut reg = permissive_registry();
    reg.register(signed_tool("fs.read", "1.0.0", RiskClass::Low, false))
        .unwrap();
    let err = reg
        .resolve(&ResolveRequest::new("fs.read", "2.0.0"))
        .expect_err("unknown version must fail");
    assert!(matches!(err, RegistryError::Unregistered { .. }), "{err}");
}

#[test]
fn quarantined_and_revoked_tools_stop_resolving() {
    let mut reg = permissive_registry();
    reg.register(signed_tool("net.fetch", "1.0.0", RiskClass::Medium, false))
        .unwrap();
    assert!(
        reg.resolve(&ResolveRequest::new("net.fetch", "1.0.0"))
            .is_ok()
    );

    reg.quarantine("net.fetch", "1.0.0", "cve-2026-1").unwrap();
    let err = reg
        .resolve(&ResolveRequest::new("net.fetch", "1.0.0"))
        .expect_err("quarantined must fail");
    assert!(matches!(err, RegistryError::Quarantined { .. }), "{err}");

    reg.revoke("net.fetch", "1.0.0", "compromised").unwrap();
    let err = reg
        .resolve(&ResolveRequest::new("net.fetch", "1.0.0"))
        .expect_err("revoked must fail");
    assert!(matches!(err, RegistryError::Revoked { .. }), "{err}");
}

#[test]
fn pin_rejects_other_versions() {
    let mut reg = permissive_registry();
    reg.register(signed_tool("fs.read", "1.0.0", RiskClass::Low, false))
        .unwrap();
    reg.register(signed_tool("fs.read", "2.0.0", RiskClass::Low, false))
        .unwrap();
    reg.pin("fs.read", "1.0.0").unwrap();

    assert!(
        reg.resolve(&ResolveRequest::new("fs.read", "1.0.0"))
            .is_ok()
    );
    let err = reg
        .resolve(&ResolveRequest::new("fs.read", "2.0.0"))
        .expect_err("non-pinned version must fail");
    assert!(
        matches!(err, RegistryError::VersionNotPinned { .. }),
        "{err}"
    );
}

#[test]
fn tampered_digest_is_rejected() {
    let mut reg = permissive_registry();
    reg.register(signed_tool("fs.read", "1.0.0", RiskClass::Low, false))
        .unwrap();
    let err = reg
        .resolve(&ResolveRequest::new("fs.read", "1.0.0").expecting_digest("deadbeef"))
        .expect_err("wrong caller digest must fail");
    assert!(matches!(err, RegistryError::DigestMismatch { .. }), "{err}");
}

#[test]
fn unsigned_tool_is_rejected_when_signature_required() {
    let mut reg = permissive_registry();
    let mut tool = signed_tool("fs.read", "1.0.0", RiskClass::Low, false);
    tool.signature = None;
    let err = reg.register(tool).expect_err("unsigned must be rejected");
    assert!(
        matches!(err, RegistryError::MissingSignature { .. }),
        "{err}"
    );
}

#[test]
fn untrusted_publisher_is_rejected() {
    let mut reg = permissive_registry();
    let mut tool = signed_tool("fs.read", "1.0.0", RiskClass::Low, false);
    if let Some(sig) = tool.signature.as_mut() {
        sig.publisher = "evil.tools".to_string();
    }
    let err = reg
        .register(tool)
        .expect_err("untrusted publisher rejected");
    assert!(
        matches!(err, RegistryError::UntrustedPublisher { .. }),
        "{err}"
    );
}

#[test]
fn signature_over_wrong_digest_is_rejected() {
    let mut reg = permissive_registry();
    let mut tool = signed_tool("fs.read", "1.0.0", RiskClass::Low, false);
    if let Some(sig) = tool.signature.as_mut() {
        sig.content_digest = "not-the-tool-digest".to_string();
    }
    let err = reg
        .register(tool)
        .expect_err("mismatched signed digest rejected");
    assert!(
        matches!(err, RegistryError::SignatureDigestMismatch { .. }),
        "{err}"
    );
}

#[test]
fn risk_ceiling_is_enforced() {
    let policy = RegistryPolicy {
        trusted_publishers: BTreeSet::from([PUBLISHER.to_string()]),
        require_signature: true,
        require_passing_tests: false,
        max_risk: Some(RiskClass::Medium),
        require_sandbox_at_or_above: None,
    };
    let mut reg = ToolRegistry::new(policy);
    let err = reg
        .register(signed_tool("deployer", "1.0.0", RiskClass::High, true))
        .expect_err("over-ceiling tool rejected");
    assert!(
        matches!(err, RegistryError::RiskCeilingExceeded { .. }),
        "{err}"
    );
}

#[test]
fn high_risk_tool_must_be_sandboxed() {
    let policy = RegistryPolicy {
        trusted_publishers: BTreeSet::from([PUBLISHER.to_string()]),
        require_signature: true,
        require_passing_tests: false,
        max_risk: Some(RiskClass::Critical),
        require_sandbox_at_or_above: Some(RiskClass::High),
    };
    let mut reg = ToolRegistry::new(policy);
    // High risk without sandbox → rejected.
    let err = reg
        .register(signed_tool("shell.exec", "1.0.0", RiskClass::High, false))
        .expect_err("unsandboxed high-risk tool rejected");
    assert!(
        matches!(err, RegistryError::SandboxRequired { .. }),
        "{err}"
    );
    // High risk *with* sandbox → accepted.
    reg.register(signed_tool("shell.exec", "1.0.0", RiskClass::High, true))
        .expect("sandboxed high-risk tool accepted");
}

#[test]
fn passing_tests_can_be_required() {
    let policy = RegistryPolicy {
        trusted_publishers: BTreeSet::from([PUBLISHER.to_string()]),
        require_signature: true,
        require_passing_tests: true,
        max_risk: Some(RiskClass::Critical),
        require_sandbox_at_or_above: None,
    };
    let mut reg = ToolRegistry::new(policy);
    let mut tool = signed_tool("fs.read", "1.0.0", RiskClass::Low, false);
    tool.test_status = TestStatus::Failing;
    reg.register(tool)
        .expect("register (test gate is at resolve)");
    let err = reg
        .resolve(&ResolveRequest::new("fs.read", "1.0.0"))
        .expect_err("failing tests must block resolution");
    assert!(
        matches!(err, RegistryError::TestsNotPassing { .. }),
        "{err}"
    );
}

#[test]
fn workspace_allowlist_scopes_tools() {
    let mut reg = permissive_registry();
    reg.register(signed_tool("fs.read", "1.0.0", RiskClass::Low, false))
        .unwrap();
    reg.register(signed_tool("net.fetch", "1.0.0", RiskClass::Medium, false))
        .unwrap();
    reg.set_workspace_allowlist("ws-secure", ["fs.read".to_string()]);

    // fs.read is allowlisted in ws-secure.
    assert!(
        reg.resolve(&ResolveRequest::new("fs.read", "1.0.0").in_workspace("ws-secure"))
            .is_ok()
    );
    // net.fetch is not.
    let err = reg
        .resolve(&ResolveRequest::new("net.fetch", "1.0.0").in_workspace("ws-secure"))
        .expect_err("tool not on workspace allowlist must fail");
    assert!(
        matches!(err, RegistryError::WorkspaceNotAllowed { .. }),
        "{err}"
    );
    // A workspace with no allowlist is unrestricted (any registered tool).
    assert!(
        reg.resolve(&ResolveRequest::new("net.fetch", "1.0.0").in_workspace("ws-open"))
            .is_ok()
    );
}

#[test]
fn double_registration_is_refused() {
    let mut reg = permissive_registry();
    reg.register(signed_tool("fs.read", "1.0.0", RiskClass::Low, false))
        .unwrap();
    let err = reg
        .register(signed_tool("fs.read", "1.0.0", RiskClass::Low, false))
        .expect_err("re-register must be refused");
    assert!(
        matches!(err, RegistryError::AlreadyRegistered { .. }),
        "{err}"
    );
}

#[test]
fn registry_is_serializable_roundtrip() {
    let mut reg = permissive_registry();
    reg.register(signed_tool("fs.read", "1.0.0", RiskClass::Low, false))
        .unwrap();
    reg.pin("fs.read", "1.0.0").unwrap();

    let json = serde_json::to_string(&reg).expect("serialize");
    let restored: ToolRegistry = serde_json::from_str(&json).expect("deserialize");

    // The restored registry resolves the pinned tool identically and keeps the
    // audit trail (Registered + Pinned).
    assert!(
        restored
            .resolve(&ResolveRequest::new("fs.read", "1.0.0"))
            .is_ok()
    );
    assert_eq!(restored.events().len(), 2);
}

#[test]
fn stricter_policy_refuses_over_ceiling_tool() {
    // A tool that a permissive registry accepts is refused outright by a
    // registry whose risk ceiling is lower — the check is on the policy, not on
    // when the tool was first seen.
    let mut permissive = permissive_registry();
    permissive
        .register(signed_tool("deployer", "1.0.0", RiskClass::High, true))
        .expect("permissive registry accepts high-risk tool");

    let strict_policy = RegistryPolicy {
        trusted_publishers: BTreeSet::from([PUBLISHER.to_string()]),
        require_signature: true,
        require_passing_tests: false,
        max_risk: Some(RiskClass::Medium),
        require_sandbox_at_or_above: None,
    };
    let mut strict = ToolRegistry::new(strict_policy);
    let err = strict
        .register(signed_tool("deployer", "1.0.0", RiskClass::High, true))
        .expect_err("strict registry refuses over-ceiling tool");
    assert!(
        matches!(err, RegistryError::RiskCeilingExceeded { .. }),
        "{err}"
    );
}

#[test]
fn partial_policy_json_stays_fail_closed() {
    // Regression guard: a policy JSON that omits fields must inherit the SAFE
    // defaults (container-level serde default), not the field-type defaults
    // (which would be an ambient-trust fail-open).
    let policy: RegistryPolicy =
        serde_json::from_str("{}").expect("deserialize empty policy object");
    assert!(
        policy.require_signature,
        "missing require_signature must default to true, not false"
    );
    assert_eq!(policy.max_risk, Some(RiskClass::High));
    assert_eq!(policy.require_sandbox_at_or_above, Some(RiskClass::High));

    // An unsigned tool is refused by a registry built from that empty policy.
    let mut reg = ToolRegistry::new(policy);
    let mut tool = signed_tool("x", "1.0.0", RiskClass::Low, false);
    tool.signature = None;
    let err = reg
        .register(tool)
        .expect_err("empty-policy registry must still require a signature");
    assert!(
        matches!(err, RegistryError::MissingSignature { .. }),
        "{err}"
    );
}

#[test]
fn full_registry_json_roundtrips_via_empty_policy_safely() {
    // A registry deserialized with `"policy": {}` must be fail-closed, matching
    // the container-default behavior above end to end.
    let json = r#"{"policy":{},"tools":{},"pins":{},"workspace_allowlists":{},"events":[]}"#;
    let reg: ToolRegistry = serde_json::from_str(json).expect("deserialize registry");
    assert!(reg.policy().require_signature);
    assert_eq!(reg.policy().max_risk, Some(RiskClass::High));
}

#[test]
fn mutations_emit_audit_events() {
    let mut reg = permissive_registry();
    reg.register(signed_tool("fs.read", "1.0.0", RiskClass::Low, false))
        .unwrap();
    reg.pin("fs.read", "1.0.0").unwrap();
    reg.quarantine("fs.read", "1.0.0", "audit").unwrap();
    // Registered, Pinned, Quarantined.
    assert_eq!(reg.events().len(), 3);
}
