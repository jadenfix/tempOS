//! Scenario corpus runner for the real beaterOS policy engine.
//!
//! The language-neutral corpus under `scenarios/**/*.scenario.json` is checked
//! by the Python conformance oracle. This crate drives the same probes through
//! the Rust product `PolicyEngine` with registry-grounded admission enabled, so
//! corpus drift against the runtime engine is visible in CI instead of hidden by
//! the reference implementation.

use std::{
    collections::{BTreeMap, BTreeSet},
    path::{Component, Path},
};

use beater_os_core::{
    ActionManifest, AdmissionContext, ApprovalEvidence, BeaterOsError, CapabilityGrant,
    DecisionResult, PaymentMandate, PolicyEngine, ScenarioManifest, SimulationEvidence,
    ToolManifest,
};
use chrono::{DateTime, Utc};
use serde::Deserialize;
use thiserror::Error;

/// Why a scenario could not be evaluated.
#[derive(Debug, Error)]
pub enum ScenarioError {
    #[error("could not parse scenario JSON: {0}")]
    Parse(#[from] serde_json::Error),
    #[error("policy engine error: {0}")]
    Policy(#[from] BeaterOsError),
    #[error("scenario {scenario_id} manifest tool {tool_id} is not listed in allowed_tools")]
    ToolNotAllowed {
        scenario_id: String,
        tool_id: String,
    },
    #[error("scenario {scenario_id} allowed tool {tool_id} is missing from tool_registry")]
    ToolRegistryMissing {
        scenario_id: String,
        tool_id: String,
    },
    #[error("scenario {scenario_id} tool_registry entry {tool_id} is not listed in allowed_tools")]
    ToolRegistryNotAllowed {
        scenario_id: String,
        tool_id: String,
    },
    #[error(
        "scenario {scenario_id} tool_registry key {registry_key} contains mismatched tool_id {tool_id}"
    )]
    ToolRegistryIdentityMismatch {
        scenario_id: String,
        registry_key: String,
        tool_id: String,
    },
    #[error("scenario {scenario_id} fixture {fixture_name} has invalid path {path}: {reason}")]
    InvalidFixturePath {
        scenario_id: String,
        fixture_name: String,
        path: String,
        reason: &'static str,
    },
    #[error("scenario {scenario_id} fixture {fixture_name} does not exist: {path}")]
    FixtureNotFound {
        scenario_id: String,
        fixture_name: String,
        path: String,
    },
}

/// A scenario file from `scenarios/**/*.scenario.json`.
#[derive(Debug, Deserialize)]
pub struct Scenario {
    pub scenario: ScenarioManifest,
    #[serde(default)]
    pub tool_registry: BTreeMap<String, ToolManifest>,
    pub probe: Probe,
}

/// The admission probe: a manifest evaluated against context with an expected outcome.
#[derive(Debug, Deserialize)]
pub struct Probe {
    pub context: ProbeContext,
    pub manifest: ActionManifest,
    pub expected_result: DecisionResult,
    #[serde(default)]
    pub must_be_blocked: bool,
}

/// Admission context fields stored in scenario JSON.
#[derive(Debug, Deserialize)]
pub struct ProbeContext {
    pub now: DateTime<Utc>,
    pub actor_id: String,
    pub session_id: String,
    pub policy_version: String,
    #[serde(default)]
    pub grants: Vec<CapabilityGrant>,
    #[serde(default)]
    pub approvals: Vec<ApprovalEvidence>,
    #[serde(default)]
    pub simulations: Vec<SimulationEvidence>,
    #[serde(default)]
    pub mandates: Vec<PaymentMandate>,
    #[serde(default)]
    pub revoked_handles: BTreeSet<String>,
}

impl ProbeContext {
    fn into_admission(self, tool_registry: BTreeMap<String, ToolManifest>) -> AdmissionContext {
        AdmissionContext {
            now: self.now,
            actor_id: self.actor_id,
            session_id: self.session_id,
            policy_version: self.policy_version,
            grants: self.grants,
            approvals: self.approvals,
            simulations: self.simulations,
            mandates: self.mandates,
            revoked_handles: self.revoked_handles,
            tool_registry,
            require_registered_tools: true,
        }
    }
}

/// The result of running one scenario through the real engine.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct ScenarioOutcome {
    pub scenario_id: String,
    pub expected: DecisionResult,
    pub actual: DecisionResult,
    pub must_be_blocked: bool,
    pub was_blocked: bool,
    pub result_matches: bool,
    pub block_invariant_holds: bool,
    pub explanation: String,
    pub matched_rules: Vec<String>,
}

impl ScenarioOutcome {
    /// The scenario passed iff the engine result matched and the block invariant held.
    pub fn passed(&self) -> bool {
        self.result_matches && self.block_invariant_holds
    }

    /// A human-readable failure reason, or `None` if the scenario passed.
    pub fn failure_reason(&self) -> Option<String> {
        if self.passed() {
            return None;
        }
        let mut reasons = Vec::new();
        if !self.result_matches {
            reasons.push(format!(
                "expected result {:?}, engine returned {:?}",
                self.expected, self.actual
            ));
        }
        if !self.block_invariant_holds {
            reasons.push(format!(
                "must_be_blocked={} but engine {} the action",
                self.must_be_blocked,
                if self.was_blocked {
                    "blocked"
                } else {
                    "allowed"
                }
            ));
        }
        Some(format!(
            "{}; explanation={}; rules={:?}",
            reasons.join("; "),
            self.explanation,
            self.matched_rules
        ))
    }
}

/// Evaluate a single scenario JSON document against the real `PolicyEngine`.
pub fn evaluate(json: &str) -> Result<ScenarioOutcome, ScenarioError> {
    let scenario: Scenario = serde_json::from_str(json)?;
    evaluate_scenario(scenario)
}

/// Evaluate a scenario JSON document and require every fixture reference to exist.
pub fn evaluate_with_fixture_root(
    json: &str,
    fixture_root: &Path,
) -> Result<ScenarioOutcome, ScenarioError> {
    let scenario: Scenario = serde_json::from_str(json)?;
    validate_fixture_paths(&scenario.scenario, Some(fixture_root))?;
    evaluate_scenario(scenario)
}

/// Evaluate a parsed scenario against the real `PolicyEngine`.
pub fn evaluate_scenario(scenario: Scenario) -> Result<ScenarioOutcome, ScenarioError> {
    let scenario_id = scenario.scenario.scenario_id.clone();
    let expected = scenario.probe.expected_result;
    let must_be_blocked = scenario.probe.must_be_blocked;
    let manifest = scenario.probe.manifest;
    validate_fixture_paths(&scenario.scenario, None)?;
    let tool_registry =
        scenario_tool_registry(&scenario.scenario, scenario.tool_registry, &manifest)?;
    let ctx = scenario.probe.context.into_admission(tool_registry);

    let decision = PolicyEngine::new().admit(&manifest, &ctx)?;
    let actual = decision.result;
    let was_blocked = actual != DecisionResult::Allowed;
    let result_matches = actual == expected;

    Ok(ScenarioOutcome {
        scenario_id,
        expected,
        actual,
        must_be_blocked,
        was_blocked,
        result_matches,
        block_invariant_holds: must_be_blocked == was_blocked,
        explanation: decision.explanation,
        matched_rules: decision.matched_rules,
    })
}

fn scenario_tool_registry(
    scenario: &ScenarioManifest,
    tool_registry: BTreeMap<String, ToolManifest>,
    manifest: &ActionManifest,
) -> Result<BTreeMap<String, ToolManifest>, ScenarioError> {
    if !scenario.allowed_tools.contains(&manifest.tool_id) {
        return Err(ScenarioError::ToolNotAllowed {
            scenario_id: scenario.scenario_id.clone(),
            tool_id: manifest.tool_id.clone(),
        });
    }

    for tool_id in &scenario.allowed_tools {
        if !tool_registry.contains_key(tool_id) {
            return Err(ScenarioError::ToolRegistryMissing {
                scenario_id: scenario.scenario_id.clone(),
                tool_id: tool_id.clone(),
            });
        }
    }
    for (tool_id, tool) in &tool_registry {
        if !scenario.allowed_tools.contains(tool_id) {
            return Err(ScenarioError::ToolRegistryNotAllowed {
                scenario_id: scenario.scenario_id.clone(),
                tool_id: tool_id.clone(),
            });
        }
        if tool.tool_id != *tool_id {
            return Err(ScenarioError::ToolRegistryIdentityMismatch {
                scenario_id: scenario.scenario_id.clone(),
                registry_key: tool_id.clone(),
                tool_id: tool.tool_id.clone(),
            });
        }
    }

    Ok(tool_registry)
}

fn validate_fixture_paths(
    scenario: &ScenarioManifest,
    fixture_root: Option<&Path>,
) -> Result<(), ScenarioError> {
    for (fixture_name, path) in &scenario.fixtures {
        let fixture_path = Path::new(path);
        if path.is_empty() {
            return Err(invalid_fixture_path(
                scenario,
                fixture_name,
                path,
                "path is empty",
            ));
        }
        if fixture_path.is_absolute() {
            return Err(invalid_fixture_path(
                scenario,
                fixture_name,
                path,
                "path must be relative",
            ));
        }
        if !path.starts_with("fixtures/") {
            return Err(invalid_fixture_path(
                scenario,
                fixture_name,
                path,
                "path must stay under fixtures/",
            ));
        }
        if fixture_path.components().any(|component| {
            matches!(
                component,
                Component::ParentDir
                    | Component::RootDir
                    | Component::Prefix(_)
                    | Component::CurDir
            )
        }) {
            return Err(invalid_fixture_path(
                scenario,
                fixture_name,
                path,
                "path contains an unsafe component",
            ));
        }
        if let Some(root) = fixture_root {
            let full_path = root.join(fixture_path);
            if !full_path.is_file() {
                return Err(ScenarioError::FixtureNotFound {
                    scenario_id: scenario.scenario_id.clone(),
                    fixture_name: fixture_name.clone(),
                    path: full_path.display().to_string(),
                });
            }
        }
    }
    Ok(())
}

fn invalid_fixture_path(
    scenario: &ScenarioManifest,
    fixture_name: &str,
    path: &str,
    reason: &'static str,
) -> ScenarioError {
    ScenarioError::InvalidFixturePath {
        scenario_id: scenario.scenario_id.clone(),
        fixture_name: fixture_name.to_string(),
        path: path.to_string(),
        reason,
    }
}
