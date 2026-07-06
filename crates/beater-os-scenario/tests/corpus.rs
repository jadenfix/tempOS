//! Drive the whole scenario corpus through the real Rust `PolicyEngine`.
//!
//! This is intentionally not an allowlisted drift sentinel. Current scenarios
//! must compile into current contracts, use an explicitly allowed registered
//! tool, and match the product engine with registry-grounding enabled.

use std::error::Error;
use std::path::{Path, PathBuf};

use beater_os_scenario::{ScenarioError, evaluate_with_fixture_root};

fn corpus_dir() -> PathBuf {
    Path::new(env!("CARGO_MANIFEST_DIR")).join("../../scenarios")
}

fn repo_dir() -> PathBuf {
    Path::new(env!("CARGO_MANIFEST_DIR")).join("../..")
}

fn collect_scenarios(dir: &Path, out: &mut Vec<PathBuf>) -> std::io::Result<()> {
    for entry in std::fs::read_dir(dir)? {
        let path = entry?.path();
        if path.is_dir() {
            collect_scenarios(&path, out)?;
        } else if path
            .file_name()
            .and_then(|name| name.to_str())
            .is_some_and(|name| name.ends_with(".scenario.json"))
        {
            out.push(path);
        }
    }
    Ok(())
}

#[test]
fn real_engine_agrees_with_registered_scenario_corpus() -> Result<(), Box<dyn Error>> {
    let dir = corpus_dir();
    let repo = repo_dir();
    let mut files = Vec::new();
    collect_scenarios(&dir, &mut files)?;
    files.sort();
    assert!(
        !files.is_empty(),
        "no scenario files found under {}",
        dir.display()
    );

    let mut failures = Vec::new();
    for path in &files {
        let rel = path
            .strip_prefix(&dir)
            .unwrap_or(path)
            .to_string_lossy()
            .into_owned();
        let json = std::fs::read_to_string(path)?;
        match evaluate_with_fixture_root(&json, &repo) {
            Ok(outcome) if outcome.passed() => {}
            Ok(outcome) => failures.push(format!(
                "{rel}: {}",
                outcome
                    .failure_reason()
                    .unwrap_or_else(|| "unknown scenario failure".to_string())
            )),
            Err(ScenarioError::ToolNotAllowed {
                scenario_id,
                tool_id,
            }) => failures.push(format!(
                "{rel}: scenario {scenario_id} manifest tool {tool_id} is not in allowed_tools"
            )),
            Err(ScenarioError::ToolRegistryMissing {
                scenario_id,
                tool_id,
            }) => failures.push(format!(
                "{rel}: scenario {scenario_id} allowed tool {tool_id} is missing from tool_registry"
            )),
            Err(ScenarioError::ToolRegistryNotAllowed {
                scenario_id,
                tool_id,
            }) => failures.push(format!(
                "{rel}: scenario {scenario_id} registry tool {tool_id} is not in allowed_tools"
            )),
            Err(ScenarioError::ToolRegistryIdentityMismatch {
                scenario_id,
                registry_key,
                tool_id,
            }) => failures.push(format!(
                "{rel}: scenario {scenario_id} registry key {registry_key} contains mismatched tool_id {tool_id}"
            )),
            Err(err) => failures.push(format!("{rel}: could not evaluate: {err}")),
        }
    }

    assert!(
        failures.is_empty(),
        "scenario corpus drift against real PolicyEngine:\n{}",
        failures.join("\n")
    );
    Ok(())
}
