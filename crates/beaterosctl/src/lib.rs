//! `beaterosctl`: the operator CLI and durable local store for the beaterOS
//! agent kernel.
//!
//! This crate is the human/operator surface over `beater-os-core`. It persists
//! sessions to an append-only, hash-chained journal on disk and exposes the
//! kernel's deterministic policy admission as inspectable commands. It adds no
//! authority of its own: every capability check is delegated to the core policy
//! engine, outside of any model output.
//!
//! See `docs/beaterosctl.md` for the command reference and a worked MVP flow.

mod args;
mod commands;
mod error;
mod store;

pub use commands::POLICY_VERSION;
pub use error::{CliError, CliResult};
pub use store::{SessionProjection, Store};

use std::env;
use std::path::PathBuf;

use args::ParsedArgs;

/// Default store location when neither `--home` nor `BEATEROS_HOME` is set.
pub const DEFAULT_HOME: &str = ".beateros";

/// The environment variable that selects the store root.
pub const HOME_ENV: &str = "BEATEROS_HOME";

/// Run the CLI from an argument iterator (including the program name).
///
/// Returns the text to print on success. Errors are returned to the caller so
/// the binary can render them and choose an exit code.
pub fn run<I: Iterator<Item = String>>(mut raw: I) -> CliResult<String> {
    let _program = raw.next();
    let args = ParsedArgs::parse(raw)?;

    if args.has_flag("help")
        || args.positional(0).is_none()
        || matches!(args.positional(0), Some("help"))
    {
        return Ok(help_text());
    }

    let home = resolve_home(&args);
    let store = Store::open(home)?;
    commands::dispatch(&store, &args)
}

/// Resolve the store root: `--home` beats `BEATEROS_HOME` beats the default.
fn resolve_home(args: &ParsedArgs) -> PathBuf {
    if let Some(home) = args.get("home") {
        return PathBuf::from(home);
    }
    match env::var(HOME_ENV) {
        Ok(value) if !value.is_empty() => PathBuf::from(value),
        _ => PathBuf::from(DEFAULT_HOME),
    }
}

/// The CLI usage/help text.
pub fn help_text() -> String {
    format!(
        "beaterosctl — operator CLI for the beaterOS agent kernel\n\
         \n\
         Store root precedence: --home > ${HOME_ENV} > ./{DEFAULT_HOME}\n\
         \n\
         COMMANDS\n\
         \x20 session create --agent <id> --workspace <id> --goal <text>\n\
         \x20                [--session <id>] [--created-by <id>] [--policy-profile <p>]\n\
         \x20 session list\n\
         \x20 session show    --session <id>\n\
         \x20 grant issue     --session <id> --resource-kind <kind> --resource-id <id>\n\
         \x20                 --actions <a,b> [--path-prefix <p>]... [--network-allow <h>]...\n\
         \x20                 [--max-risk <r>] [--expires-in-secs <n>] [--reason <text>]\n\
         \x20 action propose  --session <id> --tool <id> --kind <action>\n\
         \x20                 --target-kind <kind> --target <id> --grants <g1,g2>\n\
         \x20                 [--risk <r>] [--side-effects <s,..>] [--data-classes <d,..>]\n\
         \x20                 [--taint <t,..>] [--idempotency-key <k>] [--summary <text>]\n\
         \x20 receipt record  --session <id> --action <id> [--status <s>] [--summary <text>]\n\
         \x20 journal verify  --session <id>\n\
         \x20 trace show      --session <id>\n\
         \n\
         Enum values (kinds, actions, risk, data classes, side effects, taint)\n\
         use the snake_case names from beater-os-core, e.g. file_path, read,\n\
         write, execute, low, medium, high, critical, local_write, code."
    )
}
