//! `beateros-audit` — command-line surface for the independent audit crate.
//!
//! It reads a `beater-os-core` journal snapshot as JSON (from a file path or
//! `-` for stdin) and runs one subcommand:
//!
//! - `verify`  — run every independent audit check; exit non-zero on any failure.
//! - `show`    — print a human-legible trace timeline.
//! - `metrics` — print audit coverage metrics as JSON.
//! - `bundle`  — print a redaction-safe audit bundle as JSON.
//!
//! This is the offline reviewer tool from `final.md` §25 (trace viewer) and
//! §13.15 (export trace / incident timeline). It performs no network I/O.

use std::io::Read as _;
use std::process::ExitCode;

use beater_os_audit::{
    build_bundle, bundle_to_json, compute_metrics, render_trace, verify_snapshot,
};
use beater_os_core::JournalSnapshot;

const USAGE: &str = "\
beateros-audit — independent audit for a beaterOS journal snapshot

USAGE:
    beateros-audit <COMMAND> <SNAPSHOT>

COMMANDS:
    verify    Run every independent audit check (exit 1 if any fails)
    show      Print a human-legible trace timeline
    metrics   Print audit coverage metrics as JSON
    bundle    Print a redaction-safe audit bundle as JSON

SNAPSHOT:
    Path to a JSON journal snapshot, or - to read from stdin.
";

fn main() -> ExitCode {
    let args: Vec<String> = std::env::args().skip(1).collect();
    match run(&args) {
        Ok(code) => code,
        Err(message) => {
            eprintln!("error: {message}");
            ExitCode::from(2)
        }
    }
}

fn run(args: &[String]) -> Result<ExitCode, String> {
    let (command, source) = match args {
        [command, source] => (command.as_str(), source.as_str()),
        _ => {
            eprint!("{USAGE}");
            return Err("expected exactly two arguments: <command> <snapshot>".to_string());
        }
    };

    let raw = read_source(source)?;
    let snapshot: JournalSnapshot = serde_json::from_str(&raw)
        .map_err(|err| format!("could not parse journal snapshot as JSON: {err}"))?;

    match command {
        "verify" => {
            let report = verify_snapshot(&snapshot);
            for check in &report.checks {
                println!("[{:?}] {} — {}", check.outcome, check.check, check.detail);
            }
            if report.ok {
                println!("OK: {} record(s) passed all audit checks", report.records);
                Ok(ExitCode::SUCCESS)
            } else {
                println!("FAIL: {} check(s) failed", report.failures().count());
                Ok(ExitCode::FAILURE)
            }
        }
        "show" => {
            print!("{}", render_trace(&snapshot));
            Ok(ExitCode::SUCCESS)
        }
        "metrics" => {
            let metrics = compute_metrics(&snapshot);
            let json = serde_json::to_string_pretty(&metrics)
                .map_err(|err| format!("could not serialize metrics: {err}"))?;
            println!("{json}");
            Ok(ExitCode::SUCCESS)
        }
        "bundle" => {
            let bundle = build_bundle(&snapshot);
            let json = bundle_to_json(&bundle)
                .map_err(|err| format!("could not serialize bundle: {err}"))?;
            println!("{json}");
            Ok(ExitCode::SUCCESS)
        }
        other => {
            eprint!("{USAGE}");
            Err(format!("unknown command: {other}"))
        }
    }
}

fn read_source(source: &str) -> Result<String, String> {
    if source == "-" {
        let mut buffer = String::new();
        std::io::stdin()
            .read_to_string(&mut buffer)
            .map_err(|err| format!("could not read snapshot from stdin: {err}"))?;
        Ok(buffer)
    } else {
        std::fs::read_to_string(source)
            .map_err(|err| format!("could not read snapshot file {source}: {err}"))
    }
}
