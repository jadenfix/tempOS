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
    // The file really exists on disk.
    let created = PathBuf::from(&workdir).join("created.txt");
    assert!(created.is_file(), "command must really create the file");
    assert_eq!(fs::read_to_string(&created).unwrap(), "hello");
    // The fs-diff records the created file by its absolute (observed) path.
    assert!(
        out.contains(&format!("created=[\"{}\"]", created.display())),
        "receipt must record the created file (observed):\n{out}"
    );

    // Journal + receipt chains verify.
    let verify = ok(&h, &["journal", "verify", "--session", session]);
    assert!(verify.contains("journal OK"), "{verify}");
    assert!(verify.contains("receipts:      1"), "{verify}");
}

#[test]
fn symlinked_grant_prefix_and_cwd_are_compared_in_canonical_namespace() {
    let home = TempDir::new("home");
    let work = TempDir::new("work");
    let alias_parent = TempDir::new("alias-parent");
    let h = home.canonical();
    let workdir = work.canonical();
    let alias = alias_parent.path.join("work-alias");
    std::os::unix::fs::symlink(&work.path, &alias).unwrap();
    let alias_dir = alias.display().to_string();
    let session = "sess-symlink-prefix";

    create_session(&h, session);
    let grant_id = issue_grant(
        &h,
        session,
        &["--actions", "execute", "--path-prefix", &alias_dir],
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
            "printf ok > via_alias.txt",
            "--cwd",
            &alias_dir,
            "--grants",
            &grant_id,
            "--side-effects",
            "local_write",
            "--action-id",
            "act-symlink-prefix",
        ],
    );

    assert!(out.contains("Allowed"), "action must be admitted:\n{out}");
    assert!(
        out.contains(&format!("resolved:    {workdir}")),
        "resolved target must be canonical:\n{out}"
    );
    let created = PathBuf::from(&workdir).join("via_alias.txt");
    assert!(created.is_file(), "command must write inside real workdir");
    assert_eq!(fs::read_to_string(&created).unwrap(), "ok");
    let verify = ok(&h, &["journal", "verify", "--session", session]);
    assert!(verify.contains("journal OK"), "{verify}");
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

/// Drive an `action execute` and return the CLI output.
fn execute_script(
    h: &str,
    session: &str,
    grant_id: &str,
    workdir: &str,
    action_id: &str,
    script: &str,
) -> String {
    ok(
        h,
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
            script,
            "--cwd",
            workdir,
            "--grants",
            grant_id,
            "--action-id",
            action_id,
        ],
    )
}

/// Regression for the demonstrated escape (exploit 1): an absolute-path write
/// OUTSIDE the granted prefix. cwd anchoring admitted it as a clean success with
/// an empty receipt; real OS confinement must DENY it — no file outside, and the
/// receipt must surface a non-zero child status (not a misleading clean "ok").
#[test]
fn exploit_absolute_write_outside_prefix_is_denied() {
    let home = TempDir::new("home");
    let work = TempDir::new("work");
    let outside = TempDir::new("outside");
    let h = home.canonical();
    let workdir = work.canonical();
    let outside_dir = outside.canonical();
    let session = "sess-abs";

    create_session(&h, session);
    let grant_id = issue_grant(
        &h,
        session,
        &["--actions", "execute", "--path-prefix", &workdir],
    );

    let escaped = PathBuf::from(&outside_dir).join("escaped_abs.txt");
    let out = execute_script(
        &h,
        session,
        &grant_id,
        &workdir,
        "act-abs",
        &format!("printf PWNED > {}", escaped.display()),
    );

    assert!(
        !escaped.exists(),
        "sandbox must deny the out-of-prefix write; no file may appear outside:\n{out}"
    );
    assert!(
        !out.contains("execution:   ok"),
        "a denied write must NOT be receipted as a clean success:\n{out}"
    );
    assert!(
        out.contains("execution:   failed"),
        "the non-zero child status must be surfaced:\n{out}"
    );
    let verify = ok(&h, &["journal", "verify", "--session", session]);
    assert!(verify.contains("journal OK"), "{verify}");
}

/// Regression for exploit 2: a `../` write to the PARENT of the granted prefix.
#[test]
fn exploit_dotdot_write_to_parent_is_denied() {
    let home = TempDir::new("home");
    let work = TempDir::new("work");
    let h = home.canonical();
    let workdir = work.canonical();
    let session = "sess-dd";

    create_session(&h, session);
    let grant_id = issue_grant(
        &h,
        session,
        &["--actions", "execute", "--path-prefix", &workdir],
    );

    let parent = PathBuf::from(&workdir)
        .parent()
        .expect("workdir has a parent")
        .to_path_buf();
    let escaped = parent.join("dotdot_escape.txt");
    let out = execute_script(
        &h,
        session,
        &grant_id,
        &workdir,
        "act-dd",
        &format!("printf x > {}", escaped.display()),
    );

    assert!(
        !escaped.exists(),
        "sandbox must deny the ../ write to the prefix parent:\n{out}"
    );
    assert!(
        !out.contains("execution:   ok"),
        "must not be a clean success:\n{out}"
    );
}

/// Regression for exploit 3: reading a secret file entirely OUTSIDE the prefix.
/// The read must be denied — no secret bytes in the output digest, non-zero
/// child status.
#[test]
fn exploit_read_secret_outside_prefix_is_denied() {
    let home = TempDir::new("home");
    let work = TempDir::new("work");
    let outside = TempDir::new("outside");
    let h = home.canonical();
    let workdir = work.canonical();
    let outside_dir = outside.canonical();
    let session = "sess-read";

    let secret = PathBuf::from(&outside_dir).join("secret.txt");
    fs::write(&secret, b"TOP-SECRET-XYZZY").unwrap();

    create_session(&h, session);
    let grant_id = issue_grant(
        &h,
        session,
        &["--actions", "execute", "--path-prefix", &workdir],
    );

    // Copy the secret INTO the workdir if the read succeeded, so a leak is
    // observable on disk regardless of stdout capture.
    let out = execute_script(
        &h,
        session,
        &grant_id,
        &workdir,
        "act-read",
        &format!("cat {} > leak.txt", secret.display()),
    );

    let leak = PathBuf::from(&workdir).join("leak.txt");
    let leaked = fs::read_to_string(&leak).unwrap_or_default();
    assert!(
        !leaked.contains("TOP-SECRET-XYZZY"),
        "secret content must not leak past the sandbox: {leaked:?}\n{out}"
    );
    assert!(
        !out.contains("execution:   ok"),
        "a denied read must surface a non-zero child status:\n{out}"
    );
}

/// A legitimate in-prefix write still succeeds and the receipt truthfully
/// records the OBSERVED effect (the fs-diff + certified LocalWrite), while a
/// no-op command certifies NO effect even if the agent declared one.
#[test]
fn legitimate_write_is_observed_and_noop_certifies_nothing() {
    let home = TempDir::new("home");
    let work = TempDir::new("work");
    let h = home.canonical();
    let workdir = work.canonical();
    let session = "sess-truth";

    create_session(&h, session);
    let grant_id = issue_grant(
        &h,
        session,
        &["--actions", "execute", "--path-prefix", &workdir],
    );

    // (a) declared local_write, and a write happens -> observed + certified.
    let created = PathBuf::from(&workdir).join("real.txt");
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
            "printf hi > real.txt",
            "--cwd",
            &workdir,
            "--grants",
            &grant_id,
            "--side-effects",
            "local_write",
            "--action-id",
            "act-real",
        ],
    );
    assert!(out.contains("execution:   ok"), "{out}");
    assert!(created.is_file());
    assert!(
        out.contains(&format!("created=[\"{}\"]", created.display())),
        "observed create must be in the fs-diff:\n{out}"
    );

    // (b) declared local_write but the command writes NOTHING -> the receipt must
    // NOT certify LocalWrite (the old bug certified declared-as-happened). The
    // trace shows an empty effect set for this action.
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
            "true",
            "--cwd",
            &workdir,
            "--grants",
            &grant_id,
            "--side-effects",
            "local_write",
            "--action-id",
            "act-noop",
        ],
    );
    let trace = ok(&h, &["trace", "show", "--session", session]);
    // The no-op action's receipt certifies an empty effect set.
    assert!(
        trace.contains("act-noop"),
        "trace must include the no-op action:\n{trace}"
    );
    let noop_line = trace
        .lines()
        .filter(|l| l.contains("receipt:"))
        .find(|l| l.contains("effects=[]"))
        .expect("a receipt with an empty (uncertified) effect set must exist");
    assert!(
        noop_line.contains("effects=[]"),
        "no-op must certify no side effect:\n{trace}"
    );

    let verify = ok(&h, &["journal", "verify", "--session", session]);
    assert!(verify.contains("journal OK"), "{verify}");
}
