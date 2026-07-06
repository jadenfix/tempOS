//! `beater-os-sandbox`: a scoped local execution lane for the beaterOS kernel.
//!
//! This crate is the mediation point that actually **executes** an admitted
//! action, confined and fail-closed, and observes its side effects as a
//! filesystem diff (`final.md` §8, §10.6, §13.8). It turns a policy decision
//! into a real running OS action without ever handing the agent a raw shell with
//! the user's environment.
//!
//! The lane enforces, in order, the non-negotiable controls from §13.8
//! ("Shell And Code Execution Security"):
//!
//! 1. **Canonicalized confinement.** The working directory is resolved with
//!    `std::fs::canonicalize` (realpath), which follows every symlink, and is
//!    rejected fail-closed if the canonical path escapes the granted path
//!    prefix(es). This is the symlink-escape defense, and the returned canonical
//!    path is the kernel-derived `resolved_target` (§7.4) that a mediation point
//!    — never the agent — must author.
//! 2. **Real OS filesystem confinement.** cwd anchoring is not confinement: the
//!    agent controls the full `sh -c` command and can name absolute paths, `..`,
//!    or read arbitrary files regardless of cwd. So the child is wrapped in the
//!    macOS **Seatbelt** sandbox (`/usr/bin/sandbox-exec`) with a generated
//!    *deny-default* profile: it may WRITE only within the grant-derived
//!    canonical prefixes (plus `/dev/null`), may READ system paths (to load a
//!    shell/binary and the dynamic loader's shared cache) plus those prefixes,
//!    and is DENIED every other read of user data and every write. If the
//!    enforcer is unavailable, the lane **fails closed** — it never runs an
//!    unconfined command. This is the macOS lane; a Linux lane (seccomp-bpf +
//!    Landlock + mount namespaces) is a future implementor of the same
//!    [`Confiner`] seam.
//! 3. **Explicit environment allowlist.** The child is spawned with
//!    [`Command::env_clear`](std::process::Command::env_clear); it receives
//!    exactly the variables listed on [`SandboxRequest::environment`]. No
//!    inherited global secrets (§13.8), and no implicit `PATH`.
//! 4. **Bounded execution.** A wall-clock timeout kills a runaway process, and
//!    captured stdout/stderr are capped so a hostile command cannot exhaust
//!    memory. The filesystem walk is bounded by a file count and a per-file byte
//!    ceiling.
//! 5. **Filesystem-diff receipt.** *All* confined prefixes are snapshotted
//!    (path -> SHA-256 of contents) before and after execution; the diff of
//!    created / modified / deleted paths is the OBSERVED side effect — the
//!    source of truth for the receipt, never the agent's declared expectation.
//!
//! Everything **fails closed**: any error (canonicalization failure, confinement
//! escape, missing confinement prefix, unavailable enforcer, walk overflow)
//! returns an [`Err`] and no outcome is produced. Number of sandbox lanes is an
//! acceptable compromise (§26); sandbox isolation itself is not.

use std::collections::BTreeMap;
use std::io::Read;
use std::path::{Path, PathBuf};
use std::process::{Command, Stdio};
use std::thread;
use std::time::{Duration, Instant};

use serde::{Deserialize, Serialize};
use sha2::{Digest, Sha256};
use thiserror::Error;

/// A minimal, safe `PATH` callers may opt into for the confined child. The
/// agent's inherited `PATH` must never be forwarded; this value exposes only
/// standard system locations so common tools still resolve.
const SAFE_PATH: &str = "/usr/bin:/bin:/usr/sbin:/sbin";

/// Absolute path to the macOS Seatbelt runner. Hardcoded (not resolved via
/// `PATH`) so a hostile environment cannot shim a fake enforcer in front of it.
const SANDBOX_EXEC: &str = "/usr/bin/sandbox-exec";

/// System read-only subpaths the child needs to load a shell/binary and the
/// dynamic loader's shared cache. These expose the OS image and libraries, not
/// user data; reads of arbitrary user paths outside the granted prefixes stay
/// denied. `/private/var/select` is the shell selector; `/private/var/db/dyld`
/// is the loader database; on modern macOS the shared cache lives under
/// `/System/Volumes/Preboot/Cryptexes`, covered by the `/System` subpath.
const SYSTEM_READ_SUBPATHS: &[&str] = &[
    "/usr",
    "/bin",
    "/sbin",
    "/System",
    "/Library",
    "/private/var/db/dyld",
    "/private/var/select",
    "/dev",
];

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
    /// The OS filesystem-confinement enforcer is unavailable (e.g. this is not
    /// macOS, or `/usr/bin/sandbox-exec` is missing). We refuse to run an
    /// unconfined command. The Linux lane is a future implementor of [`Confiner`].
    #[error(
        "filesystem confinement enforcer /usr/bin/sandbox-exec is unavailable; \
         refusing to run unconfined"
    )]
    ConfinementUnavailable,
    /// A granted prefix or the program path is not valid UTF-8 and cannot be
    /// embedded into a Seatbelt profile safely, so we fail closed.
    #[error("path {path} cannot be encoded into a sandbox profile")]
    ProfilePath { path: String },
    /// Environment variables are authority-bearing process inputs. Invalid or
    /// ambiguous names/values are rejected before spawn.
    #[error("invalid sandbox environment variable {name:?}: {reason}")]
    InvalidEnvironment { name: String, reason: &'static str },
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
    /// Maximum number of explicitly allowed environment variables.
    pub max_environment_vars: usize,
    /// Maximum combined bytes across environment variable names and values.
    pub max_environment_bytes: usize,
}

impl Default for SandboxLimits {
    fn default() -> Self {
        Self {
            timeout: Duration::from_secs(30),
            max_output_bytes: 64 * 1024,
            max_files: 10_000,
            max_file_bytes: 8 * 1024 * 1024,
            max_environment_vars: 16,
            max_environment_bytes: 8 * 1024,
        }
    }
}

/// A request to execute a scoped shell action in the confined lane.
#[derive(Clone, Debug)]
pub struct SandboxRequest {
    /// The program to run (resolved by the OS; a canonical read allowlist is
    /// derived via [`SAFE_PATH`] for bare program names).
    pub command: String,
    /// Arguments passed verbatim (no shell interpolation by this crate).
    pub args: Vec<String>,
    /// Environment variables explicitly allowed for this action. The sandbox
    /// starts from `env_clear`; an empty map means the child receives no
    /// variables, including no `PATH`.
    pub environment: BTreeMap<String, String>,
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

/// The observed filesystem side effects: a diff of every confined prefix
/// snapshotted before and after execution. Paths are absolute (unambiguous
/// across distinct prefixes) and sorted for a deterministic, hashable receipt.
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

/// A narrow environment allowlist for callers that intentionally want standard
/// system command lookup. Passing an empty map to [`SandboxRequest`] is stricter
/// and gives the child no `PATH`.
pub fn safe_path_environment() -> BTreeMap<String, String> {
    BTreeMap::from([("PATH".to_string(), SAFE_PATH.to_string())])
}

/// SHA-256 (hex) of arbitrary bytes.
pub fn sha256_hex(bytes: &[u8]) -> String {
    let mut hasher = Sha256::new();
    hasher.update(bytes);
    hex::encode(hasher.finalize())
}

/// The input digest for a command + args + explicit environment: the receipt's
/// `input_digest`. A stable framing (length-prefixed) so distinct inputs never
/// collide.
pub fn command_digest(
    command: &str,
    args: &[String],
    environment: &BTreeMap<String, String>,
) -> String {
    let mut hasher = Sha256::new();
    hasher.update((command.len() as u64).to_le_bytes());
    hasher.update(command.as_bytes());
    hasher.update((args.len() as u64).to_le_bytes());
    for arg in args {
        hasher.update((arg.len() as u64).to_le_bytes());
        hasher.update(arg.as_bytes());
    }
    hasher.update((environment.len() as u64).to_le_bytes());
    for (name, value) in environment {
        hasher.update((name.len() as u64).to_le_bytes());
        hasher.update(name.as_bytes());
        hasher.update((value.len() as u64).to_le_bytes());
        hasher.update(value.as_bytes());
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
    validate_environment(&request.environment, &request.limits)?;

    let resolved_target = resolve_confined(&request.working_dir, &request.path_prefixes)?;

    // Observe across EVERY granted prefix, not just the cwd: the confinement
    // permits writes anywhere within the grant's authority (e.g. a sibling
    // prefix named by absolute path), so a receipt that watched only the cwd
    // would under-report. Paths in the diff are absolute and unambiguous.
    let prefixes = canonical_prefixes(&request.path_prefixes)?;

    // Snapshot BEFORE any side effect (fail closed if the walk overflows).
    let before = snapshot_all(&prefixes, &request.limits)?;

    let run = run_confined(request, &resolved_target)?;

    // Snapshot AFTER; the diff is the observed side effect.
    let after = snapshot_all(&prefixes, &request.limits)?;
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

/// A filesystem-confinement backend: the mechanism that restricts what the
/// child process can read and write. Only the enforcing mechanism differs per
/// OS — the rest of the lane (canonicalized cwd, env scrub, timeout, output
/// caps, fs-diff receipt) is OS-independent.
///
/// macOS ships [`SeatbeltConfiner`]. A Linux lane (seccomp-bpf + Landlock +
/// mount namespaces) is a future implementor of this same seam; nothing above
/// it changes.
trait Confiner {
    /// Whether this backend actually enforces on the current host. A backend
    /// that cannot enforce must report `false` so the lane fails closed rather
    /// than running unconfined.
    fn enforces(&self) -> bool;

    /// Build a ready-to-spawn command that runs `program` + `args` confined so
    /// it may WRITE only within `prefixes` (canonical realpaths) plus
    /// `/dev/null`, and may READ system paths plus those prefixes (and
    /// `program_path`, if resolved). All other reads of user data and all writes
    /// are denied. cwd/env/stdio are applied by the caller.
    fn confined_command(
        &self,
        program: &str,
        args: &[String],
        prefixes: &[PathBuf],
        program_path: Option<&Path>,
    ) -> SandboxResult<Command>;
}

/// The macOS filesystem-confinement lane, backed by Apple Seatbelt via
/// `/usr/bin/sandbox-exec` and a generated deny-default profile.
struct SeatbeltConfiner;

impl Confiner for SeatbeltConfiner {
    fn enforces(&self) -> bool {
        Path::new(SANDBOX_EXEC).is_file()
    }

    fn confined_command(
        &self,
        program: &str,
        args: &[String],
        prefixes: &[PathBuf],
        program_path: Option<&Path>,
    ) -> SandboxResult<Command> {
        let profile = build_seatbelt_profile(prefixes, program_path)?;
        let mut command = Command::new(SANDBOX_EXEC);
        command
            .arg("-p")
            .arg(profile)
            .arg("--")
            .arg(program)
            .args(args);
        Ok(command)
    }
}

/// Escape a path for embedding inside a Seatbelt string literal (`"..."`).
/// Backslash and double-quote are the only characters that can terminate or
/// alter the literal, so escaping them prevents a crafted prefix from injecting
/// profile syntax (e.g. closing the string and appending an `(allow ...)`).
fn escape_seatbelt(s: &str) -> String {
    let mut out = String::with_capacity(s.len() + 2);
    for c in s.chars() {
        if c == '\\' || c == '"' {
            out.push('\\');
        }
        out.push(c);
    }
    out
}

/// A canonical path -> its escaped Seatbelt string-literal contents. Rejects a
/// non-UTF-8 path fail-closed: we will not embed a lossy path into a security
/// profile.
fn path_to_profile_str(path: &Path) -> SandboxResult<String> {
    let s = path.to_str().ok_or_else(|| SandboxError::ProfilePath {
        path: path.display().to_string(),
    })?;
    Ok(escape_seatbelt(s))
}

/// Build the deny-default Seatbelt profile confining the child to `prefixes`.
///
/// The child may READ system paths (so a shell/binary and the dyld shared cache
/// load) plus each granted prefix, and may WRITE only within the prefixes and
/// `/dev/null`. Every other read of user data and every write is denied.
///
/// `(literal "/")` grants read of the root directory *node* itself — dyld's
/// `CacheFinder` stats `/` on startup and aborts (SIGABRT) without it — but does
/// NOT grant the whole tree. `file-read-metadata` is allowed globally so path
/// resolution / `getcwd` on the cwd's ancestors works; it exposes existence and
/// size, never file contents (the read-secret exploit reads *contents* and stays
/// denied). Every embedded path is a canonical realpath, escaped against
/// injection.
fn build_seatbelt_profile(
    prefixes: &[PathBuf],
    program_path: Option<&Path>,
) -> SandboxResult<String> {
    let mut reads = String::from("(literal \"/\")");
    for sys in SYSTEM_READ_SUBPATHS {
        // Compile-time constants: no untrusted input, no escaping needed.
        reads.push_str(&format!(" (subpath \"{sys}\")"));
    }
    if let Some(program) = program_path {
        reads.push_str(&format!(" (subpath \"{}\")", path_to_profile_str(program)?));
    }

    let mut writes = String::from("(literal \"/dev/null\")");
    for prefix in prefixes {
        let escaped = path_to_profile_str(prefix)?;
        reads.push_str(&format!(" (subpath \"{escaped}\")"));
        writes.push_str(&format!(" (subpath \"{escaped}\")"));
    }

    Ok(format!(
        "(version 1)\n\
         (deny default)\n\
         (allow process-exec*)\n\
         (allow process-fork)\n\
         (allow signal (target self))\n\
         (allow sysctl-read)\n\
         (allow mach-lookup)\n\
         (allow file-read-metadata)\n\
         (allow file-read* {reads})\n\
         (allow file-write* {writes})\n"
    ))
}

/// Canonicalize each granted prefix (realpath) for the confinement profile.
/// Prefixes that cannot be canonicalized (missing / broken) grant no authority
/// and are dropped; if none survive we fail closed with [`SandboxError::NoConfinement`].
fn canonical_prefixes(prefixes: &[String]) -> SandboxResult<Vec<PathBuf>> {
    let mut out: Vec<PathBuf> = prefixes
        .iter()
        .filter_map(|prefix| std::fs::canonicalize(prefix).ok())
        .collect();
    out.sort();
    out.dedup();
    if out.is_empty() {
        return Err(SandboxError::NoConfinement);
    }
    Ok(out)
}

/// Resolve the child program to a canonical path for the read allowlist. An
/// absolute or path-bearing command is canonicalized directly; a bare name is
/// resolved against [`SAFE_PATH`]. Returns `None` when it cannot be resolved —
/// standard tools are already covered by the system read subpaths, and a program
/// that cannot be found simply fails to exec inside the sandbox (fail-closed).
fn resolve_program_path(command: &str) -> Option<PathBuf> {
    if command.contains('/') {
        return std::fs::canonicalize(command).ok();
    }
    SAFE_PATH
        .split(':')
        .find_map(|dir| std::fs::canonicalize(Path::new(dir).join(command)).ok())
}

pub fn validate_environment(
    environment: &BTreeMap<String, String>,
    limits: &SandboxLimits,
) -> SandboxResult<()> {
    if environment.len() > limits.max_environment_vars {
        return Err(SandboxError::InvalidEnvironment {
            name: "*".to_string(),
            reason: "too many variables",
        });
    }

    let total_bytes = environment
        .iter()
        .map(|(name, value)| name.len() + value.len())
        .sum::<usize>();
    if total_bytes > limits.max_environment_bytes {
        return Err(SandboxError::InvalidEnvironment {
            name: "*".to_string(),
            reason: "environment byte limit exceeded",
        });
    }

    for (name, value) in environment {
        if name.is_empty() {
            return Err(SandboxError::InvalidEnvironment {
                name: name.clone(),
                reason: "name is empty",
            });
        }
        if name.contains('=') {
            return Err(SandboxError::InvalidEnvironment {
                name: name.clone(),
                reason: "name contains '='",
            });
        }
        if name.as_bytes().contains(&0) {
            return Err(SandboxError::InvalidEnvironment {
                name: name.clone(),
                reason: "name contains NUL",
            });
        }
        if name
            .chars()
            .next()
            .is_some_and(|first| first.is_ascii_digit())
            || !name.chars().all(|c| c.is_ascii_alphanumeric() || c == '_')
        {
            return Err(SandboxError::InvalidEnvironment {
                name: name.clone(),
                reason: "name is not a portable environment variable identifier",
            });
        }
        if value.as_bytes().contains(&0) {
            return Err(SandboxError::InvalidEnvironment {
                name: name.clone(),
                reason: "value contains NUL",
            });
        }
    }
    Ok(())
}

/// Spawn and supervise the confined child: real OS filesystem confinement
/// (macOS Seatbelt), scrubbed environment, confined cwd, bounded output capture,
/// and a wall-clock timeout.
fn run_confined(request: &SandboxRequest, cwd: &Path) -> SandboxResult<RunResult> {
    // Real OS filesystem confinement. Fail closed if the enforcer cannot run —
    // never fall back to an unconfined process.
    let confiner = SeatbeltConfiner;
    if !confiner.enforces() {
        return Err(SandboxError::ConfinementUnavailable);
    }
    let prefixes = canonical_prefixes(&request.path_prefixes)?;
    let program_path = resolve_program_path(&request.command);
    let mut command = confiner.confined_command(
        &request.command,
        &request.args,
        &prefixes,
        program_path.as_deref(),
    )?;
    command
        .current_dir(cwd)
        .env_clear()
        .stdin(Stdio::null())
        .stdout(Stdio::piped())
        .stderr(Stdio::piped());
    for (name, value) in &request.environment {
        command.env(name, value);
    }

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

/// Snapshot every granted prefix as `absolute path -> content digest`, unioned
/// into one map. Absolute keys keep entries unambiguous across distinct prefixes.
/// The combined walk is bounded by `max_files` (fail closed).
fn snapshot_all(
    prefixes: &[PathBuf],
    limits: &SandboxLimits,
) -> SandboxResult<BTreeMap<String, String>> {
    let mut entries = BTreeMap::new();
    for root in prefixes {
        snapshot_into(root, limits, &mut entries)?;
    }
    Ok(entries)
}

/// Walk one prefix, inserting `absolute path -> content digest` into `entries`.
///
/// Symlinks are recorded by their target (never followed), so a diff can note a
/// planted link without traversing outside the confined root. Oversize files are
/// recorded by length (not content). The cumulative `entries` count is bounded
/// by `max_files` (fail closed).
fn snapshot_into(
    root: &Path,
    limits: &SandboxLimits,
    entries: &mut BTreeMap<String, String>,
) -> SandboxResult<()> {
    let mut stack = vec![root.to_path_buf()];
    while let Some(dir) = stack.pop() {
        for entry in std::fs::read_dir(&dir)? {
            let entry = entry?;
            let path = entry.path();
            // `DirEntry::file_type` does not traverse symlinks, so a symlinked
            // directory is classified as a symlink and never recursed into.
            let file_type = entry.file_type()?;
            let key = path.to_string_lossy().to_string();
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
            entries.insert(key, digest);
            if entries.len() > limits.max_files {
                return Err(SandboxError::FileCapExceeded {
                    cap: limits.max_files,
                });
            }
        }
    }
    Ok(())
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
            environment: safe_path_environment(),
            working_dir: dir.str(),
            path_prefixes: vec![dir.str()],
            limits: SandboxLimits::default(),
        })
        .expect("execute should succeed");

        assert_eq!(outcome.status, SandboxStatus::Ok);
        assert_eq!(outcome.exit_code, Some(0));
        assert!(dir.path.join("out.txt").is_file(), "file must really exist");
        assert_eq!(
            outcome.diff.created,
            vec![dir.path.join("out.txt").display().to_string()]
        );
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
            environment: safe_path_environment(),
            working_dir: dir.str(),
            path_prefixes: vec![dir.str()],
            limits: SandboxLimits::default(),
        })
        .expect("execute");
        assert_eq!(
            outcome.diff.modified,
            vec![dir.path.join("keep.txt").display().to_string()]
        );
        assert_eq!(
            outcome.diff.deleted,
            vec![dir.path.join("gone.txt").display().to_string()]
        );
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
            environment: safe_path_environment(),
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

    /// The exact exploit the reviewer demonstrated: an absolute-path write
    /// OUTSIDE the granted prefix. cwd anchoring alone admitted it; real OS
    /// confinement must DENY it — no file created outside, and the child status
    /// is not a clean success.
    #[test]
    fn absolute_write_outside_prefix_is_denied() {
        let work = TempDir::new("work-abs");
        let outside = TempDir::new("outside-abs");
        let escaped = outside.path.join("escaped_abs.txt");
        let script = format!("printf PWNED > {}", escaped.display());
        let (command, args) = sh(&script);
        let outcome = execute(&SandboxRequest {
            command,
            args,
            environment: safe_path_environment(),
            working_dir: work.str(),
            path_prefixes: vec![work.str()],
            limits: SandboxLimits::default(),
        })
        .expect("execute should still produce an outcome");

        assert!(
            !escaped.exists(),
            "sandbox must deny the out-of-prefix write; no file may appear outside"
        );
        assert_ne!(
            outcome.status,
            SandboxStatus::Ok,
            "a denied write must NOT be receipted as a clean success"
        );
        assert!(
            outcome.diff.is_empty(),
            "no observed side effect inside the prefix: {:?}",
            outcome.diff
        );
    }

    /// The `../` variant: a write to the PARENT of the granted prefix. Must be
    /// denied by the OS sandbox regardless of cwd.
    #[test]
    fn dotdot_write_to_parent_is_denied() {
        let work = TempDir::new("work-dd");
        let parent = work
            .path
            .parent()
            .expect("temp dir has a parent")
            .to_path_buf();
        let escaped = parent.join("dotdot_escape.txt");
        let script = format!("printf x > {}", escaped.display());
        let (command, args) = sh(&script);
        let outcome = execute(&SandboxRequest {
            command,
            args,
            environment: safe_path_environment(),
            working_dir: work.str(),
            path_prefixes: vec![work.str()],
            limits: SandboxLimits::default(),
        })
        .expect("execute");
        assert!(
            !escaped.exists(),
            "sandbox must deny the ../ write to the prefix parent"
        );
        assert_ne!(outcome.status, SandboxStatus::Ok);
    }

    /// Reading a secret file entirely OUTSIDE the prefix must be denied: the read
    /// fails and no secret bytes reach stdout.
    #[test]
    fn read_outside_prefix_is_denied() {
        let work = TempDir::new("work-read");
        let outside = TempDir::new("outside-read");
        let secret = outside.path.join("secret.txt");
        fs::write(&secret, b"TOP-SECRET-XYZZY").unwrap();
        let script = format!("cat {}", secret.display());
        let (command, args) = sh(&script);
        let outcome = execute(&SandboxRequest {
            command,
            args,
            environment: safe_path_environment(),
            working_dir: work.str(),
            path_prefixes: vec![work.str()],
            limits: SandboxLimits::default(),
        })
        .expect("execute");
        let stdout = String::from_utf8_lossy(&outcome.stdout);
        assert!(
            !stdout.contains("TOP-SECRET-XYZZY"),
            "secret content must not leak past the sandbox: {stdout:?}"
        );
        assert_ne!(
            outcome.status,
            SandboxStatus::Ok,
            "a denied read must surface as a non-zero child status"
        );
    }

    /// A legitimate write INSIDE the prefix still succeeds, and the receipt's
    /// fs-diff truthfully shows the created file (observed, absolute path).
    #[test]
    fn legitimate_write_inside_prefix_succeeds_and_is_observed() {
        let work = TempDir::new("work-legit");
        let (command, args) = sh("echo hi > allowed.txt");
        let outcome = execute(&SandboxRequest {
            command,
            args,
            environment: safe_path_environment(),
            working_dir: work.str(),
            path_prefixes: vec![work.str()],
            limits: SandboxLimits::default(),
        })
        .expect("execute");
        assert_eq!(outcome.status, SandboxStatus::Ok);
        assert_eq!(outcome.exit_code, Some(0));
        let created = work.path.join("allowed.txt");
        assert!(created.is_file(), "the in-prefix file must really exist");
        assert_eq!(
            outcome.diff.created,
            vec![created.display().to_string()],
            "the observed fs-diff must show the created file"
        );
    }

    /// The generated profile must be a deny-default Seatbelt profile that grants
    /// writes only within each (escaped) prefix, and a crafted prefix cannot
    /// inject profile syntax.
    #[test]
    fn seatbelt_profile_is_deny_default_and_injection_safe() {
        let prefix = PathBuf::from("/tmp/a\"b\\c");
        let profile = build_seatbelt_profile(&[prefix], None).expect("profile");
        assert!(profile.starts_with("(version 1)\n(deny default)\n"));
        assert!(profile.contains("(allow file-write*"));
        // The embedded quote/backslash are escaped, so the string literal is not
        // terminated early: the escaped form appears verbatim.
        assert!(
            profile.contains("(subpath \"/tmp/a\\\"b\\\\c\")"),
            "prefix must be escaped in the profile: {profile}"
        );
    }

    #[test]
    fn empty_prefixes_refuse_to_run() {
        let dir = TempDir::new("noconfine");
        let result = execute(&SandboxRequest {
            command: "sh".to_string(),
            args: vec!["-c".to_string(), "touch x".to_string()],
            environment: safe_path_environment(),
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
            environment: safe_path_environment(),
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
    fn empty_environment_allows_no_variables() {
        let dir = TempDir::new("empty-env");
        let outcome = execute(&SandboxRequest {
            command: "/usr/bin/env".to_string(),
            args: Vec::new(),
            environment: BTreeMap::new(),
            working_dir: dir.str(),
            path_prefixes: vec![dir.str()],
            limits: SandboxLimits::default(),
        })
        .expect("execute");

        assert_eq!(outcome.status, SandboxStatus::Ok);
        assert!(
            outcome.stdout.is_empty(),
            "empty allowlist must produce no child environment: {:?}",
            String::from_utf8_lossy(&outcome.stdout)
        );
    }

    #[test]
    fn explicit_environment_allowlist_is_passed_without_inheritance() {
        let parent = std::env::var("CARGO_PKG_NAME").expect("cargo sets CARGO_PKG_NAME");
        let dir = TempDir::new("allow-env");
        let mut environment = BTreeMap::new();
        environment.insert("BEATER_ALLOWED".to_string(), "ok".to_string());
        let outcome = execute(&SandboxRequest {
            command: "/bin/sh".to_string(),
            args: vec![
                "-c".to_string(),
                "printf '%s:%s' \"$BEATER_ALLOWED\" \"$CARGO_PKG_NAME\"".to_string(),
            ],
            environment,
            working_dir: dir.str(),
            path_prefixes: vec![dir.str()],
            limits: SandboxLimits::default(),
        })
        .expect("execute");

        assert_eq!(outcome.status, SandboxStatus::Ok);
        let stdout = String::from_utf8_lossy(&outcome.stdout);
        assert_eq!(stdout, "ok:");
        assert!(!stdout.contains(&parent));
    }

    #[test]
    fn invalid_environment_fails_closed_before_execution() {
        let dir = TempDir::new("bad-env");
        let mut environment = BTreeMap::new();
        environment.insert("BAD-NAME".to_string(), "x".to_string());
        let result = execute(&SandboxRequest {
            command: "/bin/sh".to_string(),
            args: vec!["-c".to_string(), "touch should_not_exist".to_string()],
            environment,
            working_dir: dir.str(),
            path_prefixes: vec![dir.str()],
            limits: SandboxLimits::default(),
        });

        assert!(matches!(
            result,
            Err(SandboxError::InvalidEnvironment {
                name,
                reason: "name is not a portable environment variable identifier"
            }) if name == "BAD-NAME"
        ));
        assert!(!dir.path.join("should_not_exist").exists());
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
            environment: safe_path_environment(),
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
            environment: safe_path_environment(),
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
            environment: safe_path_environment(),
            working_dir: dir.str(),
            path_prefixes: vec![dir.str()],
            limits,
        });
        assert!(matches!(result, Err(SandboxError::FileCapExceeded { .. })));
    }

    #[test]
    fn command_digest_is_stable_and_arg_sensitive() {
        let env = safe_path_environment();
        let a = command_digest("git", &["add".to_string(), ".".to_string()], &env);
        let b = command_digest("git", &["add".to_string(), ".".to_string()], &env);
        let c = command_digest("git", &["add".to_string(), "-A".to_string()], &env);
        let mut changed_env = env.clone();
        changed_env.insert("BEATER_MODE".to_string(), "audit".to_string());
        let d = command_digest("git", &["add".to_string(), ".".to_string()], &changed_env);
        assert_eq!(a, b);
        assert_ne!(a, c);
        assert_ne!(a, d);
    }
}
