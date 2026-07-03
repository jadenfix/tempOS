//! `beater-coord`: command-line driver for the beaterOS multi-agent
//! coordination kernel.
//!
//! It persists a [`Coordinator`] to a JSON store (default
//! `.beater/coordination.json`) so several agents sharing a checkout can claim
//! disjoint work, record independent reviews, and evaluate the merge gate
//! without stepping on each other. All state transitions are journaled and
//! hash-chained by the library; this binary is a thin, dependency-light shell.

use std::collections::{BTreeMap, BTreeSet};
use std::path::{Path, PathBuf};
use std::process::ExitCode;

use beater_os_coordination::{
    AgentPrincipal, ClaimInput, ClaimStatus, Coordinator, MergePolicy, ReviewInput, ReviewVerdict,
    WriteScope,
};
use chrono::Utc;

const DEFAULT_STORE: &str = ".beater/coordination.json";

fn main() -> ExitCode {
    match run() {
        Ok(()) => ExitCode::SUCCESS,
        Err(err) => {
            eprintln!("error: {err}");
            ExitCode::FAILURE
        }
    }
}

fn run() -> Result<(), String> {
    let raw: Vec<String> = std::env::args().skip(1).collect();
    let parsed = Args::parse(&raw)?;
    let Some(command) = parsed.command.as_deref() else {
        print_usage();
        return Ok(());
    };
    let store_path = PathBuf::from(parsed.value_or("store", DEFAULT_STORE));

    match command {
        "help" | "--help" | "-h" => {
            print_usage();
            Ok(())
        }
        "init" => cmd_init(&store_path, &parsed),
        "register" => cmd_register(&store_path, &parsed),
        "claim" => cmd_claim(&store_path, &parsed),
        "status" => cmd_status(&store_path, &parsed),
        "release" => cmd_release(&store_path, &parsed),
        "review" => cmd_review(&store_path, &parsed),
        "gate" => cmd_gate(&store_path, &parsed),
        "merge" => cmd_merge(&store_path, &parsed),
        "list" => cmd_list(&store_path),
        "conflicts" => cmd_conflicts(&store_path),
        "journal" => cmd_journal(&store_path),
        "verify" => cmd_verify(&store_path),
        other => Err(format!(
            "unknown command '{other}'. Run `beater-coord help` for usage."
        )),
    }
}

fn cmd_init(path: &Path, args: &Args) -> Result<(), String> {
    if path.exists() {
        return Err(format!("store already exists at {}", path.display()));
    }
    let policy_version = args.value_or("policy-version", "coord-policy-v1");
    let mut policy = MergePolicy::default();
    if let Some(min) = args.opt("min-approvals") {
        let parsed: usize = min
            .parse()
            .map_err(|_| format!("invalid --min-approvals '{min}'"))?;
        if parsed == 0 {
            return Err(
                "--min-approvals must be at least 1 (0 would disable independent review)"
                    .to_string(),
            );
        }
        policy.min_independent_approvals = parsed;
    }
    let coord = Coordinator::with_policy(policy_version, policy);
    save_store(path, &coord)?;
    println!("initialized coordination store at {}", path.display());
    Ok(())
}

fn cmd_register(path: &Path, args: &Args) -> Result<(), String> {
    let mut coord = load_store(path)?;
    let id = args.require("id")?;
    let name = args.value_or("name", id);
    let mut principal = if args.flag("human") {
        AgentPrincipal::human(id, name)
    } else {
        AgentPrincipal::agent(id, name)
    };
    if let Some(role) = args.opt("role") {
        principal = principal.with_role(role);
    }
    coord
        .register_principal(principal, Utc::now())
        .map_err(|e| e.to_string())?;
    save_store(path, &coord)?;
    println!("registered principal {id}");
    Ok(())
}

fn cmd_claim(path: &Path, args: &Args) -> Result<(), String> {
    let mut coord = load_store(path)?;
    let slice = args.require("slice")?;
    let input = ClaimInput {
        claim_id: None,
        slice_id: slice.to_string(),
        claimant: args.require("by")?.to_string(),
        branch: args.require("branch")?.to_string(),
        write_scope: WriteScope::new(split_list(args.require("scope")?)),
        depends_on: split_list(args.value_or("depends", ""))
            .into_iter()
            .collect(),
        reason: args.value_or("reason", "").to_string(),
    };
    let claim = coord
        .claim_slice(input, Utc::now())
        .map_err(|e| e.to_string())?;
    save_store(path, &coord)?;
    println!(
        "claimed slice {} on branch {} for {} (scope: {})",
        claim.slice_id,
        claim.branch,
        claim.claimant,
        claim
            .write_scope
            .prefixes
            .iter()
            .cloned()
            .collect::<Vec<_>>()
            .join(", ")
    );
    Ok(())
}

fn cmd_status(path: &Path, args: &Args) -> Result<(), String> {
    let mut coord = load_store(path)?;
    let slice = args.require("slice")?;
    let to = parse_status(args.require("to")?)?;
    coord
        .set_status(slice, to, Utc::now())
        .map_err(|e| e.to_string())?;
    save_store(path, &coord)?;
    println!("slice {slice} -> {to}");
    Ok(())
}

fn cmd_release(path: &Path, args: &Args) -> Result<(), String> {
    let mut coord = load_store(path)?;
    let slice = args.require("slice")?;
    coord
        .release_claim(slice, args.value_or("reason", "released"), Utc::now())
        .map_err(|e| e.to_string())?;
    save_store(path, &coord)?;
    println!("released slice {slice}");
    Ok(())
}

fn cmd_review(path: &Path, args: &Args) -> Result<(), String> {
    let mut coord = load_store(path)?;
    let slice = args.require("slice")?;
    let input = ReviewInput {
        review_id: None,
        slice_id: slice.to_string(),
        subject_ref: args.value_or("subject", slice).to_string(),
        commit_sha: args.require("commit")?.to_string(),
        reviewer_id: args.require("by")?.to_string(),
        author_id: String::new(), // bound to the claimant by the coordinator
        verdict: parse_verdict(args.require("verdict")?)?,
        summary: args.value_or("summary", "").to_string(),
        checklist: Vec::new(),
        policy_version: String::new(), // set by the coordinator
    };
    let review = coord
        .submit_review(input, Utc::now())
        .map_err(|e| e.to_string())?;
    save_store(path, &coord)?;
    println!(
        "recorded {:?} review of slice {} by {} at {}",
        review.verdict, review.slice_id, review.reviewer_id, review.commit_sha
    );
    Ok(())
}

fn cmd_gate(path: &Path, args: &Args) -> Result<(), String> {
    let mut coord = load_store(path)?;
    let slice = args.require("slice")?;
    let merger = args.require("merger")?;
    let commit = args.require("commit")?;
    let ci_green = resolve_ci(args)?;
    let decision = coord
        .evaluate_merge(slice, merger, commit, ci_green, Utc::now())
        .map_err(|e| e.to_string())?;
    save_store(path, &coord)?;

    println!("merge gate for slice {slice} @ {commit}");
    println!("  result: {:?}", decision.result);
    println!("  decision_id: {}", decision.decision_id);
    println!(
        "  independent approvals: {} [{}]",
        decision.independent_approvals,
        decision.approving_reviewers.join(", ")
    );
    if !decision.matched_rules.is_empty() {
        println!("  passed: {}", decision.matched_rules.join(", "));
    }
    if decision.is_allowed() {
        println!(
            "  -> authorized. Merge with: beater-coord merge --slice {slice} --merger {merger} --decision {} --commit {commit}",
            decision.decision_id
        );
    } else {
        println!("  blocked:");
        for reason in &decision.blocking_reasons {
            println!("    - {reason}");
        }
    }
    Ok(())
}

fn cmd_merge(path: &Path, args: &Args) -> Result<(), String> {
    let mut coord = load_store(path)?;
    let slice = args.require("slice")?;
    let merger = args.require("merger")?;
    let decision = args.require("decision")?;
    let commit = args.require("commit")?;
    coord
        .mark_merged(slice, merger, decision, commit, Utc::now())
        .map_err(|e| e.to_string())?;
    save_store(path, &coord)?;
    println!("slice {slice} marked merged by {merger} at {commit}");
    Ok(())
}

fn cmd_list(path: &Path) -> Result<(), String> {
    let coord = load_store(path)?;
    println!("policy_version: {}", coord.policy_version());
    let policy = coord.merge_policy();
    println!(
        "merge_policy: min_approvals={} ci_green={} deps_merged={}",
        policy.min_independent_approvals,
        policy.require_ci_green,
        policy.require_dependencies_merged
    );
    println!("principals:");
    for p in coord.principals() {
        println!("  - {} ({:?}) {}", p.principal_id, p.kind, p.display_name);
    }
    println!("claims:");
    for claim in coord.claims() {
        let deps = if claim.depends_on.is_empty() {
            String::new()
        } else {
            format!(
                " depends_on=[{}]",
                claim
                    .depends_on
                    .iter()
                    .cloned()
                    .collect::<Vec<_>>()
                    .join(", ")
            )
        };
        println!(
            "  - {} [{}] by {} on {}{}",
            claim.slice_id, claim.status, claim.claimant, claim.branch, deps
        );
    }
    Ok(())
}

fn cmd_conflicts(path: &Path) -> Result<(), String> {
    let coord = load_store(path)?;
    let conflicts = coord.conflicts();
    if conflicts.is_empty() {
        println!("no write-scope conflicts among active claims");
        return Ok(());
    }
    println!("write-scope conflicts:");
    for (a, b, pa, pb) in conflicts {
        println!("  - {a} ({pa}) overlaps {b} ({pb})");
    }
    Ok(())
}

fn cmd_journal(path: &Path) -> Result<(), String> {
    let coord = load_store(path)?;
    for record in coord.ledger().records() {
        let event = serde_json::to_string(&record.event).map_err(|e| e.to_string())?;
        println!(
            "{:>4} {} {}",
            record.seq,
            &record.hash[..12.min(record.hash.len())],
            event
        );
    }
    println!("root_hash: {}", coord.ledger().root_hash());
    Ok(())
}

fn cmd_verify(path: &Path) -> Result<(), String> {
    let coord = load_store(path)?;
    coord.verify().map_err(|e| e.to_string())?;
    println!(
        "coordination ledger verified: {} records, root {}",
        coord.ledger().records().len(),
        coord.ledger().root_hash()
    );
    Ok(())
}

// ---------------------------------------------------------------------------
// storage
// ---------------------------------------------------------------------------

fn load_store(path: &Path) -> Result<Coordinator, String> {
    let bytes = std::fs::read(path).map_err(|e| {
        format!(
            "cannot read store {}: {e}. Run `beater-coord init` first.",
            path.display()
        )
    })?;
    serde_json::from_slice(&bytes).map_err(|e| format!("corrupt store {}: {e}", path.display()))
}

fn save_store(path: &Path, coord: &Coordinator) -> Result<(), String> {
    if let Some(parent) = path.parent()
        && !parent.as_os_str().is_empty()
    {
        std::fs::create_dir_all(parent)
            .map_err(|e| format!("cannot create {}: {e}", parent.display()))?;
    }
    let json =
        serde_json::to_vec_pretty(coord).map_err(|e| format!("cannot serialize store: {e}"))?;
    std::fs::write(path, json).map_err(|e| format!("cannot write {}: {e}", path.display()))
}

// ---------------------------------------------------------------------------
// parsing helpers
// ---------------------------------------------------------------------------

fn split_list(value: &str) -> Vec<String> {
    value
        .split(',')
        .map(str::trim)
        .filter(|s| !s.is_empty())
        .map(str::to_string)
        .collect()
}

fn parse_status(value: &str) -> Result<ClaimStatus, String> {
    match value {
        "claimed" => Ok(ClaimStatus::Claimed),
        "in_review" | "in-review" => Ok(ClaimStatus::InReview),
        "approved" => Ok(ClaimStatus::Approved),
        "merged" => Ok(ClaimStatus::Merged),
        "released" => Ok(ClaimStatus::Released),
        other => Err(format!(
            "invalid status '{other}' (claimed|in_review|approved|merged|released)"
        )),
    }
}

fn parse_verdict(value: &str) -> Result<ReviewVerdict, String> {
    match value {
        "approve" | "approved" => Ok(ReviewVerdict::Approve),
        "request_changes" | "request-changes" | "changes" => Ok(ReviewVerdict::RequestChanges),
        "reject" | "rejected" => Ok(ReviewVerdict::Reject),
        other => Err(format!(
            "invalid verdict '{other}' (approve|request_changes|reject)"
        )),
    }
}

fn resolve_ci(args: &Args) -> Result<bool, String> {
    match (args.flag("ci-green"), args.flag("ci-red")) {
        (true, true) => Err("--ci-green and --ci-red are mutually exclusive".to_string()),
        (true, false) => Ok(true),
        (false, true) => Ok(false),
        (false, false) => Ok(false), // fail closed: CI is not green unless asserted
    }
}

/// Minimal `--key value` / `--flag` parser (no external dependency).
struct Args {
    command: Option<String>,
    flags: BTreeMap<String, String>,
    bools: BTreeSet<String>,
}

/// Flags that take no value.
const BOOL_FLAGS: &[&str] = &["human", "ci-green", "ci-red"];

impl Args {
    fn parse(raw: &[String]) -> Result<Self, String> {
        let mut command = None;
        let mut flags = BTreeMap::new();
        let mut bools = BTreeSet::new();
        let mut idx = 0;
        while idx < raw.len() {
            let token = &raw[idx];
            if let Some(key) = token.strip_prefix("--") {
                if BOOL_FLAGS.contains(&key) {
                    bools.insert(key.to_string());
                    idx += 1;
                } else {
                    let value = raw
                        .get(idx + 1)
                        .ok_or_else(|| format!("flag --{key} expects a value"))?;
                    flags.insert(key.to_string(), value.clone());
                    idx += 2;
                }
            } else {
                if command.is_none() {
                    command = Some(token.clone());
                }
                idx += 1;
            }
        }
        Ok(Self {
            command,
            flags,
            bools,
        })
    }

    fn opt(&self, key: &str) -> Option<&str> {
        self.flags.get(key).map(String::as_str)
    }

    fn value_or<'a>(&'a self, key: &str, default: &'a str) -> &'a str {
        self.opt(key).unwrap_or(default)
    }

    fn require(&self, key: &str) -> Result<&str, String> {
        self.opt(key)
            .ok_or_else(|| format!("missing required --{key}"))
    }

    fn flag(&self, key: &str) -> bool {
        self.bools.contains(key)
    }
}

fn print_usage() {
    println!(
        "beater-coord — beaterOS multi-agent coordination kernel

USAGE:
  beater-coord <command> [--store <path>] [flags]

COMMANDS:
  init        --policy-version <v> [--min-approvals <n>]
  register    --id <id> [--name <n>] [--human] [--role <r>]
  claim       --slice <id> --by <principal> --branch <b> --scope <p1,p2,..>
              [--depends <s1,s2,..>] [--reason <text>]
  status      --slice <id> --to <claimed|in_review|approved|merged|released>
  release     --slice <id> [--reason <text>]
  review      --slice <id> --by <reviewer> --commit <sha>
              --verdict <approve|request_changes|reject> [--subject <ref>] [--summary <text>]
  gate        --slice <id> --merger <id> --commit <sha> [--ci-green|--ci-red]
  merge       --slice <id> --merger <id> --decision <decision_id> --commit <sha>
  list        show principals and claims
  conflicts   show write-scope conflicts among active claims
  journal     dump the hash-chained coordination ledger
  verify      verify the ledger hash chain

Default store: {DEFAULT_STORE}"
    );
}
