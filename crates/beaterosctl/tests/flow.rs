//! End-to-end tests for `beaterosctl` that drive the library exactly as the
//! binary does, against a temporary store. These prove the MVP invariants from
//! `final.md` §24: scoped grants, policy outside the model, journal-before /
//! receipt-after, and a verifiable trace.

// Assertions in tests intentionally use expect/unwrap for concise failures.
#![allow(clippy::expect_used, clippy::unwrap_used)]

use std::path::PathBuf;

use beaterosctl::{CliError, run};
use chrono::{TimeDelta, Utc};
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

    fn child_dir(&self, name: &str) -> String {
        let path = self.path.join(name);
        std::fs::create_dir_all(&path).unwrap();
        std::fs::canonicalize(path).unwrap().display().to_string()
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

fn future_rfc3339() -> String {
    (Utc::now() + TimeDelta::hours(1)).to_rfc3339()
}

fn issue_payment_session_grant_and_mandate(home: &str, session: &str, expires_at: &str) -> String {
    ok(
        home,
        &[
            "session",
            "create",
            "--session",
            session,
            "--agent",
            "agent:runtime",
            "--created-by",
            "human:owner",
            "--workspace",
            "ws-payments",
            "--goal",
            "pay approved vendor",
            "--initial-capability-id",
            "grant-spend",
        ],
    );
    let grant_out = ok(
        home,
        &[
            "grant",
            "issue",
            "--session",
            session,
            "--grant-id",
            "grant-spend",
            "--resource-kind",
            "payment_rail",
            "--resource-id",
            "stablecoin:x402",
            "--actions",
            "spend",
            "--max-risk",
            "critical",
            "--max-data-class",
            "financial",
            "--reason",
            "payment spend grant",
        ],
    );
    let grant_id = grant_out
        .lines()
        .next()
        .and_then(|line| line.strip_prefix("issued grant "))
        .map(str::to_string)
        .expect("grant id");
    let mandate = ok(
        home,
        &[
            "payment-mandate",
            "issue",
            "--session",
            session,
            "--mandate",
            "mandate-spend",
            "--rail",
            "stablecoin:x402",
            "--asset",
            "USDC",
            "--max-minor-units",
            "100",
            "--counterparty-policy",
            "prefix:vendor:",
            "--purpose",
            "vendor payment",
            "--expires-at",
            expires_at,
            "--approval-threshold-minor-units",
            "100",
            "--payment-idempotency-key",
            "pay-once",
            "--adapter",
            "x402",
            "--envelope-format",
            "x402-payment-v1",
        ],
    );
    assert!(
        mandate.contains("issued payment mandate mandate-spend"),
        "{mandate}"
    );
    grant_id
}

fn payment_spend_args<'a>(session: &'a str, grant_id: &'a str) -> Vec<&'a str> {
    vec![
        "payment-spend",
        "propose",
        "--session",
        session,
        "--action-id",
        "act-pay",
        "--mandate",
        "mandate-spend",
        "--grants",
        grant_id,
        "--amount-minor-units",
        "100",
        "--adapter-id",
        "x402",
        "--adapter-version",
        "v1",
        "--counterparty-ref",
        "vendor:runtime",
        "--counterparty-binding-hash",
        "2222222222222222222222222222222222222222222222222222222222222222",
        "--envelope-format",
        "x402-payment-v1",
        "--envelope-hash",
        "3333333333333333333333333333333333333333333333333333333333333333",
        "--summary",
        "pay vendor runtime",
    ]
}

fn admit_payment_action(home: &str, session: &str, grant_id: &str) {
    ok(home, &payment_spend_args(session, grant_id));
    ok(
        home,
        &[
            "simulation",
            "record",
            "--session",
            session,
            "--action",
            "act-pay",
        ],
    );
    ok(home, &payment_spend_args(session, grant_id));
}

#[test]
fn full_coding_workflow_end_to_end() {
    let home = TempHome::new();
    let h = home.as_str();
    let repo = home.child_dir("repo");
    let main_rs = PathBuf::from(&repo)
        .join("src")
        .join("main.rs")
        .display()
        .to_string();
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
            &repo,
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
            &main_rs,
            // Kernel-derived resolved target supplied by the mediation point.
            "--resolved-target",
            &main_rs,
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
fn trace_export_emits_schema_shaped_live_bundle() {
    let home = TempHome::new();
    let h = home.as_str();
    let repo = home.child_dir("repo");
    let file = PathBuf::from(&repo).join("README.md").display().to_string();
    let session = "sess-export";

    ok(
        &h,
        &[
            "session",
            "create",
            "--session",
            session,
            "--agent",
            "agent-export",
            "--workspace",
            "ws-export",
            "--goal",
            "export a live trace bundle",
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
            &repo,
            "--reason",
            "trace export test",
        ],
    );
    let grant_id = grant_out
        .lines()
        .next()
        .and_then(|line| line.strip_prefix("issued grant "))
        .expect("grant id in output");
    ok(
        &h,
        &[
            "action",
            "propose",
            "--session",
            session,
            "--tool",
            "tool:beater-os-runtime",
            "--kind",
            "read",
            "--target-kind",
            "file_path",
            "--target",
            &file,
            "--resolved-target",
            &file,
            "--grants",
            grant_id,
            "--action-id",
            "act-export",
            "--summary",
            "read export fixture",
        ],
    );

    let exported = ok(
        &h,
        &[
            "trace",
            "export",
            "--session",
            session,
            "--bundle-id",
            "export-bundle",
            "--description",
            "live export smoke",
        ],
    );
    let json: serde_json::Value =
        serde_json::from_str(&exported).expect("trace export should be JSON");
    assert_eq!(json["bundle_id"], "export-bundle");
    assert_eq!(json["description"], "live export smoke");
    assert_eq!(json["sessions"][0]["session_id"], session);
    assert_eq!(json["grants"].as_array().expect("grants array").len(), 1);
    assert_eq!(
        json["manifests"].as_array().expect("manifests array").len(),
        1
    );
    assert_eq!(
        json["decisions"].as_array().expect("decisions array").len(),
        1
    );
    assert!(json["journal"].as_array().expect("journal array").len() >= 4);
    assert!(
        json["sessions"][0]["memory_scope"].is_null(),
        "trace export should preserve core wire nulls for faithful replay: {exported}"
    );
}

#[test]
fn payment_operator_flow_records_typed_receipt_end_to_end() {
    let home = TempHome::new();
    let h = home.as_str();
    let session = "sess-payment-cli";
    let expires_at = future_rfc3339();
    let grant_id = issue_payment_session_grant_and_mandate(&h, session, &expires_at);

    let first = ok(&h, &payment_spend_args(session, &grant_id));
    assert!(
        first.contains("NeedsSimulation"),
        "payment should pass mandate admission and require simulation first:\n{first}"
    );
    assert!(
        first.contains("payment_authorized_by_mandate"),
        "payment mandate rule must be visible:\n{first}"
    );

    let sim = ok(
        &h,
        &[
            "simulation",
            "record",
            "--session",
            session,
            "--action",
            "act-pay",
        ],
    );
    assert!(sim.contains("recorded simulation"), "{sim}");

    let second = ok(&h, &payment_spend_args(session, &grant_id));
    assert!(
        second.contains("Allowed"),
        "payment should be allowed after action-bound simulation:\n{second}"
    );

    let receipt = ok(
        &h,
        &[
            "receipt",
            "record",
            "--session",
            session,
            "--action",
            "act-pay",
            "--status",
            "submitted",
            "--summary",
            "submitted payment",
            "--rail-receipt-hash",
            "6666666666666666666666666666666666666666666666666666666666666666",
            "--settlement-status",
            "submitted",
            "--external-id",
            "rail:receipt:runtime",
        ],
    );
    assert!(receipt.contains("recorded receipt"), "{receipt}");

    let verify = ok(&h, &["journal", "verify", "--session", session]);
    assert!(verify.contains("journal OK"), "{verify}");
    let trace = ok(&h, &["trace", "show", "--session", session]);
    assert!(trace.contains("payment mandates (1):"), "{trace}");
    assert!(trace.contains("mandate=mandate-spend"), "{trace}");
    assert!(trace.contains("payment receipt:"), "{trace}");
    assert!(
        trace.contains("6666666666666666666666666666666666666666666666666666666666666666"),
        "{trace}"
    );
    let show = ok(&h, &["session", "show", "--session", session]);
    assert!(show.contains("mandates:   1"), "{show}");
}

#[test]
fn payment_spend_without_mandate_is_refused_before_append() {
    let home = TempHome::new();
    let h = home.as_str();
    let session = "sess-payment-no-mandate";
    ok(
        &h,
        &[
            "session",
            "create",
            "--session",
            session,
            "--agent",
            "agent:runtime",
            "--workspace",
            "ws-payments",
            "--goal",
            "pay vendor",
        ],
    );
    let err = cli(&h, &payment_spend_args(session, "grant-spend"))
        .expect_err("payment spend without issued mandate must fail closed");
    assert!(
        matches!(err, CliError::Refused(ref message) if message.contains("has not been issued")),
        "unexpected error: {err}"
    );
    let trace = ok(&h, &["trace", "show", "--session", session]);
    assert!(
        trace.contains("actions (0):"),
        "refused payment should not append an action:\n{trace}"
    );
}

#[test]
fn payment_mandate_requires_explicit_adapter_and_envelope_allowlists() {
    let home = TempHome::new();
    let h = home.as_str();
    let session = "sess-payment-allowlists";
    let expires_at = future_rfc3339();
    ok(
        &h,
        &[
            "session",
            "create",
            "--session",
            session,
            "--agent",
            "agent:runtime",
            "--created-by",
            "human:owner",
            "--workspace",
            "ws-payments",
            "--goal",
            "pay vendor",
        ],
    );

    let missing_adapter = cli(
        &h,
        &[
            "payment-mandate",
            "issue",
            "--session",
            session,
            "--mandate",
            "mandate-spend",
            "--rail",
            "stablecoin:x402",
            "--asset",
            "USDC",
            "--max-minor-units",
            "100",
            "--counterparty-policy",
            "prefix:vendor:",
            "--purpose",
            "vendor payment",
            "--expires-at",
            &expires_at,
            "--payment-idempotency-key",
            "pay-once",
            "--envelope-format",
            "x402-payment-v1",
        ],
    )
    .expect_err("missing adapter allowlist must fail closed");
    assert!(
        matches!(missing_adapter, CliError::MissingFlag(ref flag) if flag == "adapter"),
        "unexpected error: {missing_adapter}"
    );

    let missing_envelope = cli(
        &h,
        &[
            "payment-mandate",
            "issue",
            "--session",
            session,
            "--mandate",
            "mandate-spend",
            "--rail",
            "stablecoin:x402",
            "--asset",
            "USDC",
            "--max-minor-units",
            "100",
            "--counterparty-policy",
            "prefix:vendor:",
            "--purpose",
            "vendor payment",
            "--expires-at",
            &expires_at,
            "--payment-idempotency-key",
            "pay-once",
            "--adapter",
            "x402",
        ],
    )
    .expect_err("missing envelope-format allowlist must fail closed");
    assert!(
        matches!(missing_envelope, CliError::MissingFlag(ref flag) if flag == "envelope-format"),
        "unexpected error: {missing_envelope}"
    );

    let trace = ok(&h, &["trace", "show", "--session", session]);
    assert!(
        trace.contains("payment mandates (0):"),
        "refused mandates should not append authority:\n{trace}"
    );
}

#[test]
fn payment_mandate_rejects_review_threshold_above_ceiling() {
    let home = TempHome::new();
    let h = home.as_str();
    let session = "sess-payment-threshold";
    let expires_at = future_rfc3339();
    ok(
        &h,
        &[
            "session",
            "create",
            "--session",
            session,
            "--agent",
            "agent:runtime",
            "--created-by",
            "human:owner",
            "--workspace",
            "ws-payments",
            "--goal",
            "pay vendor",
        ],
    );

    let err = cli(
        &h,
        &[
            "payment-mandate",
            "issue",
            "--session",
            session,
            "--mandate",
            "mandate-spend",
            "--rail",
            "stablecoin:x402",
            "--asset",
            "USDC",
            "--max-minor-units",
            "100",
            "--approval-threshold-minor-units",
            "10000",
            "--counterparty-policy",
            "prefix:vendor:",
            "--purpose",
            "vendor payment",
            "--expires-at",
            &expires_at,
            "--payment-idempotency-key",
            "pay-once",
            "--adapter",
            "x402",
            "--envelope-format",
            "x402-payment-v1",
        ],
    )
    .expect_err("approval threshold above mandate ceiling must fail closed");
    assert!(
        matches!(err, CliError::Refused(ref message) if message.contains("exceeds ceiling")),
        "unexpected error: {err}"
    );
}

#[test]
fn payment_receipt_external_id_only_is_refused_and_log_stays_verifiable() {
    let home = TempHome::new();
    let h = home.as_str();
    let session = "sess-payment-external-id-only";
    let expires_at = future_rfc3339();
    let grant_id = issue_payment_session_grant_and_mandate(&h, session, &expires_at);
    admit_payment_action(&h, session, &grant_id);

    let err = cli(
        &h,
        &[
            "receipt",
            "record",
            "--session",
            session,
            "--action",
            "act-pay",
            "--external-id",
            "rail:receipt:external-only",
        ],
    )
    .expect_err("external-id-only payment receipt must not satisfy required typed evidence");
    assert!(
        matches!(err, CliError::MissingFlag(ref flag) if flag == "settlement-status"),
        "unexpected error: {err}"
    );
    let verify = ok(&h, &["journal", "verify", "--session", session]);
    assert!(verify.contains("journal OK"), "{verify}");
}

#[test]
fn payment_receipt_settled_at_matches_settlement_status() {
    let home = TempHome::new();
    let h = home.as_str();
    let session = "sess-payment-settlement-time";
    let expires_at = future_rfc3339();
    let grant_id = issue_payment_session_grant_and_mandate(&h, session, &expires_at);
    admit_payment_action(&h, session, &grant_id);

    let missing_settled_at = cli(
        &h,
        &[
            "receipt",
            "record",
            "--session",
            session,
            "--action",
            "act-pay",
            "--rail-receipt-hash",
            "6666666666666666666666666666666666666666666666666666666666666666",
            "--settlement-status",
            "settled",
        ],
    )
    .expect_err("settled receipt must include settled-at");
    assert!(
        matches!(missing_settled_at, CliError::MissingFlag(ref flag) if flag == "settled-at"),
        "unexpected error: {missing_settled_at}"
    );

    let submitted_with_settled_at = cli(
        &h,
        &[
            "receipt",
            "record",
            "--session",
            session,
            "--action",
            "act-pay",
            "--rail-receipt-hash",
            "6666666666666666666666666666666666666666666666666666666666666666",
            "--settlement-status",
            "submitted",
            "--settled-at",
            &Utc::now().to_rfc3339(),
        ],
    )
    .expect_err("non-settled receipt must not include settled-at");
    assert!(
        matches!(submitted_with_settled_at, CliError::Refused(ref message) if message.contains("only valid")),
        "unexpected error: {submitted_with_settled_at}"
    );

    let verify = ok(&h, &["journal", "verify", "--session", session]);
    assert!(verify.contains("journal OK"), "{verify}");
}

#[test]
fn simulation_record_before_needs_simulation_is_refused() {
    let home = TempHome::new();
    let h = home.as_str();
    let session = "sess-payment-sim-before";
    let expires_at = future_rfc3339();
    issue_payment_session_grant_and_mandate(&h, session, &expires_at);

    let err = cli(
        &h,
        &[
            "simulation",
            "record",
            "--session",
            session,
            "--action",
            "act-pay",
        ],
    )
    .expect_err("simulation evidence before a proposed action must fail closed");
    assert!(
        matches!(err, CliError::Refused(ref message) if message.contains("was never proposed")),
        "unexpected error: {err}"
    );
}

#[test]
fn paused_session_refuses_payment_mandate_issue() {
    let home = TempHome::new();
    let h = home.as_str();
    let session = "sess-payment-paused";
    let expires_at = future_rfc3339();
    ok(
        &h,
        &[
            "session",
            "create",
            "--session",
            session,
            "--agent",
            "agent:runtime",
            "--created-by",
            "human:owner",
            "--workspace",
            "ws-payments",
            "--goal",
            "pay vendor",
        ],
    );
    ok(&h, &["session", "pause", "--session", session]);
    let err = cli(
        &h,
        &[
            "payment-mandate",
            "issue",
            "--session",
            session,
            "--mandate",
            "mandate-spend",
            "--rail",
            "stablecoin:x402",
            "--asset",
            "USDC",
            "--max-minor-units",
            "100",
            "--counterparty-policy",
            "prefix:vendor:",
            "--purpose",
            "vendor payment",
            "--expires-at",
            &expires_at,
            "--payment-idempotency-key",
            "pay-once",
            "--adapter",
            "x402",
            "--envelope-format",
            "x402-payment-v1",
        ],
    )
    .expect_err("paused session must refuse new payment mandate authority");
    assert!(
        matches!(err, CliError::Runtime(_)),
        "unexpected error: {err}"
    );
}

#[test]
fn file_path_prefix_grant_defaults_to_wildcard_selector_when_resource_id_is_omitted() {
    let home = TempHome::new();
    let h = home.as_str();
    let repo = home.child_dir("repo");
    let repo_file = PathBuf::from(&repo)
        .join("src")
        .join("main.rs")
        .display()
        .to_string();
    let session = "sess-prefix-default";

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
            "edit workspace files",
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
            "--actions",
            "write",
            "--path-prefix",
            &repo,
            "--reason",
            "workspace write",
        ],
    );
    assert!(
        grant_out.contains("scope:   FilePath *"),
        "file_path path-prefix grant should default selector to wildcard:\n{grant_out}"
    );
    let grant_id = grant_out
        .lines()
        .next()
        .and_then(|line| line.strip_prefix("issued grant "))
        .map(str::to_string)
        .expect("grant id in output");

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
            &repo_file,
            "--resolved-target",
            &repo_file,
            "--grants",
            &grant_id,
            "--action-id",
            "act-write-inside",
            "--summary",
            "edit main.rs",
        ],
    );
    assert!(
        allow.contains("Allowed"),
        "in-prefix write should be allowed by wildcard selector plus path-prefix:\n{allow}"
    );

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
            "--resolved-target",
            "/etc/passwd",
            "--grants",
            &grant_id,
            "--action-id",
            "act-write-outside",
        ],
    );
    assert!(
        denied.contains("NeedsNarrowedGrant"),
        "wildcard selector must remain narrowed by the path-prefix constraint:\n{denied}"
    );
}

#[test]
fn grant_issue_without_resource_id_still_fails_without_file_path_prefix() {
    let home = TempHome::new();
    let h = home.as_str();
    let session = "sess-missing-resource-id";

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
            "check grant shape",
        ],
    );

    let missing_file_target = cli(
        &h,
        &[
            "grant",
            "issue",
            "--session",
            session,
            "--resource-kind",
            "file_path",
            "--actions",
            "read",
        ],
    )
    .expect_err("file_path grant without resource-id or path-prefix must fail closed");
    assert!(
        matches!(missing_file_target, CliError::MissingFlag(ref flag) if flag == "resource-id"),
        "unexpected error: {missing_file_target}"
    );

    let missing_network_target = cli(
        &h,
        &[
            "grant",
            "issue",
            "--session",
            session,
            "--resource-kind",
            "network_endpoint",
            "--actions",
            "read",
            "--path-prefix",
            &h,
        ],
    )
    .expect_err("non-file grant without resource-id must fail closed");
    assert!(
        matches!(missing_network_target, CliError::MissingFlag(ref flag) if flag == "resource-id"),
        "unexpected error: {missing_network_target}"
    );
}

#[test]
fn receipt_is_refused_without_admitted_action() {
    let home = TempHome::new();
    let h = home.as_str();
    let repo = home.child_dir("repo");
    let repo_x = PathBuf::from(&repo).join("x").display().to_string();
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
            &repo,
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
            &repo_x,
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
fn unregistered_tool_is_denied_by_cli_admission() {
    let home = TempHome::new();
    let h = home.as_str();
    let session = "sess-tool-registry";

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
            "/repo/x",
            "--actions",
            "write",
        ],
    );
    let grant_id = grant_out
        .lines()
        .next()
        .and_then(|line| line.strip_prefix("issued grant "))
        .map(str::to_string)
        .expect("grant id");

    let denied = ok(
        &h,
        &[
            "action",
            "propose",
            "--session",
            session,
            "--tool",
            "unknown-tool",
            "--kind",
            "write",
            "--target-kind",
            "file_path",
            "--target",
            "/repo/x",
            "--grants",
            &grant_id,
            "--action-id",
            "act-unknown-tool",
            "--side-effects",
            "local_write",
        ],
    );

    assert!(denied.contains("Denied"), "{denied}");
    assert!(denied.contains("not registered"), "{denied}");
}

#[test]
fn registered_deployment_tool_cannot_be_laundered_as_execute_by_cli() {
    let home = TempHome::new();
    let h = home.as_str();
    let session = "sess-tool-launder";

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
            "tool",
            "--resource-id",
            "deployer",
            "--actions",
            "execute",
        ],
    );
    let grant_id = grant_out
        .lines()
        .next()
        .and_then(|line| line.strip_prefix("issued grant "))
        .map(str::to_string)
        .expect("grant id");

    let denied = ok(
        &h,
        &[
            "action",
            "propose",
            "--session",
            session,
            "--tool",
            "deployer",
            "--kind",
            "execute",
            "--target-kind",
            "tool",
            "--target",
            "deployer",
            "--grants",
            &grant_id,
            "--action-id",
            "act-deploy-as-exec",
            "--side-effects",
            "deployment",
            "--idempotency-key",
            "deploy-idem",
        ],
    );

    assert!(denied.contains("Denied"), "{denied}");
    assert!(denied.contains("deploy action kind"), "{denied}");
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
fn session_lifecycle_gates_new_authority_and_admission() {
    let home = TempHome::new();
    let h = home.as_str();
    let repo = home.child_dir("repo");
    let repo_x = PathBuf::from(&repo).join("x").display().to_string();
    let session = "sess-life";

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
    let paused = ok(&h, &["session", "pause", "--session", session]);
    assert!(paused.contains("Paused"), "{paused}");

    let grant_err = cli(
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
            "write",
        ],
    )
    .expect_err("paused session must not receive new grants");
    assert!(
        matches!(grant_err, CliError::Runtime(_)),
        "unexpected grant error: {grant_err}"
    );

    let resumed = ok(&h, &["session", "resume", "--session", session]);
    assert!(resumed.contains("Running"), "{resumed}");
    let grant = ok(
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
            "write",
            "--path-prefix",
            &repo,
        ],
    );
    let grant_id = grant
        .lines()
        .next()
        .and_then(|line| line.strip_prefix("issued grant "))
        .expect("grant id");

    let canceled = ok(&h, &["session", "cancel", "--session", session]);
    assert!(canceled.contains("Canceled"), "{canceled}");
    let admission_err = cli(
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
            &repo_x,
            "--resolved-target",
            &repo_x,
            "--grants",
            grant_id,
            "--action-id",
            "act-after-cancel",
        ],
    )
    .expect_err("canceled session must not admit actions");
    assert!(
        matches!(admission_err, CliError::Runtime(_)),
        "unexpected admission error: {admission_err}"
    );
}

#[test]
fn help_is_available() {
    let home = TempHome::new();
    let out = ok(&home.as_str(), &["help"]);
    assert!(out.contains("beaterosctl"));
    assert!(out.contains("session create"));
    assert!(out.contains("--revocation-handle <h>"));
    assert!(out.contains("--revoked-handle <h>"));
}

#[test]
fn action_propose_honors_revoked_handle_registry() {
    let home = TempHome::new();
    let h = home.as_str();
    let session = "sess-revoked-handle";

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
            "/repo/a",
            "--actions",
            "write",
            "--revocation-handle",
            "revoke:repo-a",
        ],
    );
    assert!(
        grant_out.contains("revokes: revoke:repo-a"),
        "grant output should expose the revocation handle:\n{grant_out}"
    );
    let grant_id = grant_out
        .lines()
        .next()
        .and_then(|line| line.strip_prefix("issued grant "))
        .map(str::to_string)
        .expect("grant id");

    let allowed = ok(
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
            &grant_id,
            "--action-id",
            "act-before-revoke",
        ],
    );
    assert!(allowed.contains("Allowed"), "{allowed}");

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
            "/repo/a",
            "--grants",
            &grant_id,
            "--revoked-handle",
            "revoke:repo-a",
            "--action-id",
            "act-after-revoke",
        ],
    );
    assert!(
        denied.contains("Denied"),
        "revoked handle must fail closed at admission:\n{denied}"
    );
    assert!(
        denied.contains("revoked, expired, or missing"),
        "denial should name the revoked authority boundary:\n{denied}"
    );
}

#[test]
fn action_execute_honors_revoked_handle_before_sandbox_execution() {
    let home = TempHome::new();
    let h = home.as_str();
    let repo = home.child_dir("repo");
    let out_file = PathBuf::from(&repo).join("out.txt");
    let session = "sess-execute-revoked-handle";

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
            "execute,write",
            "--path-prefix",
            &repo,
            "--revocation-handle",
            "revoke:exec",
        ],
    );
    let grant_id = grant_out
        .lines()
        .next()
        .and_then(|line| line.strip_prefix("issued grant "))
        .map(str::to_string)
        .expect("grant id");

    let denied = ok(
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
            "printf hi > out.txt",
            "--cwd",
            &repo,
            "--grants",
            &grant_id,
            "--side-effects",
            "local_write",
            "--revoked-handle",
            "revoke:exec",
            "--action-id",
            "act-exec-after-revoke",
        ],
    );

    assert!(
        denied.contains("Denied"),
        "revoked execute grant must fail closed at admission:\n{denied}"
    );
    assert!(
        denied.contains("execution:   skipped"),
        "revoked action must not execute:\n{denied}"
    );
    assert!(
        !out_file.exists(),
        "sandbox command must not run after revoked-handle denial"
    );
    let verify = ok(&h, &["journal", "verify", "--session", session]);
    assert!(verify.contains("journal OK"), "{verify}");
}

/// Set up a session with a write grant and one admitted write action `act-ok`.
/// Returns the temp home so the caller keeps it alive.
fn setup_admitted_write(session: &str) -> TempHome {
    let home = TempHome::new();
    let h = home.as_str();
    let repo = home.child_dir("repo");
    let repo_a = PathBuf::from(&repo).join("a").display().to_string();
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
            &repo,
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
            &repo_a,
            // `resolved_target` is kernel-derived (final.md §7.4): a mediation
            // point supplies the canonical, symlink-resolved path. The CLI no
            // longer infers it from the agent's claimed target, so a path-prefix
            // grant only admits when a resolved target is provided here.
            "--resolved-target",
            &repo_a,
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
    let ws = home.child_dir("ws");
    let ws_file = PathBuf::from(&ws).join("x.txt").display().to_string();
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
            &ws,
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
            &ws_file,
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

#[test]
fn path_prefix_grant_requires_existing_canonical_prefix() {
    let home = TempHome::new();
    let h = home.as_str();
    let missing = home.path.join("missing-prefix").display().to_string();
    ok(
        &h,
        &[
            "session",
            "create",
            "--session",
            "sess-missing-prefix",
            "--agent",
            "a",
            "--workspace",
            "w",
            "--goal",
            "g",
        ],
    );

    let err = cli(
        &h,
        &[
            "grant",
            "issue",
            "--session",
            "sess-missing-prefix",
            "--resource-kind",
            "file_path",
            "--resource-id",
            "*",
            "--actions",
            "read",
            "--path-prefix",
            &missing,
        ],
    )
    .expect_err("missing path-prefix authority must fail closed");
    assert!(
        matches!(err, CliError::Runtime(_) | CliError::Io(_)),
        "unexpected error: {err}"
    );
}
