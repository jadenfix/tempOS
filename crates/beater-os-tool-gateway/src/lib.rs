//! Runtime tool gateway for beaterOS.
//!
//! This crate is the first reusable tool-use boundary above `beater-osd`.
//! It resolves a registered tool, derives the action manifest at the mediation
//! point, asks the daemon store for admission, executes only admitted local shell
//! tools through the sandbox lane, and records receipts through the daemon.
//!
//! For the initial `local_shell` transport, the registry `content_digest` is the
//! digest of the exact command/argument vector. The gateway recomputes that
//! digest at mediation time, requires the invocation to pin it, and resolves the
//! registered tool against the same digest. This keeps durable
//! `tool_id@version#digest` evidence bound to the executable bytes/arguments
//! that actually entered the sandbox lane rather than to a caller-supplied tool
//! label.

use std::collections::{BTreeMap, BTreeSet};
use std::fs;
use std::path::{Path, PathBuf};

use beater_os_core::{
    ActionKind, ActionManifest, Budget, CapabilityGrant, CapabilityReceipt, CapabilityReceiptInput,
    CapabilitySelector, DataClass, DecisionResult, HashValue, PolicyDecision, ResourceKind,
    RiskClass, SideEffectClass, TaintLabel,
};
use beater_os_sandbox::{
    SandboxLimits, SandboxOutcome, SandboxRequest, SandboxStatus, execute as sandbox_execute,
    resolve_confined, safe_path_environment, validate_environment,
};
use beater_os_tool_registry::{ResolveRequest, ToolRegistry};
use beater_osd::{AdmissionOutcome, DaemonError, Store};
use chrono::Utc;
use sha2::{Digest, Sha256};
use thiserror::Error;

const LOCAL_SHELL_DIGEST_VERSION: &str = "beateros.local_shell_tool.v2";
const SAFE_PATH: &str = "/usr/bin:/bin:/usr/sbin:/sbin";

/// Result alias for gateway operations.
pub type GatewayResult<T> = Result<T, GatewayError>;

/// Canonical digest for a registered `local_shell` executable and arguments.
///
/// Registries should use this value as the registered tool `content_digest` for
/// the current local shell transport. Callers must also pass it as
/// [`LocalToolInvocation::expected_tool_digest`], giving the gateway a
/// three-way equality check over caller intent, registry pin, and executed
/// command.
pub fn local_shell_tool_digest(
    cwd: impl AsRef<Path>,
    command: &str,
    args: &[String],
) -> GatewayResult<String> {
    local_shell_tool_digest_with_environment(cwd, command, args, &local_shell_environment())
}

/// Canonical digest for a registered `local_shell` executable, arguments, and
/// explicit environment allowlist.
pub fn local_shell_tool_digest_with_environment(
    cwd: impl AsRef<Path>,
    command: &str,
    args: &[String],
    environment: &BTreeMap<String, String>,
) -> GatewayResult<String> {
    let executable = resolve_executable_path(cwd.as_ref(), command)?;
    let executable_bytes = fs::read(&executable).map_err(|source| GatewayError::ToolDigestIo {
        command: command.to_string(),
        source,
    })?;
    let executable_digest = Sha256::digest(&executable_bytes);
    let mut digest = Sha256::new();
    digest.update(LOCAL_SHELL_DIGEST_VERSION.as_bytes());
    digest.update([0]);
    digest.update(executable.display().to_string().as_bytes());
    digest.update([0]);
    digest.update(format!("{executable_digest:x}").as_bytes());
    digest.update([0]);
    for arg in args {
        digest.update(arg.as_bytes());
        digest.update([0]);
    }
    digest.update((environment.len() as u64).to_le_bytes());
    for (name, value) in environment {
        digest.update(name.as_bytes());
        digest.update([0]);
        digest.update(value.as_bytes());
        digest.update([0]);
    }
    Ok(format!("{:x}", digest.finalize()))
}

fn local_shell_environment() -> BTreeMap<String, String> {
    safe_path_environment()
}

/// Errors surfaced by the runtime tool gateway. Every error is fail-closed:
/// the caller must treat it as "nothing was admitted/executed/certified" unless
/// a returned [`GatewayOutcome`] explicitly carries an execution result.
#[derive(Debug, Error)]
pub enum GatewayError {
    #[error("runtime error: {0}")]
    Runtime(#[from] DaemonError),
    #[error("registry error: {0}")]
    Registry(#[from] beater_os_tool_registry::RegistryError),
    #[error("sandbox error: {0}")]
    Sandbox(#[from] beater_os_sandbox::SandboxError),
    #[error("registered tool {tool_id}@{version} uses unsupported transport {transport}")]
    UnsupportedTransport {
        tool_id: String,
        version: String,
        transport: String,
    },
    #[error("tool invocation requires at least one grant")]
    MissingGrant,
    #[error("cannot resolve executable for local shell command {command}")]
    ToolExecutableNotFound { command: String },
    #[error("cannot digest local shell command {command}: {source}")]
    ToolDigestIo {
        command: String,
        source: std::io::Error,
    },
    #[error("tool invocation must pin the registered digest for its exact command and args")]
    MissingToolDigest,
    #[error("expected tool digest does not match the exact command and args digest")]
    ToolDigestMismatch,
    #[error("named grants define no filesystem confinement prefix")]
    MissingConfinement,
    #[error("workspace {workspace_id} has no explicit tool allowlist")]
    MissingWorkspaceAllowlist { workspace_id: String },
    #[error("named grants do not cover registered tool required capabilities")]
    MissingToolCapability,
    #[error("required grant {grant_id} is no longer active")]
    MissingActiveGrant { grant_id: String },
    #[error(
        "resolved execution target changed after admission: expected {expected}, actual {actual}"
    )]
    ResolvedTargetChanged { expected: String, actual: String },
    #[error("observed side effects were not declared by the registered tool or invocation")]
    ObservedUndeclaredSideEffect,
}

/// A local shell tool invocation.
#[derive(Clone, Debug)]
pub struct LocalToolInvocation {
    pub session_id: String,
    pub tool_id: String,
    pub version: String,
    pub expected_tool_digest: Option<String>,
    pub command: String,
    pub args: Vec<String>,
    pub cwd: String,
    pub environment: BTreeMap<String, String>,
    pub required_grants: BTreeSet<String>,
    pub revoked_handles: BTreeSet<String>,
    pub action_id: String,
    pub risk_class: RiskClass,
    pub expected_side_effects: BTreeSet<SideEffectClass>,
    pub data_classes: BTreeSet<DataClass>,
    pub taint: BTreeSet<TaintLabel>,
    pub idempotency_key: Option<String>,
    pub compensation_plan: Option<String>,
    pub receipt_id: Option<String>,
    pub human_explanation: String,
    pub limits: SandboxLimits,
}

/// Result of a gateway-mediated invocation.
#[derive(Debug)]
pub struct GatewayOutcome {
    pub admission: AdmissionOutcome,
    pub decision: PolicyDecision,
    pub manifest: ActionManifest,
    pub execution: Option<SandboxOutcome>,
    pub receipt: Option<CapabilityReceipt>,
    pub evidence: Option<ExecutionReplayEvidence>,
}

/// Replay anchor for one hosted local-shell side effect.
#[derive(Clone, Debug, PartialEq, Eq)]
pub struct ExecutionReplayEvidence {
    pub session_id: String,
    pub action_id: String,
    pub tool_ref: String,
    pub manifest_hash: HashValue,
    pub proposal_seq: u64,
    pub proposal_hash: HashValue,
    pub decision_seq: u64,
    pub decision_hash: HashValue,
    pub admission_journal_root_hash: HashValue,
    pub receipt_journal_seq: u64,
    pub receipt_journal_hash: HashValue,
    pub receipt_seq: u64,
    pub receipt_hash: HashValue,
    pub receipt_root_hash: HashValue,
    pub final_journal_root_hash: HashValue,
}

/// Resolve, admit, execute, and receipt one local shell tool invocation.
pub fn execute_local_tool(
    store: &Store,
    registry: &ToolRegistry,
    invocation: LocalToolInvocation,
) -> GatewayResult<GatewayOutcome> {
    if invocation.required_grants.is_empty() {
        return Err(GatewayError::MissingGrant);
    }
    validate_environment(&invocation.environment, &invocation.limits)?;
    let inputs_digest = local_shell_tool_digest_with_environment(
        &invocation.cwd,
        &invocation.command,
        &invocation.args,
        &invocation.environment,
    )?;
    match &invocation.expected_tool_digest {
        Some(expected) if expected == &inputs_digest => {}
        Some(_) => return Err(GatewayError::ToolDigestMismatch),
        None => return Err(GatewayError::MissingToolDigest),
    }

    let projection = store.project(&invocation.session_id)?;
    if !registry.has_workspace_allowlist(&projection.session.workspace_id) {
        return Err(GatewayError::MissingWorkspaceAllowlist {
            workspace_id: projection.session.workspace_id,
        });
    }
    let tool = registry.resolve(
        &resolve_request(&invocation).in_workspace(projection.session.workspace_id.clone()),
    )?;
    if tool.manifest.transport != "local_shell" {
        return Err(GatewayError::UnsupportedTransport {
            tool_id: tool.manifest.tool_id.clone(),
            version: tool.manifest.version.clone(),
            transport: tool.manifest.transport.clone(),
        });
    }

    let now = Utc::now();
    let active_grants = projection.active_grants(now);
    if !tool_capabilities_covered(
        &active_grants,
        &invocation.required_grants,
        &tool.manifest.required_capabilities,
    ) {
        return Err(GatewayError::MissingToolCapability);
    }
    let confinement_prefixes = confinement_prefixes(&active_grants, &invocation.required_grants);
    if confinement_prefixes.is_empty() {
        return Err(GatewayError::MissingConfinement);
    }

    let resolved = resolve_confined(&invocation.cwd, &confinement_prefixes)?;
    let resolved_target = resolved.display().to_string();
    let mut inputs_summary = if invocation.args.is_empty() {
        invocation.command.clone()
    } else {
        format!("{} {}", invocation.command, invocation.args.join(" "))
    };
    if invocation.environment.len() > 1 {
        let names = invocation
            .environment
            .keys()
            .cloned()
            .collect::<Vec<_>>()
            .join(",");
        inputs_summary = format!("{inputs_summary} env=[{names}]");
    }
    let mut expected_side_effects = invocation
        .expected_side_effects
        .union(&tool.manifest.side_effects)
        .cloned()
        .collect::<BTreeSet<_>>();
    expected_side_effects.insert(SideEffectClass::LocalWrite);
    let tool_ref = format!(
        "{}@{}#{}",
        tool.manifest.tool_id, tool.manifest.version, tool.content_digest
    );
    let manifest = ActionManifest {
        action_id: invocation.action_id.clone(),
        session_id: invocation.session_id.clone(),
        tool_id: tool.manifest.tool_id.clone(),
        action_kind: ActionKind::Execute,
        target: CapabilitySelector {
            resource_kind: ResourceKind::FilePath,
            resource_id: resolved_target.clone(),
        },
        resolved_target: Some(CapabilitySelector {
            resource_kind: ResourceKind::FilePath,
            resource_id: resolved_target,
        }),
        inputs_digest: inputs_digest.clone(),
        inputs_summary,
        expected_outputs: Vec::new(),
        expected_side_effects: expected_side_effects.clone(),
        required_grants: invocation.required_grants.clone(),
        requested_budget: Budget::default(),
        risk_class: invocation.risk_class.max(tool.manifest.risk_class),
        data_classes: invocation.data_classes.clone(),
        taint: invocation.taint.clone(),
        idempotency_key: invocation.idempotency_key.clone(),
        payment_intent: None,
        compensation_plan: invocation.compensation_plan.clone(),
        human_explanation: invocation.human_explanation.clone(),
    };

    let admission = store.admit_action_with_revoked_handles(
        &invocation.session_id,
        manifest.clone(),
        invocation.revoked_handles.clone(),
    )?;
    let decision = admission.decision.clone();
    if decision.result != DecisionResult::Allowed {
        return Ok(GatewayOutcome {
            admission,
            decision,
            manifest,
            execution: None,
            receipt: None,
            evidence: None,
        });
    }

    let command = invocation.command.clone();
    let args = invocation.args.clone();
    let cwd = invocation.cwd.clone();
    let environment = invocation.environment.clone();
    let limits = invocation.limits.clone();
    let action_id = invocation.action_id.clone();
    let receipt_id = invocation.receipt_id.clone();
    let receipt_tool_id = manifest.tool_id.clone();
    let receipt_target = manifest.resolved_target.clone();
    let receipt_input_digest = inputs_digest.clone();
    let receipt_tool_ref = tool_ref.clone();
    let required_grants_for_lease = invocation.required_grants.clone();
    let required_tool_capabilities = tool.manifest.required_capabilities.clone();
    let admitted_target = manifest
        .resolved_target
        .as_ref()
        .unwrap_or(&manifest.target)
        .resource_id
        .clone();
    let (receipt_outcome, execution) =
        store.execute_and_append_receipt(&invocation.session_id, Utc::now(), |projection| {
            let active_grants = projection.active_grants(Utc::now());
            let active_grant_ids: BTreeSet<&str> = active_grants
                .iter()
                .map(|grant| grant.grant_id.as_str())
                .collect();
            for grant_id in &required_grants_for_lease {
                if !active_grant_ids.contains(grant_id.as_str()) {
                    return Err(GatewayError::MissingActiveGrant {
                        grant_id: grant_id.clone(),
                    });
                }
            }
            if !tool_capabilities_covered(
                &active_grants,
                &required_grants_for_lease,
                &required_tool_capabilities,
            ) {
                return Err(GatewayError::MissingToolCapability);
            }
            let confinement_prefixes =
                confinement_prefixes(&active_grants, &required_grants_for_lease);
            if confinement_prefixes.is_empty() {
                return Err(GatewayError::MissingConfinement);
            }
            let resolved = resolve_confined(&cwd, &confinement_prefixes)?;
            let actual_target = resolved.display().to_string();
            if actual_target != admitted_target {
                return Err(GatewayError::ResolvedTargetChanged {
                    expected: admitted_target.clone(),
                    actual: actual_target,
                });
            }
            let execution = sandbox_execute(&SandboxRequest {
                command,
                args,
                environment,
                working_dir: resolved.display().to_string(),
                path_prefixes: confinement_prefixes,
                limits,
            })?;

            let observed_effects: BTreeSet<SideEffectClass> = if execution.diff.is_empty() {
                BTreeSet::new()
            } else {
                BTreeSet::from([SideEffectClass::LocalWrite])
            };
            if !observed_effects.is_subset(&expected_side_effects) {
                return Err(GatewayError::ObservedUndeclaredSideEffect);
            }
            let certified_effects: Vec<SideEffectClass> =
                observed_effects.iter().cloned().collect();
            let side_effect_summary =
                side_effect_summary(&execution, &observed_effects, &expected_side_effects);
            let artifact_refs: Vec<String> = execution
                .diff
                .created
                .iter()
                .chain(execution.diff.modified.iter())
                .cloned()
                .collect();
            Ok((
                CapabilityReceiptInput {
                    receipt_id,
                    action_id,
                    tool_id: receipt_tool_id,
                    target: receipt_target
                        .map(|mut target| {
                            target.resource_id = execution.resolved_target.display().to_string();
                            target
                        })
                        .unwrap_or_else(|| CapabilitySelector {
                            resource_kind: ResourceKind::FilePath,
                            resource_id: execution.resolved_target.display().to_string(),
                        }),
                    started_at: now,
                    finished_at: Utc::now(),
                    status: execution.status_str().to_string(),
                    input_digest: receipt_input_digest,
                    output_digest: execution.stdout_digest(),
                    side_effect_summary,
                    side_effects: certified_effects,
                    external_ids: vec![format!("tool_ref={receipt_tool_ref}")],
                    artifact_refs,
                    payment_receipt: None,
                },
                execution,
            ))
        })?;
    let evidence = ExecutionReplayEvidence {
        session_id: manifest.session_id.clone(),
        action_id: manifest.action_id.clone(),
        tool_ref,
        manifest_hash: admission.decision.manifest_hash.clone(),
        proposal_seq: admission.proposal_record.seq,
        proposal_hash: admission.proposal_record.hash.clone(),
        decision_seq: admission.decision_record.seq,
        decision_hash: admission.decision_record.hash.clone(),
        admission_journal_root_hash: admission.decision_record.hash.clone(),
        receipt_journal_seq: receipt_outcome.receipt_record.seq,
        receipt_journal_hash: receipt_outcome.receipt_record.hash.clone(),
        receipt_seq: receipt_outcome.receipt.seq,
        receipt_hash: receipt_outcome.receipt.receipt_hash.clone(),
        receipt_root_hash: receipt_outcome.receipt.receipt_hash.clone(),
        final_journal_root_hash: receipt_outcome.receipt_record.hash.clone(),
    };

    Ok(GatewayOutcome {
        admission,
        decision,
        manifest,
        execution: Some(execution),
        receipt: Some(receipt_outcome.receipt),
        evidence: Some(evidence),
    })
}

fn resolve_request(invocation: &LocalToolInvocation) -> ResolveRequest {
    let request = ResolveRequest::new(&invocation.tool_id, &invocation.version);
    if let Some(digest) = &invocation.expected_tool_digest {
        request.expecting_digest(digest.clone())
    } else {
        request
    }
}

fn resolve_executable_path(cwd: &Path, command: &str) -> GatewayResult<PathBuf> {
    if command.contains('/') {
        let candidate = Path::new(command);
        let path = if candidate.is_absolute() {
            candidate.to_path_buf()
        } else {
            cwd.join(candidate)
        };
        return fs::canonicalize(&path).map_err(|_| GatewayError::ToolExecutableNotFound {
            command: command.to_string(),
        });
    }
    SAFE_PATH
        .split(':')
        .map(|dir| Path::new(dir).join(command))
        .find_map(|candidate| fs::canonicalize(candidate).ok())
        .ok_or_else(|| GatewayError::ToolExecutableNotFound {
            command: command.to_string(),
        })
}

fn confinement_prefixes(
    active_grants: &[CapabilityGrant],
    required_grants: &BTreeSet<String>,
) -> Vec<String> {
    let mut prefixes = BTreeSet::new();
    for grant in active_grants
        .iter()
        .filter(|grant| required_grants.contains(&grant.grant_id))
    {
        for prefix in &grant.constraints.path_prefixes {
            prefixes.insert(prefix.clone());
        }
        let selector = &grant.scope.selector;
        if selector.resource_kind == ResourceKind::FilePath && selector.resource_id != "*" {
            prefixes.insert(selector.resource_id.clone());
        }
    }
    prefixes.into_iter().collect()
}

fn tool_capabilities_covered(
    active_grants: &[CapabilityGrant],
    required_grants: &BTreeSet<String>,
    required_capabilities: &[beater_os_core::CapabilityScope],
) -> bool {
    required_capabilities.iter().all(|required| {
        required.actions.iter().all(|action| {
            active_grants
                .iter()
                .filter(|grant| required_grants.contains(&grant.grant_id))
                .any(|grant| grant.scope.allows(&required.selector, action))
        })
    })
}

fn side_effect_summary(
    execution: &SandboxOutcome,
    observed_effects: &BTreeSet<SideEffectClass>,
    expected_side_effects: &BTreeSet<SideEffectClass>,
) -> String {
    let mut summary = format!(
        "gateway local_shell status={} exit={:?} timed_out={} | OBSERVED {} effects={:?} | DECLARED expected_effects={:?}",
        execution.status_str(),
        execution.exit_code,
        execution.status == SandboxStatus::Timeout,
        execution.diff.summary(),
        observed_effects,
        expected_side_effects,
    );
    let observed_not_declared: Vec<SideEffectClass> = observed_effects
        .difference(expected_side_effects)
        .cloned()
        .collect();
    let declared_not_observed: Vec<SideEffectClass> = expected_side_effects
        .difference(observed_effects)
        .cloned()
        .collect();
    if !observed_not_declared.is_empty() || !declared_not_observed.is_empty() {
        summary.push_str(&format!(
            " | DIVERGENCE observed_not_declared={observed_not_declared:?} declared_not_observed={declared_not_observed:?}"
        ));
    }
    summary
}
