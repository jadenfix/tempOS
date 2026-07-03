use std::collections::{BTreeMap, BTreeSet};

use chrono::{DateTime, Utc};
use serde::{Deserialize, Serialize};
use uuid::Uuid;

use crate::claim::{ClaimStatus, SliceClaim, WriteScope, is_valid_prefix};
use crate::error::{CoordinationError, CoordinationResult};
use crate::journal::{CoordinationEvent, CoordinationLedger};
use crate::merge::{MergeEvaluation, MergeGateDecision, MergePolicy};
use crate::principal::AgentPrincipal;
use crate::review::{ReviewAttestation, ReviewInput};

/// Fields required to claim a slice. `claim_id` is generated when omitted.
#[derive(Clone, Debug, PartialEq, Eq, Serialize, Deserialize)]
pub struct ClaimInput {
    #[serde(default)]
    pub claim_id: Option<String>,
    pub slice_id: String,
    pub claimant: String,
    pub branch: String,
    pub write_scope: WriteScope,
    #[serde(default)]
    pub depends_on: BTreeSet<String>,
    pub reason: String,
}

/// The multi-agent coordination kernel.
///
/// A `Coordinator` is the single source of truth for who is building what, who
/// reviewed it, and whether it may merge. Every mutating method appends to the
/// tamper-evident [`CoordinationLedger`], so the full history is replayable and
/// verifiable — the same discipline beaterOS applies to agent runs, turned on
/// its own development process.
#[derive(Clone, Debug, PartialEq, Eq, Serialize, Deserialize)]
pub struct Coordinator {
    policy_version: String,
    merge_policy: MergePolicy,
    principals: BTreeMap<String, AgentPrincipal>,
    /// Claims keyed by `slice_id`.
    claims: BTreeMap<String, SliceClaim>,
    reviews: Vec<ReviewAttestation>,
    ledger: CoordinationLedger,
}

impl Coordinator {
    /// Create a coordinator with the default merge policy.
    pub fn new(policy_version: impl Into<String>) -> Self {
        Self::with_policy(policy_version, MergePolicy::default())
    }

    /// Create a coordinator with an explicit merge policy.
    pub fn with_policy(policy_version: impl Into<String>, merge_policy: MergePolicy) -> Self {
        Self {
            policy_version: policy_version.into(),
            merge_policy,
            principals: BTreeMap::new(),
            claims: BTreeMap::new(),
            reviews: Vec::new(),
            ledger: CoordinationLedger::new(),
        }
    }

    pub fn policy_version(&self) -> &str {
        &self.policy_version
    }

    pub fn merge_policy(&self) -> &MergePolicy {
        &self.merge_policy
    }

    pub fn ledger(&self) -> &CoordinationLedger {
        &self.ledger
    }

    pub fn principals(&self) -> impl Iterator<Item = &AgentPrincipal> {
        self.principals.values()
    }

    pub fn claims(&self) -> impl Iterator<Item = &SliceClaim> {
        self.claims.values()
    }

    /// The claim for a slice, if one exists.
    pub fn claim(&self, slice_id: &str) -> Option<&SliceClaim> {
        self.claims.get(slice_id)
    }

    /// Claims that still reserve a write scope (not merged or released).
    pub fn active_claims(&self) -> impl Iterator<Item = &SliceClaim> {
        self.claims
            .values()
            .filter(|claim| claim.status.holds_write_scope())
    }

    /// All reviews recorded for a slice, in submission order.
    pub fn reviews_for(&self, slice_id: &str) -> impl Iterator<Item = &ReviewAttestation> {
        self.reviews.iter().filter(move |r| r.slice_id == slice_id)
    }

    /// Register a principal so it can author, review, and merge slices.
    pub fn register_principal(
        &mut self,
        principal: AgentPrincipal,
        now: DateTime<Utc>,
    ) -> CoordinationResult<()> {
        if self.principals.contains_key(&principal.principal_id) {
            return Err(CoordinationError::DuplicatePrincipal {
                principal_id: principal.principal_id,
            });
        }
        self.principals
            .insert(principal.principal_id.clone(), principal.clone());
        self.ledger
            .append(CoordinationEvent::PrincipalRegistered { principal }, now)?;
        Ok(())
    }

    fn require_principal(&self, principal_id: &str) -> CoordinationResult<()> {
        if self.principals.contains_key(principal_id) {
            Ok(())
        } else {
            Err(CoordinationError::UnknownPrincipal {
                principal_id: principal_id.to_string(),
            })
        }
    }

    fn require_claim(&self, slice_id: &str) -> CoordinationResult<&SliceClaim> {
        self.claims
            .get(slice_id)
            .ok_or_else(|| CoordinationError::UnknownSlice {
                slice_id: slice_id.to_string(),
            })
    }

    /// Claim a slice with a bounded, disjoint write scope.
    ///
    /// Fails closed if the claimant is unregistered, the slice or branch is
    /// already actively claimed, the write scope is empty or malformed, or the
    /// scope overlaps another active claim. Overlaps are journaled as
    /// `ConflictDetected` before the error is returned, so the collision is
    /// visible to every agent watching the ledger.
    pub fn claim_slice(
        &mut self,
        input: ClaimInput,
        now: DateTime<Utc>,
    ) -> CoordinationResult<SliceClaim> {
        let mut input = input;
        self.require_principal(&input.claimant)?;

        // Re-normalize the scope so a deserialized `ClaimInput` cannot smuggle
        // an un-normalized prefix (e.g. `crates//x/`) past overlap detection.
        input.write_scope = WriteScope::new(input.write_scope.prefixes);

        if let Some(existing) = self.claims.get(&input.slice_id)
            && existing.status.holds_write_scope()
        {
            return Err(CoordinationError::SliceAlreadyClaimed {
                slice_id: input.slice_id,
                claimant: existing.claimant.clone(),
            });
        }

        if input.write_scope.is_empty() {
            return Err(CoordinationError::EmptyWriteScope {
                slice_id: input.slice_id,
            });
        }
        for prefix in &input.write_scope.prefixes {
            if !is_valid_prefix(prefix) {
                return Err(CoordinationError::InvalidWriteScopePrefix {
                    slice_id: input.slice_id,
                    prefix: prefix.clone(),
                });
            }
        }

        // Branch uniqueness across active claims.
        for claim in self.active_claims() {
            if claim.slice_id != input.slice_id && claim.branch == input.branch {
                return Err(CoordinationError::BranchAlreadyClaimed {
                    branch: input.branch,
                    slice_id: claim.slice_id.clone(),
                });
            }
        }

        // Write-scope disjointness against every other active claim.
        for claim in self.claims.values() {
            if claim.slice_id == input.slice_id || !claim.status.holds_write_scope() {
                continue;
            }
            if let Some((prefix, other_prefix)) =
                input.write_scope.first_overlap(&claim.write_scope)
            {
                let detail = format!(
                    "prefix {prefix} overlaps {}'s prefix {other_prefix}",
                    claim.slice_id
                );
                self.ledger.append(
                    CoordinationEvent::ConflictDetected {
                        slice_id: input.slice_id.clone(),
                        other_slice_id: claim.slice_id.clone(),
                        detail,
                    },
                    now,
                )?;
                return Err(CoordinationError::WriteScopeConflict {
                    slice_id: input.slice_id,
                    prefix,
                    other_slice_id: claim.slice_id.clone(),
                    other_prefix,
                });
            }
        }

        let claim = SliceClaim {
            claim_id: input.claim_id.unwrap_or_else(|| Uuid::new_v4().to_string()),
            slice_id: input.slice_id.clone(),
            claimant: input.claimant,
            branch: input.branch,
            write_scope: input.write_scope,
            depends_on: input.depends_on,
            status: ClaimStatus::Claimed,
            approved_commit: None,
            reason: input.reason,
            created_at: now,
            updated_at: now,
        };
        self.claims.insert(input.slice_id, claim.clone());
        self.ledger.append(
            CoordinationEvent::SliceClaimed {
                claim: claim.clone(),
            },
            now,
        )?;
        Ok(claim)
    }

    /// Transition a claim to a new status, validating the state machine.
    pub fn set_status(
        &mut self,
        slice_id: &str,
        to: ClaimStatus,
        now: DateTime<Utc>,
    ) -> CoordinationResult<()> {
        let from = self.require_claim(slice_id)?.status;
        if from == to {
            return Ok(());
        }
        if !from.can_transition_to(to) {
            return Err(CoordinationError::InvalidClaimTransition {
                slice_id: slice_id.to_string(),
                from: from.to_string(),
                to: to.to_string(),
            });
        }
        self.apply_status(slice_id, to, now)
    }

    /// Release a claim, freeing its write scope for other agents.
    pub fn release_claim(
        &mut self,
        slice_id: &str,
        reason: impl Into<String>,
        now: DateTime<Utc>,
    ) -> CoordinationResult<()> {
        let from = self.require_claim(slice_id)?.status;
        if matches!(from, ClaimStatus::Merged | ClaimStatus::Released) {
            return Err(CoordinationError::InvalidClaimTransition {
                slice_id: slice_id.to_string(),
                from: from.to_string(),
                to: ClaimStatus::Released.to_string(),
            });
        }
        self.apply_status(slice_id, ClaimStatus::Released, now)?;
        self.ledger.append(
            CoordinationEvent::ClaimReleased {
                slice_id: slice_id.to_string(),
                reason: reason.into(),
            },
            now,
        )?;
        Ok(())
    }

    fn apply_status(
        &mut self,
        slice_id: &str,
        to: ClaimStatus,
        now: DateTime<Utc>,
    ) -> CoordinationResult<()> {
        let from = {
            let claim =
                self.claims
                    .get_mut(slice_id)
                    .ok_or_else(|| CoordinationError::UnknownSlice {
                        slice_id: slice_id.to_string(),
                    })?;
            let from = claim.status;
            claim.status = to;
            claim.updated_at = now;
            // Leaving `Approved` invalidates any commit-bound authorization, so
            // a later re-approval cannot be merged with a stale decision.
            if to != ClaimStatus::Approved {
                claim.approved_commit = None;
            }
            from
        };
        self.ledger.append(
            CoordinationEvent::ClaimStatusChanged {
                slice_id: slice_id.to_string(),
                from,
                to,
            },
            now,
        )?;
        Ok(())
    }

    /// Record an independent review. The author is bound to the claimant, so a
    /// review can never be attributed to the wrong author or pass self-review.
    pub fn submit_review(
        &mut self,
        mut input: ReviewInput,
        now: DateTime<Utc>,
    ) -> CoordinationResult<ReviewAttestation> {
        let claimant = self.require_claim(&input.slice_id)?.claimant.clone();
        self.require_principal(&input.reviewer_id)?;
        input.author_id = claimant;
        input.policy_version = self.policy_version.clone();
        let attestation = ReviewAttestation::build(input, now)?;
        self.reviews.push(attestation.clone());
        self.ledger.append(
            CoordinationEvent::ReviewSubmitted {
                review: attestation.clone(),
            },
            now,
        )?;
        Ok(attestation)
    }

    /// Evaluate the merge gate for a slice at a specific commit.
    ///
    /// On an authorized result the claim advances to `Approved` and records the
    /// gated commit in `approved_commit`, so `mark_merged` can require that the
    /// merged commit is exactly the one that was reviewed. The decision is
    /// journaled either way, so denials are auditable too.
    pub fn evaluate_merge(
        &mut self,
        slice_id: &str,
        merger_id: &str,
        commit_sha: impl Into<String>,
        ci_green: bool,
        now: DateTime<Utc>,
    ) -> CoordinationResult<MergeGateDecision> {
        self.require_principal(merger_id)?;
        let claim = self.require_claim(slice_id)?.clone();

        let dependency_statuses = claim
            .depends_on
            .iter()
            .map(|dep| (dep.clone(), self.claims.get(dep).map(|c| c.status)))
            .collect();

        let reviews: Vec<ReviewAttestation> = self.reviews_for(slice_id).cloned().collect();

        let eval = MergeEvaluation {
            now,
            merger_id: merger_id.to_string(),
            commit_sha: commit_sha.into(),
            ci_green,
            policy_version: self.policy_version.clone(),
            dependency_statuses,
        };
        let decision = self.merge_policy.evaluate(&claim, &reviews, &eval);
        self.ledger.append(
            CoordinationEvent::MergeGateEvaluated {
                decision: decision.clone(),
            },
            now,
        )?;

        if decision.is_allowed() {
            if claim.status == ClaimStatus::InReview {
                self.apply_status(slice_id, ClaimStatus::Approved, now)?;
            }
            // Bind the approval to the exact commit that was gated.
            if let Some(current) = self.claims.get_mut(slice_id) {
                current.approved_commit = Some(eval.commit_sha.clone());
                current.updated_at = now;
            }
        }
        Ok(decision)
    }

    /// Mark a slice merged at a specific commit.
    ///
    /// Requires a prior `Allowed` gate decision — recorded in the ledger — that
    /// authorized exactly this `merger_id` for this `slice_id` **at this
    /// `commit_sha`**, and that the claim is currently `Approved` for that same
    /// commit. Binding to the commit is what stops a stale authorization (issued
    /// for an earlier, reviewed commit) from merging later, unreviewed code.
    pub fn mark_merged(
        &mut self,
        slice_id: &str,
        merger_id: &str,
        decision_id: &str,
        commit_sha: &str,
        now: DateTime<Utc>,
    ) -> CoordinationResult<()> {
        let claim = self.require_claim(slice_id)?.clone();
        if merger_id == claim.claimant {
            return Err(CoordinationError::SelfMerge {
                merger_id: merger_id.to_string(),
                slice_id: slice_id.to_string(),
            });
        }
        if !self.has_authorizing_decision(slice_id, merger_id, decision_id, commit_sha) {
            return Err(CoordinationError::MergeNotAuthorized {
                slice_id: slice_id.to_string(),
                merger_id: merger_id.to_string(),
            });
        }
        // The claim's live authorization must still be for this exact commit.
        if claim.status != ClaimStatus::Approved
            || claim.approved_commit.as_deref() != Some(commit_sha)
        {
            return Err(CoordinationError::MergeNotAuthorized {
                slice_id: slice_id.to_string(),
                merger_id: merger_id.to_string(),
            });
        }
        self.apply_status(slice_id, ClaimStatus::Merged, now)?;
        self.ledger.append(
            CoordinationEvent::SliceMerged {
                slice_id: slice_id.to_string(),
                merger_id: merger_id.to_string(),
                decision_id: decision_id.to_string(),
                commit_sha: commit_sha.to_string(),
            },
            now,
        )?;
        Ok(())
    }

    /// Whether the ledger contains an `Allowed` gate decision with this id that
    /// authorized `merger_id` to merge `slice_id` at `commit_sha`.
    fn has_authorizing_decision(
        &self,
        slice_id: &str,
        merger_id: &str,
        decision_id: &str,
        commit_sha: &str,
    ) -> bool {
        self.ledger.records().iter().any(|record| {
            matches!(
                &record.event,
                CoordinationEvent::MergeGateEvaluated { decision }
                    if decision.decision_id == decision_id
                        && decision.slice_id == slice_id
                        && decision.merger_id == merger_id
                        && decision.commit_sha == commit_sha
                        && decision.is_allowed()
            )
        })
    }

    /// Detect write-scope overlaps between currently active claims.
    ///
    /// Returns `(slice_id, other_slice_id, self_prefix, other_prefix)` for each
    /// overlapping pair. An empty result means parallel work is safely disjoint.
    pub fn conflicts(&self) -> Vec<(String, String, String, String)> {
        let active: Vec<&SliceClaim> = self.active_claims().collect();
        let mut out = Vec::new();
        for (i, a) in active.iter().enumerate() {
            for b in active.iter().skip(i + 1) {
                if let Some((prefix, other_prefix)) = a.write_scope.first_overlap(&b.write_scope) {
                    out.push((a.slice_id.clone(), b.slice_id.clone(), prefix, other_prefix));
                }
            }
        }
        out
    }

    /// Verify the coordination ledger's hash chain end to end.
    pub fn verify(&self) -> CoordinationResult<()> {
        self.ledger.verify_chain()
    }
}
