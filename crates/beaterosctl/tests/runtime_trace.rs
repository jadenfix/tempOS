//! Runtime trace-property tests for the real `beaterosctl` execution path.
//!
//! These are a small stepping stone for issue #90: they do not yet load the
//! scenario corpus, but they assert the properties that the future scenario
//! runner must prove after driving a real session/grant/action/receipt loop.

// Assertions in tests intentionally use expect/unwrap for concise failures.
#![allow(clippy::expect_used, clippy::unwrap_used)]

use std::fs;
use std::path::PathBuf;

use beater_os_core::{
    CapabilityReceipt, DecisionResult, JournalEvent, JournalRecord, SideEffectClass,
};
use beaterosctl::{CliError, run};
use uuid::Uuid;

struct TempDir {
    path: PathBuf,
}

impl TempDir {
    fn new(tag: &str) -> Self {
        let path =
            std::env::temp_dir().join(format!("beaterosctl-runtime-{tag}-{}", Uuid::new_v4()));
        fs::create_dir_all(&path).unwrap();
        Self { path }
    }

    fn canonical(&self) -> String {
        fs::canonicalize(&self.path).unwrap().display().to_string()
    }
}

impl Drop for TempDir {
    fn drop(&mut self) {
        let _ = fs::remove_dir_all(&self.path);
    }
}

fn cli(home: &str, args: &[&str]) -> Result<String, CliError> {
    let mut argv = vec![
        "beaterosctl".to_string(),
        "--home".to_string(),
        home.to_string(),
    ];
    argv.extend(args.iter().map(|a| a.to_string()));
    run(argv.into_iter())
}

fn ok(home: &str, args: &[&str]) -> String {
    match cli(home, args) {
        Ok(output) => output,
        Err(err) => panic!("expected success for {args:?}, got error: {err}"),
    }
}

fn create_session(home: &str, session: &str) {
    ok(
        home,
        &[
            "session",
            "create",
            "--session",
            session,
            "--agent",
            "runner",
            "--workspace",
            "runtime-trace",
            "--goal",
            "prove runtime trace properties",
        ],
    );
}

fn issue_file_grant(home: &str, session: &str, actions: &str, prefix: &str) -> String {
    let out = ok(
        home,
        &[
            "grant",
            "issue",
            "--session",
            session,
            "--resource-kind",
            "file_path",
            "--resource-id",
            "*",
            "--actions",
            actions,
            "--path-prefix",
            prefix,
            "--max-risk",
            "high",
        ],
    );
    out.lines()
        .next()
        .and_then(|line| line.strip_prefix("issued grant "))
        .map(str::to_string)
        .expect("grant id in output")
}

fn journal_records(home: &str, session: &str) -> Vec<JournalRecord> {
    let journal = PathBuf::from(home)
        .join("sessions")
        .join(session)
        .join("journal.jsonl");
    fs::read_to_string(&journal)
        .unwrap()
        .lines()
        .map(|line| serde_json::from_str::<JournalRecord>(line).unwrap())
        .collect()
}

fn action_proposed_seq(records: &[JournalRecord], action_id: &str) -> u64 {
    records
        .iter()
        .find_map(|record| match &record.event {
            JournalEvent::ActionProposed { manifest } if manifest.action_id == action_id => {
                Some(record.seq)
            }
            _ => None,
        })
        .expect("action proposed event")
}

fn policy_decided_seq(records: &[JournalRecord], action_id: &str) -> u64 {
    records
        .iter()
        .find_map(|record| match &record.event {
            JournalEvent::PolicyDecided { decision } if decision.action_id == action_id => {
                Some(record.seq)
            }
            _ => None,
        })
        .expect("policy decided event")
}

fn decision_result(records: &[JournalRecord], action_id: &str) -> Option<DecisionResult> {
    records.iter().find_map(|record| match &record.event {
        JournalEvent::PolicyDecided { decision } if decision.action_id == action_id => {
            Some(decision.result.clone())
        }
        _ => None,
    })
}

fn receipt_events<'a>(
    records: &'a [JournalRecord],
    action_id: &str,
) -> Vec<(u64, &'a CapabilityReceipt)> {
    records
        .iter()
        .filter_map(|record| match &record.event {
            JournalEvent::ReceiptAppended { receipt } if receipt.action_id == action_id => {
                Some((record.seq, receipt))
            }
            _ => None,
        })
        .collect()
}

#[test]
fn allowed_execute_emits_action_decision_receipt_and_trace_evidence() {
    let home = TempDir::new("home");
    let work = TempDir::new("work");
    let h = home.canonical();
    let workdir = work.canonical();
    let session = "sess-runtime-trace-allowed";

    create_session(&h, session);
    let grant_id = issue_file_grant(&h, session, "execute", &workdir);
    let created = PathBuf::from(&workdir).join("runtime-created.txt");

    let out = ok(
        &h,
        &[
            "action",
            "execute",
            "--session",
            session,
            "--tool",
            "shell",
            "--command",
            "sh",
            "--arg",
            "-c",
            "--arg",
            "printf trace-proof > runtime-created.txt",
            "--cwd",
            &workdir,
            "--grants",
            &grant_id,
            "--side-effects",
            "local_write",
            "--action-id",
            "act-runtime-allowed",
        ],
    );

    assert!(out.contains("decision:    Allowed"), "{out}");
    assert!(out.contains("execution:   ok"), "{out}");
    assert!(
        out.contains(&format!("created=[\"{}\"]", created.display())),
        "stdout must expose observed fs-diff:\n{out}"
    );
    assert_eq!(fs::read_to_string(&created).unwrap(), "trace-proof");

    let records = journal_records(&h, session);
    let proposed_seq = action_proposed_seq(&records, "act-runtime-allowed");
    let decision_seq = policy_decided_seq(&records, "act-runtime-allowed");
    assert_eq!(
        decision_result(&records, "act-runtime-allowed"),
        Some(DecisionResult::Allowed)
    );
    let receipts = receipt_events(&records, "act-runtime-allowed");
    assert_eq!(
        receipts.len(),
        1,
        "allowed action must emit exactly one receipt"
    );
    let (receipt_seq, receipt) = receipts[0];
    assert!(
        proposed_seq < decision_seq && decision_seq < receipt_seq,
        "allowed action journal order must be proposal < decision < receipt"
    );
    assert_eq!(receipt.status, "ok");
    assert_eq!(receipt.tool_id, "shell");
    assert_eq!(receipt.side_effects, vec![SideEffectClass::LocalWrite]);
    assert_eq!(receipt.artifact_refs, vec![created.display().to_string()]);
    assert!(
        receipt.side_effect_summary.contains("runtime-created.txt"),
        "receipt must preserve observed file effect: {}",
        receipt.side_effect_summary
    );

    let verify = ok(&h, &["journal", "verify", "--session", session]);
    assert!(verify.contains("journal OK"), "{verify}");
    assert!(verify.contains("receipts:      1"), "{verify}");

    let trace = ok(&h, &["trace", "show", "--session", session]);
    assert!(trace.contains("act-runtime-allowed"), "{trace}");
    assert!(trace.contains("decision: Allowed"), "{trace}");
    assert!(trace.contains("receipt:"), "{trace}");
    assert!(trace.contains("effects=[LocalWrite]"), "{trace}");
}

#[test]
fn denied_execute_emits_decision_but_no_receipt_or_side_effect() {
    let home = TempDir::new("home");
    let work = TempDir::new("work");
    let h = home.canonical();
    let workdir = work.canonical();
    let session = "sess-runtime-trace-denied";

    create_session(&h, session);
    let grant_id = issue_file_grant(&h, session, "read", &workdir);
    let forbidden = PathBuf::from(&workdir).join("should-not-exist.txt");
    let forbidden_arg = format!("printf denied > {}", forbidden.display());

    let out = ok(
        &h,
        &[
            "action",
            "execute",
            "--session",
            session,
            "--tool",
            "shell",
            "--command",
            "sh",
            "--arg",
            "-c",
            "--arg",
            &forbidden_arg,
            "--cwd",
            &workdir,
            "--grants",
            &grant_id,
            "--side-effects",
            "local_write",
            "--action-id",
            "act-runtime-denied",
        ],
    );

    assert!(out.contains("decision:    NeedsNarrowedGrant"), "{out}");
    assert!(out.contains("execution:   skipped"), "{out}");
    assert!(
        !forbidden.exists(),
        "denied action must not create {}",
        forbidden.display()
    );

    let records = journal_records(&h, session);
    let proposed_seq = action_proposed_seq(&records, "act-runtime-denied");
    let decision_seq = policy_decided_seq(&records, "act-runtime-denied");
    assert!(
        proposed_seq < decision_seq,
        "denied action journal order must be proposal < decision"
    );
    assert_eq!(
        decision_result(&records, "act-runtime-denied"),
        Some(DecisionResult::NeedsNarrowedGrant)
    );
    assert_eq!(receipt_events(&records, "act-runtime-denied").len(), 0);

    let verify = ok(&h, &["journal", "verify", "--session", session]);
    assert!(verify.contains("journal OK"), "{verify}");
    assert!(verify.contains("receipts:      0"), "{verify}");

    let trace = ok(&h, &["trace", "show", "--session", session]);
    assert!(trace.contains("act-runtime-denied"), "{trace}");
    assert!(trace.contains("decision: NeedsNarrowedGrant"), "{trace}");
    assert!(!trace.contains("receipt:"), "{trace}");
}
