//! End-to-end tests for `beaterosctl` that drive the library exactly as the
//! binary does, against a temporary store. These prove the MVP invariants from
//! `final.md` §24: scoped grants, policy outside the model, journal-before /
//! receipt-after, and a verifiable trace.

// Assertions in tests intentionally use expect/unwrap for concise failures.
#![allow(clippy::expect_used, clippy::unwrap_used)]

use std::path::PathBuf;

use beaterosctl::{CliError, run};
use uuid::Uuid;

/// A temporary store directory that cleans itself up.
struct TempHome {
    path: PathBuf,
}

impl TempHome {
    fn new() -> Self {
        let path = std::env::temp_dir().join(format!("beaterosctl-test-{}", Uuid::new_v4()));
        Self { path }
    }

    fn as_str(&self) -> String {
        self.path.display().to_string()
    }
}

impl Drop for TempHome {
    fn drop(&mut self) {
        let _ = std::fs::remove_dir_all(&self.path);
    }
}

/// Invoke the CLI with a home dir; returns Ok output or the error.
fn cli(home: &str, args: &[&str]) -> Result<String, CliError> {
    let mut argv = vec![
        "beaterosctl".to_string(),
        "--home".to_string(),
        home.to_string(),
    ];
    argv.extend(args.iter().map(|a| a.to_string()));
    run(argv.into_iter())
}

/// Convenience: run and unwrap, panicking with the error on failure.
fn ok(home: &str, args: &[&str]) -> String {
    match cli(home, args) {
        Ok(output) => output,
        Err(err) => panic!("expected success for {args:?}, got error: {err}"),
    }
}

#[test]
fn full_coding_workflow_end_to_end() {
    let home = TempHome::new();
    let h = home.as_str();
    let session = "sess-mvp";

    // 1. Create a session from a goal.
    ok(
        &h,
        &[
            "session",
            "create",
            "--session",
            session,
            "--agent",
            "agent-coder",
            "--workspace",
            "ws-repo",
            "--goal",
            "refactor the parser",
        ],
    );

    // 2. Issue a scoped file grant bounded to a path prefix.
    let grant_out = ok(
        &h,
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
            "read,write",
            "--path-prefix",
            "/workspace/repo",
            "--reason",
            "coding task",
        ],
    );
    let grant_id = grant_out
        .lines()
        .next()
        .and_then(|line| line.strip_prefix("issued grant "))
        .map(str::to_string)
        .expect("grant id in output");

    // 3. A write inside the granted prefix is ADMITTED.
    let allow = ok(
        &h,
        &[
            "action",
            "propose",
            "--session",
            session,
            "--tool",
            "fs.write",
            "--kind",
            "write",
            "--target-kind",
            "file_path",
            "--target",
            "/workspace/repo/src/main.rs",
            // Kernel-derived resolved target supplied by the mediation point.
            "--resolved-target",
            "/workspace/repo/src/main.rs",
            "--grants",
            &grant_id,
            "--action-id",
            "act-allow",
            "--summary",
            "edit main.rs",
        ],
    );
    assert!(
        allow.contains("Allowed"),
        "in-scope write should be allowed:\n{allow}"
    );

    // 4. A write OUTSIDE the prefix is refused by policy (not by the model).
    let denied = ok(
        &h,
        &[
            "action",
            "propose",
            "--session",
            session,
            "--tool",
            "fs.write",
            "--kind",
            "write",
            "--target-kind",
            "file_path",
            "--target",
            "/etc/passwd",
            // Even with a kernel-resolved target, a path OUTSIDE the granted
            // prefix must be refused by the path constraint (not by the model).
            "--resolved-target",
            "/etc/passwd",
            "--grants",
            &grant_id,
            "--action-id",
            "act-escape",
        ],
    );
    assert!(
        denied.contains("NeedsNarrowedGrant"),
        "out-of-scope write must not be allowed:\n{denied}"
    );

    // 5. A deploy against a file grant has no matching authority: no ambient power.
    let deploy = ok(
        &h,
        &[
            "action",
            "propose",
            "--session",
            session,
            "--tool",
            "deployer",
            "--kind",
            "deploy",
            "--target-kind",
            "cloud_resource",
            "--target",
            "prod",
            "--grants",
            &grant_id,
            "--action-id",
            "act-deploy",
            "--idempotency-key",
            "k1",
        ],
    );
    assert!(
        deploy.contains("NeedsNarrowedGrant"),
        "deploy without a deploy grant must not be allowed:\n{deploy}"
    );

    // 6. Record a receipt for the admitted action.
    let receipt = ok(
        &h,
        &[
            "receipt",
            "record",
            "--session",
            session,
            "--action",
            "act-allow",
            "--status",
            "ok",
        ],
    );
    assert!(receipt.contains("recorded receipt"), "{receipt}");

    // 7. The journal and receipt chains verify.
    let verify = ok(&h, &["journal", "verify", "--session", session]);
    assert!(verify.contains("journal OK"), "{verify}");

    // 8. The trace explains every action and its decision.
    let trace = ok(&h, &["trace", "show", "--session", session]);
    assert!(
        trace.contains("act-allow"),
        "trace missing allowed action:\n{trace}"
    );
    assert!(
        trace.contains("act-escape"),
        "trace missing denied action:\n{trace}"
    );
    assert!(
        trace.contains("refactor the parser"),
        "trace missing goal:\n{trace}"
    );
}

#[test]
fn receipt_is_refused_without_admitted_action() {
    let home = TempHome::new();
    let h = home.as_str();
    let session = "sess-refuse";

    ok(
        &h,
        &[
            "session",
            "create",
            "--session",
            session,
            "--agent",
            "a1",
            "--workspace",
            "w1",
            "--goal",
            "g",
        ],
    );
    let grant_out = ok(
        &h,
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
            "read",
            "--path-prefix",
            "/repo",
        ],
    );
    let grant_id = grant_out
        .lines()
        .next()
        .and_then(|line| line.strip_prefix("issued grant "))
        .map(str::to_string)
        .expect("grant id");

    // Propose a write that the read-only grant will not admit.
    let denied = ok(
        &h,
        &[
            "action",
            "propose",
            "--session",
            session,
            "--tool",
            "fs.write",
            "--kind",
            "write",
            "--target-kind",
            "file_path",
            "--target",
            "/repo/x",
            "--grants",
            &grant_id,
            "--action-id",
            "act-x",
        ],
    );
    assert!(denied.contains("NeedsNarrowedGrant"), "{denied}");

    // Recording a receipt for a non-admitted action must fail closed.
    let err = cli(
        &h,
        &[
            "receipt",
            "record",
            "--session",
            session,
            "--action",
            "act-x",
        ],
    )
    .expect_err("receipt for non-admitted action must be refused");
    assert!(
        matches!(err, CliError::Refused(_)),
        "unexpected error: {err}"
    );
}

#[test]
fn unknown_session_fails_closed() {
    let home = TempHome::new();
    let h = home.as_str();
    let err = cli(&h, &["trace", "show", "--session", "does-not-exist"])
        .expect_err("unknown session must error");
    assert!(
        matches!(err, CliError::SessionNotFound(_)),
        "unexpected: {err}"
    );
}

#[test]
fn help_is_available() {
    let home = TempHome::new();
    let out = ok(&home.as_str(), &["help"]);
    assert!(out.contains("beaterosctl"));
    assert!(out.contains("session create"));
}

/// Set up a session with a write grant and one admitted write action `act-ok`.
/// Returns the temp home so the caller keeps it alive.
fn setup_admitted_write(session: &str) -> TempHome {
    let home = TempHome::new();
    let h = home.as_str();
    ok(
        &h,
        &[
            "session",
            "create",
            "--session",
            session,
            "--agent",
            "a",
            "--workspace",
            "w",
            "--goal",
            "g",
        ],
    );
    let grant_out = ok(
        &h,
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
            "read,write",
            "--path-prefix",
            "/repo",
        ],
    );
    let grant_id = grant_out
        .lines()
        .next()
        .and_then(|line| line.strip_prefix("issued grant "))
        .map(str::to_string)
        .expect("grant id");
    ok(
        &h,
        &[
            "action",
            "propose",
            "--session",
            session,
            "--tool",
            "fs.write",
            "--kind",
            "write",
            "--target-kind",
            "file_path",
            "--target",
            "/repo/a",
            // `resolved_target` is kernel-derived (final.md §7.4): a mediation
            // point supplies the canonical, symlink-resolved path. The CLI no
            // longer infers it from the agent's claimed target, so a path-prefix
            // grant only admits when a resolved target is provided here.
            "--resolved-target",
            "/repo/a",
            "--grants",
            &grant_id,
            "--action-id",
            "act-ok",
        ],
    );
    home
}

#[test]
fn receipt_rejects_undeclared_side_effect_and_keeps_journal_verifiable() {
    let session = "sess-effects";
    let home = setup_admitted_write(session);
    let h = home.as_str();

    // The action declared only local_write; declaring `deployment` must be
    // refused at write time so the append-only journal stays verifiable.
    let err = cli(
        &h,
        &[
            "receipt",
            "record",
            "--session",
            session,
            "--action",
            "act-ok",
            "--side-effects",
            "deployment",
        ],
    )
    .expect_err("undeclared side effect must be refused");
    assert!(
        matches!(err, CliError::Refused(_)),
        "unexpected error: {err}"
    );

    // Nothing poisoned the log: verification still passes.
    let verify = ok(&h, &["journal", "verify", "--session", session]);
    assert!(verify.contains("journal OK"), "{verify}");

    // A subset of the declared effects is still allowed.
    let recorded = ok(
        &h,
        &[
            "receipt",
            "record",
            "--session",
            session,
            "--action",
            "act-ok",
            "--side-effects",
            "local_write",
        ],
    );
    assert!(recorded.contains("recorded receipt"), "{recorded}");
    assert!(ok(&h, &["journal", "verify", "--session", session]).contains("journal OK"));
}

#[test]
fn duplicate_action_id_is_refused_and_keeps_journal_verifiable() {
    let session = "sess-dup";
    let home = setup_admitted_write(session);
    let h = home.as_str();

    // Re-proposing the same action id must be refused (core forbids double
    // proposal, which would otherwise break verification permanently).
    let err = cli(
        &h,
        &[
            "action",
            "propose",
            "--session",
            session,
            "--tool",
            "fs.write",
            "--kind",
            "write",
            "--target-kind",
            "file_path",
            "--target",
            "/repo/a",
            "--grants",
            "ignored",
            "--action-id",
            "act-ok",
        ],
    )
    .expect_err("duplicate action id must be refused");
    assert!(
        matches!(err, CliError::Refused(_)),
        "unexpected error: {err}"
    );

    let verify = ok(&h, &["journal", "verify", "--session", session]);
    assert!(verify.contains("journal OK"), "{verify}");
}

/// A hand-edited journal must be rejected on load, so the admission path can
/// never operate on tampered state (fail closed — not only under `journal
/// verify`). Regression for the privilege-escalation-via-tamper finding.
#[test]
fn tampered_journal_is_refused_on_every_command() {
    let home = TempHome::new();
    let h = home.as_str();
    let session = "sess-tamper";
    ok(
        &h,
        &[
            "session",
            "create",
            "--session",
            session,
            "--agent",
            "a",
            "--workspace",
            "w",
            "--goal",
            "g",
        ],
    );
    let grant_out = ok(
        &h,
        &[
            "grant",
            "issue",
            "--session",
            session,
            "--resource-kind",
            "file_path",
            "--resource-id",
            "/tmp/f.txt",
            "--actions",
            "read",
        ],
    );
    let grant_id = grant_out
        .lines()
        .next()
        .and_then(|line| line.strip_prefix("issued grant "))
        .map(str::to_string)
        .expect("grant id");

    // Tamper: widen the grant from read to read+write directly in the journal,
    // without recomputing the hash chain.
    let journal = PathBuf::from(&h)
        .join("sessions")
        .join(session)
        .join("journal.jsonl");
    let original = std::fs::read_to_string(&journal).expect("read journal");
    let tampered = original.replacen("[\"read\"]", "[\"read\",\"write\"]", 1);
    assert_ne!(original, tampered, "tamper edit must change the journal");
    std::fs::write(&journal, tampered).expect("write tampered journal");

    // Every command that loads the journal must now fail closed.
    assert!(
        cli(&h, &["journal", "verify", "--session", session]).is_err(),
        "journal verify must reject a tampered chain"
    );
    assert!(
        cli(&h, &["trace", "show", "--session", session]).is_err(),
        "trace show must refuse to render tampered state"
    );
    let propose = cli(
        &h,
        &[
            "action",
            "propose",
            "--session",
            session,
            "--tool",
            "t",
            "--kind",
            "write",
            "--target-kind",
            "file_path",
            "--target",
            "/tmp/f.txt",
            "--resolved-target",
            "/tmp/f.txt",
            "--grants",
            &grant_id,
            "--action-id",
            "act-escalate",
            "--side-effects",
            "local_write",
        ],
    );
    assert!(
        propose.is_err(),
        "admission must fail closed against a tampered journal, got: {propose:?}"
    );
}

/// A `--session` id is attacker-controlled and used as a path segment, so an id
/// that escapes a single safe segment must be rejected (no path traversal).
#[test]
fn path_traversal_session_id_is_rejected() {
    let home = TempHome::new();
    let h = home.as_str();
    for bad in ["../../pwned", "a/b", "..", ".", "with space", ""] {
        let out = cli(
            &h,
            &[
                "session",
                "create",
                "--session",
                bad,
                "--agent",
                "a",
                "--workspace",
                "w",
                "--goal",
                "g",
            ],
        );
        assert!(out.is_err(), "unsafe session id {bad:?} must be rejected");
    }
    // Nothing was written outside the store root.
    let escaped = PathBuf::from(&h).join("../pwned");
    assert!(
        !escaped.exists(),
        "traversal must not create files outside root"
    );
}

/// A path-prefix grant must fail closed when no kernel-derived `resolved_target`
/// is supplied: the CLI (the agent surface) must not infer it from the agent's
/// own claimed target. Only an exact-resource grant admits without a resolved
/// target.
#[test]
fn path_prefix_grant_without_resolved_target_fails_closed() {
    let home = TempHome::new();
    let h = home.as_str();
    let session = "sess-noresolve";
    ok(
        &h,
        &[
            "session",
            "create",
            "--session",
            session,
            "--agent",
            "a",
            "--workspace",
            "w",
            "--goal",
            "g",
        ],
    );
    let grant_out = ok(
        &h,
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
            "read,write",
            "--path-prefix",
            "/ws",
        ],
    );
    let grant_id = grant_out
        .lines()
        .next()
        .and_then(|line| line.strip_prefix("issued grant "))
        .map(str::to_string)
        .expect("grant id");
    // No --resolved-target: the path constraint cannot be satisfied -> fail closed.
    let out = ok(
        &h,
        &[
            "action",
            "propose",
            "--session",
            session,
            "--tool",
            "t",
            "--kind",
            "write",
            "--target-kind",
            "file_path",
            "--target",
            "/ws/x.txt",
            "--grants",
            &grant_id,
            "--action-id",
            "act-noresolve",
            "--side-effects",
            "local_write",
        ],
    );
    assert!(
        out.contains("NeedsNarrowedGrant"),
        "path-prefix grant must fail closed without a resolved target:\n{out}"
    );
}
