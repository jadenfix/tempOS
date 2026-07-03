use beater_os_coordination::{
    AgentPrincipal, ClaimInput, ClaimStatus, CoordinationError, CoordinationEvent, Coordinator,
    MergeGateDecision, MergeGateResult, MergePolicy, ReviewInput, ReviewVerdict, WriteScope,
};
use chrono::{DateTime, Duration, TimeZone, Utc};

// ---------------------------------------------------------------------------
// helpers (no unwrap/expect: the workspace denies both clippy lints)
// ---------------------------------------------------------------------------

fn t0() -> DateTime<Utc> {
    Utc.with_ymd_and_hms(2026, 7, 3, 12, 0, 0)
        .single()
        .unwrap_or_else(Utc::now)
}

fn at(mins: i64) -> DateTime<Utc> {
    t0() + Duration::minutes(mins)
}

/// Unwrap a coordination result or fail the test with a message.
#[track_caller]
fn ok<T>(result: Result<T, CoordinationError>) -> T {
    result.unwrap_or_else(|err| panic!("expected Ok, got error: {err}"))
}

/// Assert a coordination result is an error and return it.
#[track_caller]
fn err<T>(result: Result<T, CoordinationError>) -> CoordinationError {
    match result {
        Ok(_) => panic!("expected an error, got Ok"),
        Err(e) => e,
    }
}

/// A coordinator with `codex` and `claude` registered as agents.
fn coord_with_two_agents() -> Coordinator {
    let mut coord = Coordinator::new("coord-policy-v1");
    ok(coord.register_principal(AgentPrincipal::agent("codex", "Codex agent"), at(0)));
    ok(coord.register_principal(AgentPrincipal::agent("claude", "Claude agent"), at(0)));
    coord
}

fn claim_input(slice: &str, by: &str, branch: &str, scope: &[&str]) -> ClaimInput {
    ClaimInput {
        claim_id: None,
        slice_id: slice.to_string(),
        claimant: by.to_string(),
        branch: branch.to_string(),
        write_scope: WriteScope::new(scope.iter().map(|s| s.to_string())),
        depends_on: Default::default(),
        reason: "test slice".to_string(),
    }
}

fn review_input(slice: &str, reviewer: &str, commit: &str, verdict: ReviewVerdict) -> ReviewInput {
    ReviewInput {
        review_id: None,
        slice_id: slice.to_string(),
        subject_ref: format!("pr:{slice}"),
        commit_sha: commit.to_string(),
        reviewer_id: reviewer.to_string(),
        author_id: String::new(),
        verdict,
        summary: "reviewed".to_string(),
        checklist: Vec::new(),
        policy_version: String::new(),
    }
}

/// Claim a `crates/core/` slice and move it to `InReview`.
fn slice_in_review(coord: &mut Coordinator, slice: &str, author: &str) {
    ok(coord.claim_slice(
        claim_input(
            slice,
            author,
            &format!("{author}/{slice}"),
            &["crates/core/"],
        ),
        at(1),
    ));
    ok(coord.set_status(slice, ClaimStatus::InReview, at(2)));
}

// ---------------------------------------------------------------------------
// write-scope disjointness
// ---------------------------------------------------------------------------

#[test]
fn write_scope_directory_overlaps_are_detected() {
    let a = WriteScope::new(["crates/x/"]);
    let nested = WriteScope::new(["crates/x/src/lib.rs"]);
    let sibling = WriteScope::new(["crates/xyz/"]);
    let unrelated = WriteScope::new(["docs/"]);

    assert!(!a.is_disjoint(&nested), "nested path must overlap");
    assert!(
        a.is_disjoint(&sibling),
        "sibling dir sharing a textual prefix must not overlap"
    );
    assert!(a.is_disjoint(&unrelated));
    assert!(!a.is_disjoint(&a), "identical scope overlaps itself");
}

#[test]
fn write_scope_normalizes_redundant_segments() {
    // `//`, `/./`, and a leading `./` must canonicalize to the same path so
    // they cannot be used to dodge overlap detection.
    let dir = WriteScope::new(["crates/x/"]);
    for spelling in ["crates//x/mod.rs", "crates/x/./mod.rs", "./crates/x/mod.rs"] {
        let other = WriteScope::new([spelling]);
        assert!(
            !dir.is_disjoint(&other),
            "{spelling} must overlap crates/x/"
        );
    }
}

#[test]
fn overlapping_claims_via_redundant_slashes_are_rejected() {
    let mut coord = coord_with_two_agents();
    ok(coord.claim_slice(claim_input("a", "codex", "codex/a", &["crates/x/"]), at(1)));
    // Double-slash spelling of a path inside crates/x/ must still conflict.
    let e = err(coord.claim_slice(
        claim_input("b", "claude", "claude/b", &["crates//x/mod.rs"]),
        at(2),
    ));
    assert!(matches!(e, CoordinationError::WriteScopeConflict { .. }));
}

#[test]
fn write_scope_exact_file_conflicts() {
    let a = WriteScope::new(["Cargo.toml"]);
    let b = WriteScope::new(["Cargo.toml"]);
    let c = WriteScope::new(["Cargo.lock"]);
    assert!(!a.is_disjoint(&b));
    assert!(a.is_disjoint(&c));
}

// ---------------------------------------------------------------------------
// claiming
// ---------------------------------------------------------------------------

#[test]
fn claim_requires_registered_principal() {
    let mut coord = Coordinator::new("v1");
    let e = err(coord.claim_slice(claim_input("s1", "ghost", "b1", &["crates/a/"]), at(1)));
    assert!(matches!(e, CoordinationError::UnknownPrincipal { .. }));
}

#[test]
fn overlapping_claims_are_rejected_and_journaled() {
    let mut coord = coord_with_two_agents();
    ok(coord.claim_slice(
        claim_input("core", "codex", "codex/core", &["crates/core/"]),
        at(1),
    ));
    let e = err(coord.claim_slice(
        claim_input("core2", "claude", "claude/core2", &["crates/core/src/"]),
        at(2),
    ));
    assert!(matches!(e, CoordinationError::WriteScopeConflict { .. }));

    let has_conflict = coord
        .ledger()
        .records()
        .iter()
        .any(|r| matches!(&r.event, CoordinationEvent::ConflictDetected { .. }));
    assert!(
        has_conflict,
        "conflict must be journaled for other agents to see"
    );
}

#[test]
fn disjoint_claims_coexist() {
    let mut coord = coord_with_two_agents();
    ok(coord.claim_slice(
        claim_input("core", "codex", "codex/core", &["crates/core/"]),
        at(1),
    ));
    ok(coord.claim_slice(
        claim_input(
            "coord",
            "claude",
            "claude/coord",
            &["crates/coord/", "docs/coord.md"],
        ),
        at(2),
    ));
    assert_eq!(coord.active_claims().count(), 2);
    assert!(coord.conflicts().is_empty());
}

#[test]
fn branch_cannot_be_reused_by_active_claim() {
    let mut coord = coord_with_two_agents();
    ok(coord.claim_slice(
        claim_input("core", "codex", "shared", &["crates/core/"]),
        at(1),
    ));
    let e = err(coord.claim_slice(
        claim_input("coord", "claude", "shared", &["crates/coord/"]),
        at(2),
    ));
    assert!(matches!(e, CoordinationError::BranchAlreadyClaimed { .. }));
}

#[test]
fn empty_and_escaping_scopes_are_rejected() {
    let mut coord = coord_with_two_agents();
    let empty = err(coord.claim_slice(claim_input("s", "codex", "b", &[]), at(1)));
    assert!(matches!(empty, CoordinationError::EmptyWriteScope { .. }));

    let escaping = err(coord.claim_slice(claim_input("s", "codex", "b", &["../secrets/"]), at(1)));
    assert!(matches!(
        escaping,
        CoordinationError::InvalidWriteScopePrefix { .. }
    ));
}

#[test]
fn released_scope_can_be_reclaimed() {
    let mut coord = coord_with_two_agents();
    ok(coord.claim_slice(
        claim_input("core", "codex", "codex/core", &["crates/core/"]),
        at(1),
    ));
    ok(coord.release_claim("core", "abandoned", at(2)));
    // Same scope is free for another agent once released.
    ok(coord.claim_slice(
        claim_input("core-redo", "claude", "claude/core", &["crates/core/"]),
        at(3),
    ));
}

// ---------------------------------------------------------------------------
// review independence
// ---------------------------------------------------------------------------

#[test]
fn author_cannot_review_own_slice() {
    let mut coord = coord_with_two_agents();
    ok(coord.claim_slice(
        claim_input("core", "codex", "codex/core", &["crates/core/"]),
        at(1),
    ));
    let e = err(coord.submit_review(
        review_input("core", "codex", "sha1", ReviewVerdict::Approve),
        at(2),
    ));
    assert!(matches!(e, CoordinationError::SelfReview { .. }));
}

#[test]
fn reviewer_must_be_registered() {
    let mut coord = coord_with_two_agents();
    ok(coord.claim_slice(
        claim_input("core", "codex", "codex/core", &["crates/core/"]),
        at(1),
    ));
    let e = err(coord.submit_review(
        review_input("core", "stranger", "sha1", ReviewVerdict::Approve),
        at(2),
    ));
    assert!(matches!(e, CoordinationError::UnknownPrincipal { .. }));
}

#[test]
fn review_author_is_bound_to_claimant() {
    let mut coord = coord_with_two_agents();
    ok(coord.claim_slice(
        claim_input("core", "codex", "codex/core", &["crates/core/"]),
        at(1),
    ));
    let review = ok(coord.submit_review(
        review_input("core", "claude", "sha1", ReviewVerdict::Approve),
        at(2),
    ));
    assert_eq!(
        review.author_id, "codex",
        "author is bound to the claimant, not the input"
    );
    assert_eq!(review.policy_version, "coord-policy-v1");
}

// ---------------------------------------------------------------------------
// merge gate
// ---------------------------------------------------------------------------

#[test]
fn full_happy_path_allows_and_merges() {
    let mut coord = coord_with_two_agents();
    slice_in_review(&mut coord, "core", "codex");
    ok(coord.submit_review(
        review_input("core", "claude", "sha-final", ReviewVerdict::Approve),
        at(3),
    ));

    let decision = ok(coord.evaluate_merge("core", "claude", "sha-final", true, at(4)));
    assert_eq!(
        decision.result,
        MergeGateResult::Allowed,
        "{:?}",
        decision.blocking_reasons
    );
    assert_eq!(decision.independent_approvals, 1);
    assert_eq!(
        coord.claim("core").map(|c| c.status),
        Some(ClaimStatus::Approved)
    );

    ok(coord.mark_merged("core", "claude", &decision.decision_id, "sha-final", at(5)));
    assert_eq!(
        coord.claim("core").map(|c| c.status),
        Some(ClaimStatus::Merged)
    );
}

#[test]
fn merge_authorization_is_bound_to_the_reviewed_commit() {
    let mut coord = coord_with_two_agents();
    slice_in_review(&mut coord, "core", "codex");
    ok(coord.submit_review(
        review_input("core", "claude", "sha-reviewed", ReviewVerdict::Approve),
        at(3),
    ));
    let decision = ok(coord.evaluate_merge("core", "claude", "sha-reviewed", true, at(4)));
    assert_eq!(decision.result, MergeGateResult::Allowed);

    // Merging a DIFFERENT (unreviewed) commit with the same decision id must
    // fail: authorization is bound to the reviewed commit.
    let e = err(coord.mark_merged(
        "core",
        "claude",
        &decision.decision_id,
        "sha-unreviewed",
        at(5),
    ));
    assert!(matches!(e, CoordinationError::MergeNotAuthorized { .. }));
    assert_eq!(
        coord.claim("core").map(|c| c.status),
        Some(ClaimStatus::Approved)
    );

    // Merging the exact reviewed commit succeeds.
    ok(coord.mark_merged(
        "core",
        "claude",
        &decision.decision_id,
        "sha-reviewed",
        at(6),
    ));
    assert_eq!(
        coord.claim("core").map(|c| c.status),
        Some(ClaimStatus::Merged)
    );
}

#[test]
fn approval_is_invalidated_when_claim_returns_to_review() {
    let mut coord = coord_with_two_agents();
    slice_in_review(&mut coord, "core", "codex");
    ok(coord.submit_review(
        review_input("core", "claude", "sha1", ReviewVerdict::Approve),
        at(3),
    ));
    let decision = ok(coord.evaluate_merge("core", "claude", "sha1", true, at(4)));
    assert_eq!(decision.result, MergeGateResult::Allowed);

    // A new commit arrives: back to InReview (clears the approval), then back to
    // Approved WITHOUT a fresh gate.
    ok(coord.set_status("core", ClaimStatus::InReview, at(5)));
    ok(coord.set_status("core", ClaimStatus::Approved, at(6)));

    // The stale decision must not merge: approved_commit was cleared.
    let e = err(coord.mark_merged("core", "claude", &decision.decision_id, "sha1", at(7)));
    assert!(matches!(e, CoordinationError::MergeNotAuthorized { .. }));
}

#[test]
fn author_cannot_merge_own_slice() {
    let mut coord = coord_with_two_agents();
    slice_in_review(&mut coord, "core", "codex");
    ok(coord.submit_review(
        review_input("core", "claude", "sha1", ReviewVerdict::Approve),
        at(3),
    ));
    let decision = ok(coord.evaluate_merge("core", "codex", "sha1", true, at(4)));
    assert_eq!(decision.result, MergeGateResult::Denied);
    assert!(
        decision
            .blocking_reasons
            .iter()
            .any(|r| r.contains("self-merge")),
        "{:?}",
        decision.blocking_reasons
    );
}

#[test]
fn merge_blocked_without_independent_approval() {
    let mut coord = coord_with_two_agents();
    slice_in_review(&mut coord, "core", "codex");
    let decision = ok(coord.evaluate_merge("core", "claude", "sha1", true, at(4)));
    assert_eq!(decision.result, MergeGateResult::Denied);
    assert_eq!(decision.independent_approvals, 0);
}

#[test]
fn request_changes_and_reject_block_merge() {
    for verdict in [ReviewVerdict::RequestChanges, ReviewVerdict::Reject] {
        let mut coord = coord_with_two_agents();
        slice_in_review(&mut coord, "core", "codex");
        ok(coord.submit_review(review_input("core", "claude", "sha1", verdict), at(3)));
        let decision = ok(coord.evaluate_merge("core", "claude", "sha1", true, at(4)));
        assert_eq!(
            decision.result,
            MergeGateResult::Denied,
            "verdict {verdict:?} must block"
        );
    }
}

#[test]
fn ci_red_blocks_merge() {
    let mut coord = coord_with_two_agents();
    slice_in_review(&mut coord, "core", "codex");
    ok(coord.submit_review(
        review_input("core", "claude", "sha1", ReviewVerdict::Approve),
        at(3),
    ));
    let decision = ok(coord.evaluate_merge("core", "claude", "sha1", false, at(4)));
    assert_eq!(decision.result, MergeGateResult::Denied);
    assert!(decision.blocking_reasons.iter().any(|r| r.contains("CI")));
}

#[test]
fn stale_request_changes_on_old_commit_does_not_block_fixed_commit() {
    let mut coord = coord_with_two_agents();
    slice_in_review(&mut coord, "core", "codex");
    // Changes requested on the old commit...
    ok(coord.submit_review(
        review_input("core", "claude", "sha-old", ReviewVerdict::RequestChanges),
        at(3),
    ));
    // ...then approved on the fixed commit.
    ok(coord.submit_review(
        review_input("core", "claude", "sha-new", ReviewVerdict::Approve),
        at(4),
    ));
    let decision = ok(coord.evaluate_merge("core", "claude", "sha-new", true, at(5)));
    assert_eq!(
        decision.result,
        MergeGateResult::Allowed,
        "{:?}",
        decision.blocking_reasons
    );
}

#[test]
fn dependency_must_be_merged_first() {
    let mut coord = coord_with_two_agents();
    // Dependency slice, claimed but not yet merged.
    ok(coord.claim_slice(
        claim_input("core", "codex", "codex/core", &["crates/core/"]),
        at(1),
    ));
    // Dependent slice declares the dependency.
    let mut dependent = claim_input("coord", "claude", "claude/coord", &["crates/coord/"]);
    dependent.depends_on = ["core".to_string()].into_iter().collect();
    ok(coord.claim_slice(dependent, at(2)));
    ok(coord.set_status("coord", ClaimStatus::InReview, at(3)));
    ok(coord.submit_review(
        review_input("coord", "codex", "sha1", ReviewVerdict::Approve),
        at(4),
    ));

    let blocked = ok(coord.evaluate_merge("coord", "codex", "sha1", true, at(5)));
    assert_eq!(blocked.result, MergeGateResult::Denied);
    assert!(
        blocked
            .blocking_reasons
            .iter()
            .any(|r| r.contains("dependency"))
    );

    // Merge the dependency through the gate, then the dependent clears.
    ok(coord.set_status("core", ClaimStatus::InReview, at(6)));
    ok(coord.submit_review(
        review_input("core", "claude", "sha-core", ReviewVerdict::Approve),
        at(6),
    ));
    let dep_decision: MergeGateDecision =
        ok(coord.evaluate_merge("core", "claude", "sha-core", true, at(7)));
    assert_eq!(
        dep_decision.result,
        MergeGateResult::Allowed,
        "{:?}",
        dep_decision.blocking_reasons
    );
    ok(coord.mark_merged(
        "core",
        "claude",
        &dep_decision.decision_id,
        "sha-core",
        at(8),
    ));

    let cleared = ok(coord.evaluate_merge("coord", "codex", "sha1", true, at(9)));
    assert_eq!(
        cleared.result,
        MergeGateResult::Allowed,
        "{:?}",
        cleared.blocking_reasons
    );
}

#[test]
fn mark_merged_requires_prior_authorizing_gate() {
    let mut coord = coord_with_two_agents();
    slice_in_review(&mut coord, "core", "codex");
    ok(coord.submit_review(
        review_input("core", "claude", "sha1", ReviewVerdict::Approve),
        at(3),
    ));
    // Skip the gate and try to merge directly.
    let e = err(coord.mark_merged("core", "claude", "fabricated-id", "sha1", at(4)));
    assert!(matches!(e, CoordinationError::MergeNotAuthorized { .. }));
}

#[test]
fn min_two_approvals_policy_is_enforced() {
    let mut coord = Coordinator::with_policy(
        "coord-policy-v1",
        MergePolicy {
            min_independent_approvals: 2,
            require_ci_green: true,
            require_dependencies_merged: true,
        },
    );
    for id in ["codex", "claude", "fable"] {
        ok(coord.register_principal(AgentPrincipal::agent(id, id), at(0)));
    }
    slice_in_review(&mut coord, "core", "codex");
    ok(coord.submit_review(
        review_input("core", "claude", "sha1", ReviewVerdict::Approve),
        at(3),
    ));
    let one = ok(coord.evaluate_merge("core", "claude", "sha1", true, at(4)));
    assert_eq!(
        one.result,
        MergeGateResult::Denied,
        "one approval is not enough"
    );

    ok(coord.submit_review(
        review_input("core", "fable", "sha1", ReviewVerdict::Approve),
        at(5),
    ));
    let two = ok(coord.evaluate_merge("core", "fable", "sha1", true, at(6)));
    assert_eq!(
        two.result,
        MergeGateResult::Allowed,
        "{:?}",
        two.blocking_reasons
    );
    assert_eq!(two.independent_approvals, 2);
}

// ---------------------------------------------------------------------------
// ledger integrity
// ---------------------------------------------------------------------------

#[test]
fn ledger_verifies_after_full_flow() {
    let mut coord = coord_with_two_agents();
    slice_in_review(&mut coord, "core", "codex");
    ok(coord.submit_review(
        review_input("core", "claude", "sha1", ReviewVerdict::Approve),
        at(3),
    ));
    let decision = ok(coord.evaluate_merge("core", "claude", "sha1", true, at(4)));
    ok(coord.mark_merged("core", "claude", &decision.decision_id, "sha1", at(5)));
    ok(coord.verify());
}

#[test]
fn tampering_with_the_ledger_is_detected() {
    let mut coord = coord_with_two_agents();
    ok(coord.claim_slice(
        claim_input("core", "codex", "codex/core", &["crates/core/"]),
        at(1),
    ));

    // Serialize, tamper a hashed field, reload, and expect verification to fail.
    let mut value: serde_json::Value =
        serde_json::to_value(&coord).unwrap_or_else(|e| panic!("serialize: {e}"));
    match value.pointer_mut("/ledger/records/0/event/principal/display_name") {
        Some(field) => *field = serde_json::Value::String("tampered".to_string()),
        None => panic!("expected a hashed principal field at seq 0"),
    }
    let tampered: Coordinator =
        serde_json::from_value(value).unwrap_or_else(|e| panic!("reload: {e}"));

    let e = err(tampered.verify());
    assert!(matches!(e, CoordinationError::LedgerChain { .. }));
}

#[test]
fn invalid_status_transition_is_rejected() {
    let mut coord = coord_with_two_agents();
    ok(coord.claim_slice(
        claim_input("core", "codex", "codex/core", &["crates/core/"]),
        at(1),
    ));
    // Claimed -> Merged is not a legal direct transition.
    let e = err(coord.set_status("core", ClaimStatus::Merged, at(2)));
    assert!(matches!(
        e,
        CoordinationError::InvalidClaimTransition { .. }
    ));
}
