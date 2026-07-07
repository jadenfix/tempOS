//! Bounded runtime worker supervisor for local-shell agent work.
//!
//! This binary is intentionally a thin service layer over `AgentRuntime`.
//! It does not receive direct store mutation authority, does not treat
//! `Allowed` policy decisions as execution authority, and does not retry
//! expired side effects. Each cycle delegates to the runtime's supervised
//! worker primitive: reconcile expired open leases as `outcome_unknown`, then
//! claim and complete runnable work through daemon-owned execution leases.

use std::collections::{BTreeMap, BTreeSet};
use std::path::PathBuf;
use std::process::ExitCode;
use std::thread;
use std::time::Duration;

use beater_os_core::{RiskClass, SideEffectClass};
use beater_os_runtime::{
    AgentRuntime, RuntimeLocalShellWorkerLoopRequest, RuntimeLocalShellWorkerLoopStopReason,
    RuntimeLocalShellWorkerRequest, RuntimeSupervisedLocalShellWorkerCycleRequest,
};
use beater_os_sandbox::safe_path_environment;
use beater_os_tool_gateway::local_shell_tool_digest_with_environment;
use serde::Serialize;

const DEFAULT_MAX_ACTIONS: usize = 16;
const DEFAULT_MAX_RECOVERIES: usize = 16;
const DEFAULT_MAX_CYCLES: usize = 1;
const DEFAULT_IDLE_SLEEP_MS: u64 = 250;

const USAGE: &str = "\
beater-os-runtime-worker - supervised beaterOS runtime worker

USAGE:
    beater-os-runtime-worker supervise-local-shell --root <path> --session-id <id> --cwd <path> --command <cmd> [options]

OPTIONS:
    --arg <value>                    Repeatable command argument
    --tool <id>                      Tool id to match/register (default: shell)
    --tool-version <version>         Optional pinned tool version
    --tool-digest <digest>           Optional pinned command digest; computed when omitted
    --side-effect local_write        Repeatable side effect declaration (default: local_write)
    --timeout-secs <n>               Sandbox timeout seconds
    --max-output-bytes <n>           Sandbox output byte cap
    --max-actions <n>                Per-cycle action cap (default: 16)
    --max-recoveries <n>             Per-cycle recovery cap (default: 16)
    --max-cycles <n>                 Bounded supervisor cycles (default: 1)
    --idle-sleep-ms <n>              Sleep between blocked cycles (default: 250)
    --initial-lease-ms <n>           Initial worker lease duration
    --heartbeat-interval-ms <n>      Heartbeat interval while sandbox runs
    --heartbeat-extend-ms <n>        Heartbeat lease extension
    --worker-id <id>                 Worker id for claims/heartbeats
    --heartbeat-evidence-ref <ref>   Repeatable heartbeat evidence reference
    --reconciled-by <id>             Recovery actor id
    --recovery-reason <text>         Recovery reason for expired leases
    --recovery-evidence-ref <ref>    Repeatable recovery evidence reference
    --json                          Emit machine-readable JSON
";

fn main() -> ExitCode {
    match run(std::env::args().collect()) {
        Ok(()) => ExitCode::SUCCESS,
        Err(message) => {
            eprintln!("{message}");
            ExitCode::FAILURE
        }
    }
}

fn run(args: Vec<String>) -> Result<(), String> {
    let config = SupervisorConfig::parse(&args)?;
    let report = run_supervisor(&config)?;
    if config.json {
        let output = serde_json::to_string_pretty(&report)
            .map_err(|err| format!("failed to serialize supervisor report: {err}"))?;
        println!("{output}");
    } else {
        println!("runtime worker supervisor OK");
        println!("  session: {}", report.session_id);
        println!("  cycles: {}", report.cycles);
        println!("  executions: {}", report.executions);
        println!("  recoveries: {}", report.recoveries);
        println!("  stop: {}", report.stop_reason);
        println!("  runnable: {}", report.runnable_pending_actions);
        println!("  open leases: {}", report.open_execution_leases);
    }
    report.ensure_complete()?;
    Ok(())
}

fn run_supervisor(config: &SupervisorConfig) -> Result<SupervisorReport, String> {
    if config.max_cycles == 0 {
        return Err("max-cycles must be greater than zero".to_string());
    }
    if config.max_actions == 0 {
        return Err("max-actions must be greater than zero".to_string());
    }
    if config.max_recoveries == 0 {
        return Err("max-recoveries must be greater than zero".to_string());
    }
    let environment = safe_path_environment();
    let tool_digest = match &config.tool_digest {
        Some(digest) => digest.clone(),
        None => local_shell_tool_digest_with_environment(
            &config.cwd,
            &config.command,
            &config.args,
            &environment,
        )
        .map_err(|err| format!("failed to compute local-shell tool digest: {err}"))?,
    };
    let runtime = AgentRuntime::open(&config.root)
        .map_err(|err| format!("failed to open runtime store: {err}"))?;

    let mut cycles = 0usize;
    let mut executions = 0usize;
    let mut recoveries = 0usize;
    let mut stop_reason = "not_started".to_string();
    let mut receipts = 0usize;
    let mut runnable_pending_actions = 0usize;
    let mut open_execution_leases = 0usize;
    let mut live_open_execution_leases = 0usize;
    let mut expired_recoverable_execution_leases = 0usize;
    let mut execution_reconciliations = 0usize;

    for _ in 0..config.max_cycles {
        cycles += 1;
        let outcome = runtime
            .run_supervised_local_shell_worker_cycle(
                RuntimeSupervisedLocalShellWorkerCycleRequest {
                    max_recoveries: config.max_recoveries,
                    recovery_reason: config.recovery_reason.clone(),
                    reconciled_by: config.reconciled_by.clone(),
                    recovery_evidence_refs: config.recovery_evidence_refs.clone(),
                    worker_loop: RuntimeLocalShellWorkerLoopRequest {
                        max_actions: config.max_actions,
                        worker: RuntimeLocalShellWorkerRequest {
                            session_id: config.session_id.clone(),
                            action_id: None,
                            lease_id: None,
                            tool: Some(config.tool.clone()),
                            tool_version: config.tool_version.clone(),
                            tool_digest: Some(tool_digest.clone()),
                            command: config.command.clone(),
                            args: config.args.clone(),
                            cwd: config.cwd.clone(),
                            env: BTreeMap::new(),
                            side_effects: config.side_effects.clone(),
                            risk: Some(config.risk),
                            receipt_id: None,
                            timeout_secs: config.timeout_secs,
                            max_output_bytes: config.max_output_bytes,
                            initial_lease_ms: config.initial_lease_ms,
                            heartbeat_interval_ms: config.heartbeat_interval_ms,
                            heartbeat_extend_ms: config.heartbeat_extend_ms,
                            worker_id: config.worker_id.clone(),
                            heartbeat_evidence_refs: config.heartbeat_evidence_refs.clone(),
                        },
                    },
                },
            )
            .map_err(|err| format!("supervised worker cycle failed: {err}"))?;

        executions += outcome.worker_loop.executions.len();
        recoveries += outcome.recoveries.len();
        stop_reason = format!("{:?}", outcome.worker_loop.stop_reason);
        receipts = outcome.projection.receipts;
        runnable_pending_actions = outcome.projection.runnable_pending_actions;
        open_execution_leases = outcome.projection.open_execution_leases;
        live_open_execution_leases = outcome.projection.live_open_execution_leases;
        expired_recoverable_execution_leases =
            outcome.projection.expired_recoverable_execution_leases;
        execution_reconciliations = outcome.projection.execution_reconciliations;

        let idle = outcome.recoveries.is_empty()
            && outcome.worker_loop.executions.is_empty()
            && matches!(
                outcome.worker_loop.stop_reason,
                RuntimeLocalShellWorkerLoopStopReason::NoRunnableAction
                    | RuntimeLocalShellWorkerLoopStopReason::NoMatchingRunnableAction
            );
        if idle {
            break;
        }
        let blocked = matches!(
            outcome.worker_loop.stop_reason,
            RuntimeLocalShellWorkerLoopStopReason::RecoveryBlocked
        );
        if blocked && config.idle_sleep_ms > 0 {
            thread::sleep(Duration::from_millis(config.idle_sleep_ms));
        }
    }

    Ok(SupervisorReport {
        command: "supervise-local-shell",
        session_id: config.session_id.clone(),
        tool: config.tool.clone(),
        tool_digest,
        cycles,
        executions,
        recoveries,
        stop_reason,
        receipts,
        runnable_pending_actions,
        open_execution_leases,
        live_open_execution_leases,
        expired_recoverable_execution_leases,
        execution_reconciliations,
    })
}

#[derive(Debug)]
struct SupervisorConfig {
    root: PathBuf,
    session_id: String,
    cwd: String,
    command: String,
    args: Vec<String>,
    tool: String,
    tool_version: Option<String>,
    tool_digest: Option<String>,
    side_effects: BTreeSet<SideEffectClass>,
    risk: RiskClass,
    timeout_secs: Option<u64>,
    max_output_bytes: Option<usize>,
    max_actions: usize,
    max_recoveries: usize,
    max_cycles: usize,
    idle_sleep_ms: u64,
    initial_lease_ms: Option<u64>,
    heartbeat_interval_ms: Option<u64>,
    heartbeat_extend_ms: Option<u64>,
    worker_id: Option<String>,
    heartbeat_evidence_refs: Vec<String>,
    reconciled_by: Option<String>,
    recovery_reason: String,
    recovery_evidence_refs: Vec<String>,
    json: bool,
}

impl SupervisorConfig {
    fn parse(args: &[String]) -> Result<Self, String> {
        if args.len() < 2 || args[1] == "--help" || args[1] == "-h" {
            return Err(USAGE.to_string());
        }
        if args[1] != "supervise-local-shell" {
            return Err(format!("{USAGE}unsupported command: {}", args[1]));
        }
        let mut root = None;
        let mut session_id = None;
        let mut cwd = None;
        let mut command = None;
        let mut command_args = Vec::new();
        let mut tool = "shell".to_string();
        let mut tool_version = None;
        let mut tool_digest = None;
        let mut side_effects = BTreeSet::new();
        let risk = RiskClass::Low;
        let mut timeout_secs = None;
        let mut max_output_bytes = None;
        let mut max_actions = DEFAULT_MAX_ACTIONS;
        let mut max_recoveries = DEFAULT_MAX_RECOVERIES;
        let mut max_cycles = DEFAULT_MAX_CYCLES;
        let mut idle_sleep_ms = DEFAULT_IDLE_SLEEP_MS;
        let mut initial_lease_ms = None;
        let mut heartbeat_interval_ms = None;
        let mut heartbeat_extend_ms = None;
        let mut worker_id = None;
        let mut heartbeat_evidence_refs = Vec::new();
        let mut reconciled_by = None;
        let mut recovery_reason =
            "runtime worker supervisor found an expired open execution lease".to_string();
        let mut recovery_evidence_refs = Vec::new();
        let mut json = false;

        let mut idx = 2;
        while idx < args.len() {
            match args[idx].as_str() {
                "--root" => root = Some(PathBuf::from(next_value(args, &mut idx, "--root")?)),
                "--session-id" => session_id = Some(next_value(args, &mut idx, "--session-id")?),
                "--cwd" => cwd = Some(next_value(args, &mut idx, "--cwd")?),
                "--command" => command = Some(next_value(args, &mut idx, "--command")?),
                "--arg" => command_args.push(next_value(args, &mut idx, "--arg")?),
                "--tool" => tool = next_value(args, &mut idx, "--tool")?,
                "--tool-version" => {
                    tool_version = Some(next_value(args, &mut idx, "--tool-version")?)
                }
                "--tool-digest" => tool_digest = Some(next_value(args, &mut idx, "--tool-digest")?),
                "--side-effect" => {
                    side_effects.insert(parse_side_effect(&next_value(
                        args,
                        &mut idx,
                        "--side-effect",
                    )?)?);
                }
                "--timeout-secs" => {
                    timeout_secs = Some(parse_u64(
                        &next_value(args, &mut idx, "--timeout-secs")?,
                        "--timeout-secs",
                    )?);
                }
                "--max-output-bytes" => {
                    max_output_bytes = Some(parse_usize(
                        &next_value(args, &mut idx, "--max-output-bytes")?,
                        "--max-output-bytes",
                    )?);
                }
                "--max-actions" => {
                    max_actions = parse_usize(
                        &next_value(args, &mut idx, "--max-actions")?,
                        "--max-actions",
                    )?;
                }
                "--max-recoveries" => {
                    max_recoveries = parse_usize(
                        &next_value(args, &mut idx, "--max-recoveries")?,
                        "--max-recoveries",
                    )?;
                }
                "--max-cycles" => {
                    max_cycles =
                        parse_usize(&next_value(args, &mut idx, "--max-cycles")?, "--max-cycles")?;
                }
                "--idle-sleep-ms" => {
                    idle_sleep_ms = parse_u64(
                        &next_value(args, &mut idx, "--idle-sleep-ms")?,
                        "--idle-sleep-ms",
                    )?;
                }
                "--initial-lease-ms" => {
                    initial_lease_ms = Some(parse_u64(
                        &next_value(args, &mut idx, "--initial-lease-ms")?,
                        "--initial-lease-ms",
                    )?);
                }
                "--heartbeat-interval-ms" => {
                    heartbeat_interval_ms = Some(parse_u64(
                        &next_value(args, &mut idx, "--heartbeat-interval-ms")?,
                        "--heartbeat-interval-ms",
                    )?);
                }
                "--heartbeat-extend-ms" => {
                    heartbeat_extend_ms = Some(parse_u64(
                        &next_value(args, &mut idx, "--heartbeat-extend-ms")?,
                        "--heartbeat-extend-ms",
                    )?);
                }
                "--worker-id" => worker_id = Some(next_value(args, &mut idx, "--worker-id")?),
                "--heartbeat-evidence-ref" => {
                    heartbeat_evidence_refs.push(next_value(
                        args,
                        &mut idx,
                        "--heartbeat-evidence-ref",
                    )?);
                }
                "--reconciled-by" => {
                    reconciled_by = Some(next_value(args, &mut idx, "--reconciled-by")?)
                }
                "--recovery-reason" => {
                    recovery_reason = next_value(args, &mut idx, "--recovery-reason")?;
                }
                "--recovery-evidence-ref" => {
                    recovery_evidence_refs.push(next_value(
                        args,
                        &mut idx,
                        "--recovery-evidence-ref",
                    )?);
                }
                "--json" => json = true,
                other => return Err(format!("{USAGE}unsupported option: {other}")),
            }
            idx += 1;
        }

        if side_effects.is_empty() {
            side_effects.insert(SideEffectClass::LocalWrite);
        }
        if recovery_reason.trim().is_empty() {
            return Err("recovery-reason must not be empty".to_string());
        }
        if reconciled_by
            .as_ref()
            .is_some_and(|actor| actor.trim().is_empty())
        {
            return Err("reconciled-by must not be empty".to_string());
        }
        if heartbeat_evidence_refs
            .iter()
            .chain(recovery_evidence_refs.iter())
            .any(|reference| reference.trim().is_empty())
        {
            return Err("evidence refs must not be empty".to_string());
        }

        Ok(Self {
            root: root.ok_or_else(|| "--root is required".to_string())?,
            session_id: session_id.ok_or_else(|| "--session-id is required".to_string())?,
            cwd: cwd.ok_or_else(|| "--cwd is required".to_string())?,
            command: command.ok_or_else(|| "--command is required".to_string())?,
            args: command_args,
            tool,
            tool_version,
            tool_digest,
            side_effects,
            risk,
            timeout_secs,
            max_output_bytes,
            max_actions,
            max_recoveries,
            max_cycles,
            idle_sleep_ms,
            initial_lease_ms,
            heartbeat_interval_ms,
            heartbeat_extend_ms,
            worker_id,
            heartbeat_evidence_refs,
            reconciled_by,
            recovery_reason,
            recovery_evidence_refs,
            json,
        })
    }
}

#[derive(Debug, Serialize)]
struct SupervisorReport {
    command: &'static str,
    session_id: String,
    tool: String,
    tool_digest: String,
    cycles: usize,
    executions: usize,
    recoveries: usize,
    stop_reason: String,
    receipts: usize,
    runnable_pending_actions: usize,
    open_execution_leases: usize,
    live_open_execution_leases: usize,
    expired_recoverable_execution_leases: usize,
    execution_reconciliations: usize,
}

impl SupervisorReport {
    fn ensure_complete(&self) -> Result<(), String> {
        if self.runnable_pending_actions == 0
            && self.open_execution_leases == 0
            && self.expired_recoverable_execution_leases == 0
            && self.stop_reason
                != format!("{:?}", RuntimeLocalShellWorkerLoopStopReason::MaxActions)
            && self.stop_reason
                != format!(
                    "{:?}",
                    RuntimeLocalShellWorkerLoopStopReason::RecoveryBlocked
                )
        {
            return Ok(());
        }
        Err(format!(
            "runtime worker supervisor stopped incomplete: stop={} runnable_pending_actions={} open_execution_leases={} live_open_execution_leases={} expired_recoverable_execution_leases={}",
            self.stop_reason,
            self.runnable_pending_actions,
            self.open_execution_leases,
            self.live_open_execution_leases,
            self.expired_recoverable_execution_leases
        ))
    }
}

fn next_value(args: &[String], idx: &mut usize, flag: &str) -> Result<String, String> {
    *idx += 1;
    args.get(*idx)
        .cloned()
        .ok_or_else(|| format!("{flag} requires a value"))
}

fn parse_u64(value: &str, flag: &str) -> Result<u64, String> {
    value
        .parse::<u64>()
        .map_err(|err| format!("{flag} must be an unsigned integer: {err}"))
}

fn parse_usize(value: &str, flag: &str) -> Result<usize, String> {
    value
        .parse::<usize>()
        .map_err(|err| format!("{flag} must be an unsigned integer: {err}"))
}

fn parse_side_effect(value: &str) -> Result<SideEffectClass, String> {
    match value {
        "local_write" => Ok(SideEffectClass::LocalWrite),
        other => Err(format!(
            "unsupported side effect {other:?}; supported: local_write"
        )),
    }
}
