//! `beater-os-sandbox`: a scoped local execution lane for the beaterOS kernel.
//!
//! This crate is the mediation point that actually **executes** an admitted
//! action, confined and fail-closed, and observes its side effects as a
//! filesystem diff (`final.md` §8, §10.6, §13.8). It turns a policy decision
//! into a real running OS action without ever handing the agent a raw shell with
//! the user's environment.
//!
//! The lane enforces, in order, the non-negotiable controls from §13.8
//! ("Shell And Code Execution Security") that are achievable in pure,
//! macOS-first user space:
//!
//! 1. **Canonicalized confinement.** The working directory is resolved with
//!    `std::fs::canonicalize` (realpath), which follows every symlink, and is
//!    rejected fail-closed if the canonical path escapes the granted path
//!    prefix(es). This is the symlink-escape defense, and the returned canonical
//!    path is the kernel-derived `resolved_target` (§7.4) that a mediation point
//!    — never the agent — must author.
//! 2. **Scrubbed environment.** The child is spawned with
//!    [`Command::env_clear`](std::process::Command::env_clear); only a minimal
//!    safe `PATH` is set. No inherited global secrets (§13.8).
//! 3. **Bounded execution.** A wall-clock timeout kills a runaway process, and
//!    captured stdout/stderr are capped so a hostile command cannot exhaust
//!    memory. The filesystem walk is bounded by a file count and a per-file byte
//!    ceiling.
//! 4. **Filesystem-diff receipt.** The confined directory is snapshotted
//!    (path -> SHA-256 of contents) before and after execution; the diff of
//!    created / modified / deleted paths is the observed side effect.
//!
//! Everything **fails closed**: any error (canonicalization failure, confinement
//! escape, missing confinement prefix, walk overflow) returns an [`Err`] and no
//! outcome is produced. Number of sandbox lanes is an acceptable compromise
//! (§26); sandbox isolation itself is not. This is the single, portable
//! `Pure function`/local lane from §10.6 — network isolation, seccomp, and
//! container/VM lanes are explicit future targets, not silently assumed here.

use std::collections::BTreeMap;
use std::io::Read;
use std::path::{Path, PathBuf};
use std::process::{Command, Stdio};
use std::thread;
use std::time::{Duration, Instant};

use serde::{Deserialize, Serialize};
use sha2::{Digest, Sha256};
use thiserror::Error;

/// A minimal, safe `PATH` for the confined child. The agent's inherited `PATH`
/// (which may point at attacker-writable directories) is discarded; only the
/// standard system locations are exposed so common tools still resolve.
const SAFE_PATH: &str = "/usr/bin:/bin:/usr/sbin:/sbin";

/// Poll interval for the wall-clock timeout loop.
const POLL_INTERVAL: Duration = Duration::from_millis(10);

/// Result alias for sandbox operations.
pub type SandboxResult<T> = Result<T, SandboxError>;

/// Errors surfaced by the sandbox lane. Every variant is fail-closed: the caller
/// must not execute or certify anything when one is returned.
#[derive(Debug, Error)]
pub enum SandboxError {
    #[error("io error: {0}")]
    Io(#[from] std::io::Error),
    /// The working directory could not be canonicalized (missing, unreadable, or
    /// a broken symlink). We refuse rather than guess a path.
    #[error("cannot canonicalize working directory {path}: {source}")]
    Canonicalize {
        path: String,
        source: std::io::Error,
    },
    /// No confinement prefix was supplied. An unconfined execute lane is an
    /// ambient-authority hole, so we refuse to run at all.
    #[error("refusing to execute without a confinement prefix")]
    NoConfinement,
    /// The canonical working directory is not a directory.
    #[error("working directory {0} is not a directory")]
    NotADirectory(String),
    /// The canonical working directory escapes every granted prefix. This is the
    /// symlink-escape defense firing.
    #[error("working directory {resolved} escapes the granted prefixes")]
    Confinement { resolved: String },
    /// The filesystem walk exceeded its file-count ceiling; we cannot produce a
    /// complete, trustworthy diff, so we fail closed.
    #[error("filesystem walk exceeded the {cap}-file ceiling")]
    FileCapExceeded { cap: usize },
}

/// Bounds on a single sandboxed execution. Defaults are conservative and cheap;
/// the caller may tighten them per action.
#[derive(Clone, Debug, PartialEq, Eq)]
pub struct SandboxLimits {
    /// Wall-clock timeout. The child is killed if it exceeds this.
    pub timeout: Duration,
    /// Maximum captured bytes for stdout and, separately, stderr. Excess is
    /// drained and discarded (never buffered), so output cannot exhaust memory.
    pub max_output_bytes: usize,
    /// Maximum number of files snapshotted in the confined directory. Exceeding
    /// it fails closed rather than truncating a security-relevant diff.
    pub max_files: usize,
    /// Files larger than this are recorded by length (not content) to bound the
    /// memory a single hostile file can force us to read.
    pub max_file_bytes: u64,
}

impl Default for SandboxLimits {
    fn default() -> Self {
        Self {
            timeout: Duration::from_secs(30),
            max_output_bytes: 64 * 1024,
            max_files: 10_000,
            max_file_bytes: 8 * 1024 * 1024,
        }
    }
}

/// A request to execute a scoped shell action in the confined lane.
#[derive(Clone, Debug)]
pub struct SandboxRequest {
    /// The program to run (resolved via [`SAFE_PATH`], not the agent's `PATH`).
    pub command: String,
    /// Arguments passed verbatim (no shell interpolation by this crate).
    pub args: Vec<String>,
    /// The granted working directory. Canonicalized and confined before use.
    pub working_dir: String,
    /// The grant's path prefix(es). The canonical working directory must lie
    /// inside at least one of them or execution is refused.
    pub path_prefixes: Vec<String>,
    /// Execution and snapshot bounds.
    pub limits: SandboxLimits,
}

/// How the child process terminated.
#[derive(Clone, Copy, Debug, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum SandboxStatus {
    /// Exited with status code 0.
    Ok,
    /// Exited with a non-zero status code.
    Failed,
    /// Killed by the wall-clock timeout.
    Timeout,
    /// Terminated by a signal (no exit code).
    Signaled,
}

/// The observed filesystem side effects: a diff of the confined directory
/// snapshotted before and after execution. Paths are relative to the confined
/// root and sorted for a deterministic, hashable receipt.
#[derive(Clone, Debug, Default, PartialEq, Eq, Serialize, Deserialize)]
pub struct FsDiff {
    pub created: Vec<String>,
    pub modified: Vec<String>,
    pub deleted: Vec<String>,
}

impl FsDiff {
    /// Whether nothing changed on disk.
    pub fn is_empty(&self) -> bool {
        self.created.is_empty() && self.modified.is_empty() && self.deleted.is_empty()
    }

    /// A compact, human-legible one-line summary for a receipt's
    /// `side_effect_summary`.
    pub fn summary(&self) -> String {
        format!(
            "fs-diff created={:?} modified={:?} deleted={:?}",
            self.created, self.modified, self.deleted
        )
    }
}

/// The structured outcome of a confined execution.
#[derive(Clone, Debug, PartialEq, Eq)]
pub struct SandboxOutcome {
    /// The kernel-derived, canonical working directory (§7.4). This is the
    /// `resolved_target` a mediation point authors.
    pub resolved_target: PathBuf,
    /// How the process terminated.
    pub status: SandboxStatus,
    /// The exit code, if the process exited normally.
    pub exit_code: Option<i32>,
    /// Captured (capped) stdout bytes.
    pub stdout: Vec<u8>,
    /// Captured (capped) stderr bytes.
    pub stderr: Vec<u8>,
    /// Whether stdout was truncated at the cap.
    pub stdout_truncated: bool,
    /// Whether stderr was truncated at the cap.
    pub stderr_truncated: bool,
    /// The observed filesystem diff.
    pub diff: FsDiff,
}

impl SandboxOutcome {
    /// SHA-256 (hex) of the captured stdout — the receipt's `output_digest`.
    pub fn stdout_digest(&self) -> String {
        sha256_hex(&self.stdout)
    }

    /// SHA-256 (hex) of the captured stderr.
    pub fn stderr_digest(&self) -> String {
        sha256_hex(&self.stderr)
    }

    /// A short status string suitable for a receipt's `status` field.
    pub fn status_str(&self) -> &'static str {
        match self.status {
            SandboxStatus::Ok => "ok",
            SandboxStatus::Failed => "failed",
            SandboxStatus::Timeout => "timeout",
            SandboxStatus::Signaled => "signaled",
        }
    }
}

/// SHA-256 (hex) of arbitrary bytes.
pub fn sha256_hex(bytes: &[u8]) -> String {
    let mut hasher = Sha256::new();
    hasher.update(bytes);
    hex::encode(hasher.finalize())
}

/// The input digest for a command + args: the receipt's `input_digest`. A stable
/// framing (length-prefixed) so distinct argument vectors never collide.
pub fn command_digest(command: &str, args: &[String]) -> String {
    let mut hasher = Sha256::new();
    hasher.update((command.len() as u64).to_le_bytes());
    hasher.update(command.as_bytes());
    hasher.update((args.len() as u64).to_le_bytes());
    for arg in args {
        hasher.update((arg.len() as u64).to_le_bytes());
        hasher.update(arg.as_bytes());
    }
    hex::encode(hasher.finalize())
}

/// Canonicalize `working_dir` and confine it to `path_prefixes`, returning the
/// canonical directory as the kernel-derived `resolved_target`.
///
/// This is the mediation-point computation of `resolved_target` (§7.4): it
/// follows every symlink via realpath, so a working directory (or a symlink
/// component within it) that resolves outside the granted prefix is rejected
/// fail-closed. Both the working directory and each prefix are canonicalized, so
/// the comparison is between real paths and cannot be defeated by symlinks on
/// either side.
pub fn resolve_confined(working_dir: &str, path_prefixes: &[String]) -> SandboxResult<PathBuf> {
    if path_prefixes.is_empty() {
        return Err(SandboxError::NoConfinement);
    }
    let canonical =
        std::fs::canonicalize(working_dir).map_err(|source| SandboxError::Canonicalize {
            path: working_dir.to_string(),
            source,
        })?;
    if !canonical.is_dir() {
        return Err(SandboxError::NotADirectory(canonical.display().to_string()));
    }
    let inside = path_prefixes.iter().any(|prefix| {
        std::fs::canonicalize(prefix)
            .map(|canonical_prefix| path_within(&canonical, &canonical_prefix))
            .unwrap_or(false)
    });
    if !inside {
        return Err(SandboxError::Confinement {
            resolved: canonical.display().to_string(),
        });
    }
    Ok(canonical)
}

/// Whether `path` is `prefix` or lives beneath it. Both must already be
/// canonical; `Path::starts_with` compares whole components, so this is not a
/// string-prefix check (`/a/bc` is not inside `/a/b`).
fn path_within(path: &Path, prefix: &Path) -> bool {
    path == prefix || path.starts_with(prefix)
}

/// Execute a scoped shell action in the confined lane.
///
/// Re-runs [`resolve_confined`] internally (defense in depth): the lane can
/// never spawn a process outside the confined canonical directory, even if
/// called directly. On success the returned [`SandboxOutcome`] carries the exit
/// status, capped stdout/stderr, and the observed filesystem diff.
pub fn execute(request: &SandboxRequest) -> SandboxResult<SandboxOutcome> {
    let resolved_target = resolve_confined(&request.working_dir, &request.path_prefixes)?;

    // Snapshot BEFORE any side effect (fail closed if the walk overflows).
    let before = snapshot(&resolved_target, &request.limits)?;

    let run = run_confined(request, &resolved_target)?;

    // Snapshot AFTER; the diff is the observed side effect.
    let after = snapshot(&resolved_target, &request.limits)?;
    let diff = diff_snapshots(&before, &after);

    Ok(SandboxOutcome {
        resolved_target,
        status: run.status,
        exit_code: run.exit_code,
        stdout: run.stdout,
        stderr: run.stderr,
        stdout_truncated: run.stdout_truncated,
        stderr_truncated: run.stderr_truncated,
        diff,
    })
}

/// The raw result of running the child, before the filesystem diff is attached.
struct RunResult {
    status: SandboxStatus,
    exit_code: Option<i32>,
    stdout: Vec<u8>,
    stderr: Vec<u8>,
    stdout_truncated: bool,
    stderr_truncated: bool,
}

/// Spawn and supervise the confined child: scrubbed environment, confined cwd,
/// bounded output capture, and a wall-clock timeout.
fn run_confined(request: &SandboxRequest, cwd: &Path) -> SandboxResult<RunResult> {
    let mut command = Command::new(&request.command);
    command
        .args(&request.args)
        .current_dir(cwd)
        // No inherited secrets (§13.8): start from an empty environment and add
        // back only a minimal safe PATH so the child still resolves tools.
        .env_clear()
        .env("PATH", SAFE_PATH)
        .stdin(Stdio::null())
        .stdout(Stdio::piped())
        .stderr(Stdio::piped());

    let mut child = command.spawn()?;
    let cap = request.limits.max_output_bytes;
    let out_reader = spawn_capped_reader(child.stdout.take(), cap);
    let err_reader = spawn_capped_reader(child.stderr.take(), cap);

    let deadline = Instant::now() + request.limits.timeout;
    let mut timed_out = false;
    let exit_status = loop {
        match child.try_wait()? {
            Some(status) => break status,
            None => {
                if Instant::now() >= deadline {
                    // Best-effort kill; then reap so we never leak a zombie.
                    let _ = child.kill();
                    let status = child.wait()?;
                    timed_out = true;
                    break status;
                }
                thread::sleep(POLL_INTERVAL);
            }
        }
    };

    // A panicked reader thread is treated as empty-and-truncated (fail closed on
    // capture rather than propagate a panic).
    let (stdout, stdout_truncated) = out_reader.join().unwrap_or_else(|_| (Vec::new(), true));
    let (stderr, stderr_truncated) = err_reader.join().unwrap_or_else(|_| (Vec::new(), true));

    let exit_code = exit_status.code();
    let status = if timed_out {
        SandboxStatus::Timeout
    } else {
        match exit_code {
            Some(0) => SandboxStatus::Ok,
            Some(_) => SandboxStatus::Failed,
            None => SandboxStatus::Signaled,
        }
    };

    Ok(RunResult {
        status,
        exit_code,
        stdout,
        stderr,
        stdout_truncated,
        stderr_truncated,
    })
}

/// Drain a child pipe on its own thread, buffering at most `cap` bytes and
/// discarding the rest. Draining (rather than stopping) prevents a full-pipe
/// deadlock while still bounding memory.
fn spawn_capped_reader<R>(pipe: Option<R>, cap: usize) -> thread::JoinHandle<(Vec<u8>, bool)>
where
    R: Read + Send + 'static,
{
    thread::spawn(move || {
        let mut buffer = Vec::new();
        let mut truncated = false;
        if let Some(mut pipe) = pipe {
            let mut chunk = [0u8; 8192];
            loop {
                match pipe.read(&mut chunk) {
                    Ok(0) => break,
                    Ok(read) => {
                        if buffer.len() < cap {
                            let room = cap - buffer.len();
                            let take = room.min(read);
                            buffer.extend_from_slice(&chunk[..take]);
                            if take < read {
                                truncated = true;
                            }
                        } else {
                            truncated = true;
                        }
                    }
                    Err(_) => {
                        truncated = true;
                        break;
                    }
                }
            }
        }
        (buffer, truncated)
    })
}

/// Snapshot the confined directory as `relative path -> content digest`.
///
/// Symlinks are recorded by their target (never followed), so a diff can note a
/// planted link without traversing outside the confined root. The walk is
/// bounded by `max_files` (fail closed) and `max_file_bytes` (oversize files are
/// recorded by length, not content).
fn snapshot(root: &Path, limits: &SandboxLimits) -> SandboxResult<BTreeMap<String, String>> {
    let mut entries = BTreeMap::new();
    let mut stack = vec![root.to_path_buf()];
    while let Some(dir) = stack.pop() {
        for entry in std::fs::read_dir(&dir)? {
            let entry = entry?;
            let path = entry.path();
            // `DirEntry::file_type` does not traverse symlinks, so a symlinked
            // directory is classified as a symlink and never recursed into.
            let file_type = entry.file_type()?;
            let relative = path
                .strip_prefix(root)
                .unwrap_or(&path)
                .to_string_lossy()
                .to_string();
            if file_type.is_dir() {
                stack.push(path);
                continue;
            }
            let digest = if file_type.is_symlink() {
                let target = std::fs::read_link(&path)
                    .map(|target| target.to_string_lossy().to_string())
                    .unwrap_or_default();
                format!("symlink:{}", sha256_hex(target.as_bytes()))
            } else if file_type.is_file() {
                let length = entry.metadata()?.len();
                if length > limits.max_file_bytes {
                    format!("oversize:{length}")
                } else {
                    sha256_hex(&std::fs::read(&path)?)
                }
            } else {
                // Sockets, FIFOs, devices: record their kind, no content.
                "special".to_string()
            };
            entries.insert(relative, digest);
            if entries.len() > limits.max_files {
                return Err(SandboxError::FileCapExceeded {
                    cap: limits.max_files,
                });
            }
        }
    }
    Ok(entries)
}

/// Compute created / modified / deleted paths between two snapshots.
fn diff_snapshots(before: &BTreeMap<String, String>, after: &BTreeMap<String, String>) -> FsDiff {
    let mut diff = FsDiff::default();
    for (path, digest) in after {
        match before.get(path) {
            None => diff.created.push(path.clone()),
            Some(previous) if previous != digest => diff.modified.push(path.clone()),
            Some(_) => {}
        }
    }
    for path in before.keys() {
        if !after.contains_key(path) {
            diff.deleted.push(path.clone());
        }
    }
    diff
}

#[cfg(test)]
mod tests {
    #![allow(clippy::expect_used, clippy::unwrap_used)]

    use super::*;
    use std::fs;

    /// A temporary directory that cleans itself up. Canonicalized so tests are
    /// robust to macOS `/var` -> `/private/var` symlinking.
    struct TempDir {
        path: PathBuf,
    }

    impl TempDir {
        fn new(tag: &str) -> Self {
            let unique = format!(
                "beater-sandbox-{tag}-{}-{:?}",
                std::process::id(),
                std::time::SystemTime::now()
                    .duration_since(std::time::UNIX_EPOCH)
                    .unwrap()
                    .as_nanos()
            );
            let path = std::env::temp_dir().join(unique);
            fs::create_dir_all(&path).unwrap();
            let path = fs::canonicalize(&path).unwrap();
            Self { path }
        }

        fn str(&self) -> String {
            self.path.display().to_string()
        }
    }

    impl Drop for TempDir {
        fn drop(&mut self) {
            let _ = fs::remove_dir_all(&self.path);
        }
    }

    fn sh(script: &str) -> (String, Vec<String>) {
        ("sh".to_string(), vec!["-c".to_string(), script.to_string()])
    }

    #[test]
    fn executes_and_diffs_a_created_file() {
        let dir = TempDir::new("create");
        let (command, args) = sh("printf hello > out.txt");
        let outcome = execute(&SandboxRequest {
            command,
            args,
            working_dir: dir.str(),
            path_prefixes: vec![dir.str()],
            limits: SandboxLimits::default(),
        })
        .expect("execute should succeed");

        assert_eq!(outcome.status, SandboxStatus::Ok);
        assert_eq!(outcome.exit_code, Some(0));
        assert!(dir.path.join("out.txt").is_file(), "file must really exist");
        assert_eq!(outcome.diff.created, vec!["out.txt".to_string()]);
        assert!(outcome.diff.modified.is_empty());
        assert!(outcome.diff.deleted.is_empty());
        assert_eq!(outcome.resolved_target, dir.path);
    }

    #[test]
    fn detects_modified_and_deleted() {
        let dir = TempDir::new("modify");
        fs::write(dir.path.join("keep.txt"), b"one").unwrap();
        fs::write(dir.path.join("gone.txt"), b"bye").unwrap();
        let (command, args) = sh("printf two > keep.txt; rm gone.txt");
        let outcome = execute(&SandboxRequest {
            command,
            args,
            working_dir: dir.str(),
            path_prefixes: vec![dir.str()],
            limits: SandboxLimits::default(),
        })
        .expect("execute");
        assert_eq!(outcome.diff.modified, vec!["keep.txt".to_string()]);
        assert_eq!(outcome.diff.deleted, vec!["gone.txt".to_string()]);
        assert!(outcome.diff.created.is_empty());
    }

    #[test]
    fn symlink_escape_is_rejected_fail_closed() {
        let granted = TempDir::new("granted");
        let outside = TempDir::new("outside");
        let link = granted.path.join("escape");
        std::os::unix::fs::symlink(&outside.path, &link).unwrap();

        let result = execute(&SandboxRequest {
            command: "sh".to_string(),
            args: vec!["-c".to_string(), "touch pwned".to_string()],
            working_dir: link.display().to_string(),
            path_prefixes: vec![granted.str()],
            limits: SandboxLimits::default(),
        });
        assert!(
            matches!(result, Err(SandboxError::Confinement { .. })),
            "symlink escape must be rejected: {result:?}"
        );
        // Nothing executed: no file was created in the escape target.
        assert!(!outside.path.join("pwned").exists());
    }

    #[test]
    fn empty_prefixes_refuse_to_run() {
        let dir = TempDir::new("noconfine");
        let result = execute(&SandboxRequest {
            command: "sh".to_string(),
            args: vec!["-c".to_string(), "touch x".to_string()],
            working_dir: dir.str(),
            path_prefixes: vec![],
            limits: SandboxLimits::default(),
        });
        assert!(matches!(result, Err(SandboxError::NoConfinement)));
        assert!(!dir.path.join("x").exists());
    }

    #[test]
    fn environment_is_scrubbed_of_secrets() {
        // The workspace forbids `unsafe`, so we cannot call the (unsafe in
        // edition 2024) `std::env::set_var`. Instead we prove scrubbing against a
        // variable cargo injects into the test process: `CARGO_PKG_NAME`. It is
        // present in the parent (asserted below) but must NOT leak to the child
        // once `env_clear` runs. This stands in for any inherited secret.
        let secret = std::env::var("CARGO_PKG_NAME").expect("cargo sets CARGO_PKG_NAME");
        assert!(!secret.is_empty(), "expected a non-empty parent env var");

        let dir = TempDir::new("secret");
        let (command, args) = sh("echo pkg=$CARGO_PKG_NAME");
        let outcome = execute(&SandboxRequest {
            command,
            args,
            working_dir: dir.str(),
            path_prefixes: vec![dir.str()],
            limits: SandboxLimits::default(),
        })
        .expect("execute");
        let stdout = String::from_utf8_lossy(&outcome.stdout);
        assert!(
            !stdout.contains(&secret),
            "env_clear must scrub inherited env, but child saw {secret:?}: {stdout:?}"
        );
        assert!(stdout.contains("pkg="), "command should still run");
    }

    #[test]
    fn output_is_capped_not_unbounded() {
        let dir = TempDir::new("cap");
        // Emit far more than the cap; capture must be bounded and flagged.
        let (command, args) = sh("head -c 200000 /dev/zero | tr '\\0' 'a'");
        let limits = SandboxLimits {
            max_output_bytes: 4096,
            ..SandboxLimits::default()
        };
        let outcome = execute(&SandboxRequest {
            command,
            args,
            working_dir: dir.str(),
            path_prefixes: vec![dir.str()],
            limits,
        })
        .expect("execute");
        assert_eq!(outcome.stdout.len(), 4096, "stdout must be capped");
        assert!(outcome.stdout_truncated, "truncation must be flagged");
    }

    #[test]
    fn timeout_kills_a_runaway() {
        let dir = TempDir::new("timeout");
        let (command, args) = sh("sleep 30");
        let limits = SandboxLimits {
            timeout: Duration::from_millis(300),
            ..SandboxLimits::default()
        };
        let start = Instant::now();
        let outcome = execute(&SandboxRequest {
            command,
            args,
            working_dir: dir.str(),
            path_prefixes: vec![dir.str()],
            limits,
        })
        .expect("execute");
        assert_eq!(outcome.status, SandboxStatus::Timeout);
        assert!(
            start.elapsed() < Duration::from_secs(5),
            "the runaway must be killed promptly"
        );
    }

    #[test]
    fn file_cap_fails_closed() {
        let dir = TempDir::new("filecap");
        for i in 0..5 {
            fs::write(dir.path.join(format!("f{i}")), b"x").unwrap();
        }
        let limits = SandboxLimits {
            max_files: 2,
            ..SandboxLimits::default()
        };
        let result = execute(&SandboxRequest {
            command: "sh".to_string(),
            args: vec!["-c".to_string(), "true".to_string()],
            working_dir: dir.str(),
            path_prefixes: vec![dir.str()],
            limits,
        });
        assert!(matches!(result, Err(SandboxError::FileCapExceeded { .. })));
    }

    #[test]
    fn command_digest_is_stable_and_arg_sensitive() {
        let a = command_digest("git", &["add".to_string(), ".".to_string()]);
        let b = command_digest("git", &["add".to_string(), ".".to_string()]);
        let c = command_digest("git", &["add".to_string(), "-A".to_string()]);
        assert_eq!(a, b);
        assert_ne!(a, c);
    }
}
