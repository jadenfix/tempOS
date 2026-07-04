//! End-to-end tests for the `action execute` sandbox lane (final.md §8, §13.8,
//! backlog slice 5). These drive the CLI exactly as the binary does and prove
//! the security boundary: real confined execution, canonicalized symlink-escape
//! rejection, a scrubbed environment (no inherited secrets), and fail-closed
//! behavior when an action is not admitted — all while preserving the
//! hash-chained journal/receipt invariants.

// Assertions in tests intentionally use expect/unwrap for concise failures.
#![allow(clippy::expect_used, clippy::unwrap_used)]

use std::fs;
use std::path::PathBuf;

use beaterosctl::{CliError, run};
use uuid::Uuid;

/// A temporary directory (store home or confined workdir) that cleans itself up.
struct TempDir {
    path: PathBuf,
}

impl TempDir {
    fn new(tag: &str) -> Self {
        let path = std::env::temp_dir().join(format!("beaterosctl-{tag}-{}", Uuid::new_v4()));
        fs::create_dir_all(&path).unwrap();
        Self { path }
    }

    /// Canonical path string (robust to macOS `/var` -> `/private/var`).
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
            "coder",
            "--workspace",
            "ws",
            "--goal",
            "run a scoped shell action",
        ],
    );
}

/// Issue a grant and return its id.
fn issue_grant(home: &str, session: &str, extra: &[&str]) -> String {
    let mut args = vec![
        "grant",
        "issue",
        "--session",
        session,
        "--resource-kind",
        "file_path",
        "--resource-id",
        "*",
    ];
    args.extend_from_slice(extra);
    let out = ok(home, &args);
    out.lines()
        .next()
        .and_then(|line| line.strip_prefix("issued grant "))
        .map(str::to_string)
        .expect("grant id in output")
}

#[test]
fn execute_runs_for_real_and_records_a_filesystem_diff() {
    let home = TempDir::new("home");
    let work = TempDir::new("work");
    let h = home.canonical();
    let workdir = work.canonical();
    let session = "sess-exec";

    create_session(&h, session);
    // Execute grant confined to the canonical work directory via path prefix.
    let grant_id = issue_grant(
        &h,
        session,
        &["--actions", "execute", "--path-prefix", &workdir],
    );

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
            "printf hello > created.txt",
            "--cwd",
            &workdir,
            "--grants",
            &grant_id,
            "--side-effects",
            "local_write",
            "--action-id",
            "act-exec",
        ],
    );

    assert!(out.contains("Allowed"), "action must be admitted:\n{out}");
    assert!(out.contains("execution:   ok"), "must execute:\n{out}");
    assert!(
        out.contains("created=[\"created.txt\"]"),
        "receipt must record the created file:\n{out}"
    );
    // The file really exists on disk.
    let created = PathBuf::from(&workdir).join("created.txt");
    assert!(created.is_file(), "command must really create the file");
    assert_eq!(fs::read_to_string(&created).unwrap(), "hello");

    // Journal + receipt chains verify.
    let verify = ok(&h, &["journal", "verify", "--session", session]);
    assert!(verify.contains("journal OK"), "{verify}");
    assert!(verify.contains("receipts:      1"), "{verify}");
}

#[test]
fn symlink_escape_is_rejected_and_nothing_executes() {
    let home = TempDir::new("home");
    let granted = TempDir::new("granted");
    let outside = TempDir::new("outside");
    let h = home.canonical();
    let granted_dir = granted.canonical();
    let session = "sess-escape";

    create_session(&h, session);
    let grant_id = issue_grant(
        &h,
        session,
        &["--actions", "execute", "--path-prefix", &granted_dir],
    );

    // A symlink inside the granted dir that points outside it.
    let link = PathBuf::from(&granted_dir).join("escape");
    std::os::unix::fs::symlink(fs::canonicalize(&outside.path).unwrap(), &link).unwrap();

    let result = cli(
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
            "touch pwned",
            // cwd is the symlink, whose realpath escapes the granted prefix.
            "--cwd",
            &link.display().to_string(),
            "--grants",
            &grant_id,
            "--action-id",
            "act-escape",
        ],
    );
    assert!(
        matches!(result, Err(CliError::Sandbox(_))),
        "symlink escape must fail closed: {result:?}"
    );
    // Nothing executed in the escape target.
    assert!(!PathBuf::from(&outside.path).join("pwned").exists());
    // Nothing was even journaled: the mediation point aborted before proposal.
    let show = ok(&h, &["session", "show", "--session", session]);
    assert!(
        show.contains("actions:    0"),
        "no action journaled:\n{show}"
    );
}

#[test]
fn environment_is_scrubbed_no_inherited_secrets() {
    let home = TempDir::new("home");
    let work = TempDir::new("work");
    let h = home.canonical();
    let workdir = work.canonical();
    let session = "sess-secret";

    // Prove against a var cargo injects into the test process (`env_clear`
    // cannot be exercised without the forbidden `unsafe set_var`). It exists in
    // the parent but must not leak to the child.
    let secret = std::env::var("CARGO_PKG_NAME").expect("cargo sets CARGO_PKG_NAME");
    assert!(!secret.is_empty());

    create_session(&h, session);
    let grant_id = issue_grant(
        &h,
        session,
        &["--actions", "execute", "--path-prefix", &workdir],
    );

    ok(
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
            "echo pkg=$CARGO_PKG_NAME > leak.txt",
            "--cwd",
            &workdir,
            "--grants",
            &grant_id,
            "--side-effects",
            "local_write",
            "--action-id",
            "act-secret",
        ],
    );

    let leaked = fs::read_to_string(PathBuf::from(&workdir).join("leak.txt")).unwrap();
    assert!(
        !leaked.contains(&secret),
        "env_clear must scrub inherited env, but child wrote {secret:?}: {leaked:?}"
    );
    assert!(
        leaked.contains("pkg="),
        "command should still run: {leaked:?}"
    );
}

#[test]
fn not_admitted_action_does_not_execute_and_leaves_no_receipt() {
    let home = TempDir::new("home");
    let work = TempDir::new("work");
    let h = home.canonical();
    let workdir = work.canonical();
    let session = "sess-denied";

    create_session(&h, session);
    // Grant READ only (no Execute) on the work dir: the execute action has no
    // covering authority.
    let grant_id = issue_grant(
        &h,
        session,
        &["--actions", "read", "--path-prefix", &workdir],
    );

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
            "touch should_not_exist.txt",
            "--cwd",
            &workdir,
            "--grants",
            &grant_id,
            "--action-id",
            "act-denied",
        ],
    );

    assert!(
        out.contains("NeedsNarrowedGrant"),
        "execute without an Execute grant must not be admitted:\n{out}"
    );
    assert!(out.contains("skipped"), "execution must be skipped:\n{out}");
    // No command ran: no file, no receipt.
    assert!(
        !PathBuf::from(&workdir)
            .join("should_not_exist.txt")
            .exists()
    );
    let show = ok(&h, &["session", "show", "--session", session]);
    assert!(show.contains("receipts:   0"), "no receipt:\n{show}");
    let verify = ok(&h, &["journal", "verify", "--session", session]);
    assert!(verify.contains("journal OK"), "{verify}");
}
