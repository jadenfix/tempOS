//! `beateros-audit` — command-line surface for the independent audit crate.
//!
//! It reads a `beater-os-core` journal snapshot as JSON (from a file path or
//! `-` for stdin) and runs one subcommand:
//!
//! - `verify`  — run every independent audit check; exit non-zero on any failure.
//! - `show`    — print a human-legible trace timeline.
//! - `metrics` — print audit coverage metrics as JSON.
//! - `bundle`  — print a redaction-safe audit bundle as JSON.
//! - `verify-trace` — verify a full trace bundle exported by `beaterosctl`.
//!
//! This is the offline reviewer tool from `final.md` §25 (trace viewer) and
//! §13.15 (export trace / incident timeline). It performs no network I/O.

use std::io::Read as _;
use std::process::ExitCode;

use beater_os_audit::{
    CheckOutcome, CheckResult, TraceBundle, TraceBundleVerifyOptions, build_bundle, bundle_to_json,
    compute_metrics, render_trace, verify_expected_root, verify_snapshot,
    verify_trace_bundle_with_options,
};
use beater_os_core::JournalSnapshot;

const USAGE: &str = "\
beateros-audit — independent audit for a beaterOS journal snapshot

USAGE:
    beateros-audit <COMMAND> <SNAPSHOT>
    beateros-audit verify [--expected-root <HASH>] <SNAPSHOT>
    beateros-audit verify-trace [--expected-root <HASH>] <TRACE_BUNDLE>

COMMANDS:
    verify        Run every independent audit check (exit 1 if any fails)
    verify-trace  Verify a full trace bundle exported by beaterosctl trace export
    show          Print a human-legible trace timeline
    metrics       Print audit coverage metrics as JSON
    bundle        Print a redaction-safe audit bundle as JSON

SNAPSHOT:
    Path to a JSON journal snapshot or trace bundle, or - to read from stdin.

OPTIONS:
    --expected-root <HASH>
        For verify and verify-trace only: require the journal root hash to match
        an externally trusted anchor. This detects truncation or coherent
        re-hashing relative to that anchor.
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
    let (command, expected_root, source) = parse_args(args)?;

    // Validate the command before touching input, so an unknown command reports
    // itself rather than a downstream file/parse error.
    if !matches!(
        command,
        "verify" | "verify-trace" | "show" | "metrics" | "bundle"
    ) {
        eprint!("{USAGE}");
        return Err(format!("unknown command: {command}"));
    }

    let raw = read_source(source)?;
    if command == "verify-trace" {
        let bundle: TraceBundle = serde_json::from_str(&raw)
            .map_err(|err| format!("could not parse trace bundle as JSON: {err}"))?;
        let report = verify_trace_bundle_with_options(
            &bundle,
            TraceBundleVerifyOptions {
                expected_journal_root: expected_root,
            },
        );
        for check in &report.checks {
            println!("[{:?}] {} — {}", check.outcome, check.check, check.detail);
        }
        if report.records == 0 {
            println!("note: trace bundle journal is empty — this attests nothing about a real run");
        }
        if report.ok {
            println!(
                "OK: trace bundle {} verified over {} record(s), journal root {}",
                report.bundle_id, report.records, report.journal_root_hash
            );
            return Ok(ExitCode::SUCCESS);
        }
        let failures = report
            .checks
            .iter()
            .filter(|check| !check_passed(check))
            .count();
        println!("FAIL: {failures} trace bundle check(s) failed");
        return Ok(ExitCode::FAILURE);
    }
    let snapshot: JournalSnapshot = serde_json::from_str(&raw)
        .map_err(|err| format!("could not parse journal snapshot as JSON: {err}"))?;

    match command {
        "verify" => {
            let report = verify_snapshot(&snapshot);
            for check in &report.checks {
                println!("[{:?}] {} — {}", check.outcome, check.check, check.detail);
            }
            let root_check =
                expected_root.map(|expected| verify_expected_root(&snapshot, expected));
            if let Some(check) = &root_check {
                println!("[{:?}] {} — {}", check.outcome, check.check, check.detail);
            }
            if report.records == 0 {
                println!("note: journal is empty — this attests nothing about a real run");
            }
            if report.ok && root_check.as_ref().is_none_or(check_passed) {
                println!("OK: {} record(s) passed all audit checks", report.records);
                Ok(ExitCode::SUCCESS)
            } else {
                let failures = report.failures().count()
                    + root_check
                        .as_ref()
                        .filter(|check| !check_passed(check))
                        .map(|_| 1)
                        .unwrap_or(0);
                println!("FAIL: {failures} check(s) failed");
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

fn parse_args(args: &[String]) -> Result<(&str, Option<&str>, &str), String> {
    match args {
        [command, source] => Ok((command.as_str(), None, source.as_str())),
        [command, flag, expected_root, source]
            if matches!(command.as_str(), "verify" | "verify-trace")
                && flag == "--expected-root" =>
        {
            Ok((
                command.as_str(),
                Some(expected_root.as_str()),
                source.as_str(),
            ))
        }
        [command, flag, ..]
            if flag == "--expected-root"
                && !matches!(command.as_str(), "verify" | "verify-trace") =>
        {
            eprint!("{USAGE}");
            Err("--expected-root is only valid with verify or verify-trace".to_string())
        }
        _ => {
            eprint!("{USAGE}");
            Err(
                "expected <command> <snapshot> or verify[/-trace] --expected-root <hash> <input>"
                    .to_string(),
            )
        }
    }
}

fn check_passed(check: &CheckResult) -> bool {
    check.outcome == CheckOutcome::Pass
}

/// Maximum accepted snapshot size. The audit tool fully materializes the
/// snapshot in memory (`serde_json::from_str` over a `Vec<JournalRecord>`), so
/// an unbounded read is an unbounded allocation from untrusted input. This
/// offline reviewer never needs a journal larger than this; above it we fail
/// closed (AGENTS.md: "bounded by construction — memory").
const MAX_SNAPSHOT_BYTES: u64 = 64 * 1024 * 1024; // 64 MiB

fn read_source(source: &str) -> Result<String, String> {
    if source == "-" {
        read_capped(std::io::stdin().lock(), MAX_SNAPSHOT_BYTES, "stdin")
    } else {
        let file = std::fs::File::open(source)
            .map_err(|err| format!("could not open snapshot file {source}: {err}"))?;
        read_capped(file, MAX_SNAPSHOT_BYTES, source)
    }
}

/// Read at most `cap` bytes from `reader`, failing closed if the input exceeds
/// the cap rather than allocating without bound. `label` names the source for
/// error messages.
fn read_capped<R: std::io::Read>(reader: R, cap: u64, label: &str) -> Result<String, String> {
    let mut buffer = Vec::new();
    // take(cap + 1): reading cap+1 bytes means the input is over the limit, and
    // we never buffer more than one byte past the cap.
    reader
        .take(cap.saturating_add(1))
        .read_to_end(&mut buffer)
        .map_err(|err| format!("could not read snapshot from {label}: {err}"))?;
    if buffer.len() as u64 > cap {
        return Err(format!(
            "snapshot from {label} exceeds the {cap}-byte audit input cap; refusing to read further"
        ));
    }
    String::from_utf8(buffer)
        .map_err(|err| format!("snapshot from {label} is not valid UTF-8: {err}"))
}

#[cfg(test)]
mod tests {
    use super::read_capped;
    use std::io::Cursor;

    #[test]
    fn reads_input_under_the_cap() {
        let out = read_capped(Cursor::new(b"{\"records\":[]}".to_vec()), 64, "test");
        assert_eq!(out.as_deref(), Ok("{\"records\":[]}"));
    }

    #[test]
    fn accepts_input_exactly_at_the_cap() {
        let out = read_capped(Cursor::new(vec![b'x'; 8]), 8, "test");
        assert_eq!(out.as_deref(), Ok("xxxxxxxx"));
    }

    #[test]
    fn rejects_input_one_byte_over_the_cap() {
        let out = read_capped(Cursor::new(vec![b'x'; 9]), 8, "test");
        assert!(out.is_err(), "expected the over-cap read to fail closed");
        if let Err(msg) = out {
            assert!(msg.contains("exceeds the 8-byte audit input cap"));
        }
    }

    #[test]
    fn rejects_non_utf8_input() {
        let out = read_capped(Cursor::new(vec![0xff, 0xfe]), 64, "test");
        assert!(out.is_err());
    }
}
