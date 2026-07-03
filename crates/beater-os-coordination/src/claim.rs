use std::collections::BTreeSet;
use std::fmt;

use chrono::{DateTime, Utc};
use serde::{Deserialize, Serialize};

/// A set of repository path prefixes a slice is allowed to write.
///
/// This is the coordination analogue of a `CapabilityGrant` scope: it bounds
/// *which files a parallel agent may touch*, so two agents working at the same
/// time cannot silently clobber each other. Disjoint write scopes are how this
/// kernel prevents the "ambient authority" failure mode (`final.md` 13.2) at
/// the level of the repository itself.
#[derive(Clone, Debug, Default, PartialEq, Eq, Serialize, Deserialize)]
pub struct WriteScope {
    /// Relative repo path prefixes, e.g. `crates/beater-os-coordination/`.
    /// A prefix that names a directory should end with `/`; a prefix without a
    /// trailing slash matches an exact file (e.g. `Cargo.toml`).
    pub prefixes: BTreeSet<String>,
}

impl WriteScope {
    /// Build a write scope from an iterator of prefixes, normalizing each.
    pub fn new<I, S>(prefixes: I) -> Self
    where
        I: IntoIterator<Item = S>,
        S: Into<String>,
    {
        Self {
            prefixes: prefixes.into_iter().map(|p| normalize(&p.into())).collect(),
        }
    }

    /// True when this scope owns no prefixes.
    pub fn is_empty(&self) -> bool {
        self.prefixes.is_empty()
    }

    /// Return the first overlapping `(self_prefix, other_prefix)` pair, if any.
    ///
    /// Two prefixes overlap when one is a path-prefix of (or equal to) the
    /// other. Overlap is symmetric and conservative: it treats nesting in
    /// either direction as a conflict, because either direction lets two agents
    /// write the same file.
    pub fn first_overlap(&self, other: &WriteScope) -> Option<(String, String)> {
        for a in &self.prefixes {
            for b in &other.prefixes {
                if prefixes_overlap(a, b) {
                    return Some((a.clone(), b.clone()));
                }
            }
        }
        None
    }

    /// True when the two scopes share no writable paths.
    pub fn is_disjoint(&self, other: &WriteScope) -> bool {
        self.first_overlap(other).is_none()
    }
}

/// Canonicalize a prefix so that textually different spellings of the same path
/// compare equal: trim whitespace, drop a leading `/` (scopes are repo-relative)
/// and empty (`//`) or `.` segments. A trailing slash is preserved because it
/// distinguishes "directory" from "exact file". `..` segments are intentionally
/// left intact so [`is_valid_prefix`] can reject them.
///
/// This runs on every constructed [`WriteScope`] *and* again inside
/// `claim_slice`, so a deserialized (store or `ClaimInput`) scope cannot slip an
/// un-normalized prefix past overlap detection.
fn normalize(prefix: &str) -> String {
    let trimmed = prefix.trim();
    let is_dir = trimmed.ends_with('/');
    let segments: Vec<&str> = trimmed
        .split('/')
        .filter(|seg| !seg.is_empty() && *seg != ".")
        .collect();
    let joined = segments.join("/");
    if joined.is_empty() {
        // e.g. "", "/", ".", "//" -> empty, rejected later by is_valid_prefix.
        String::new()
    } else if is_dir {
        format!("{joined}/")
    } else {
        joined
    }
}

/// A prefix is invalid if it is empty, absolute, or tries to escape the repo
/// root via `..` components.
pub(crate) fn is_valid_prefix(prefix: &str) -> bool {
    if prefix.is_empty() || prefix.starts_with('/') {
        return false;
    }
    !prefix.split('/').any(|component| component == "..")
}

/// Directory-aware prefix overlap.
///
/// `crates/x/` overlaps `crates/x/mod.rs` and `crates/x/`, but not
/// `crates/xyz/` — segment boundaries are respected so sibling directories
/// that merely share a textual prefix do not collide.
fn prefixes_overlap(a: &str, b: &str) -> bool {
    contains(a, b) || contains(b, a)
}

/// True when `outer` is a path-prefix of (or equal to) `inner`.
fn contains(outer: &str, inner: &str) -> bool {
    if outer == inner {
        return true;
    }
    // A directory prefix (ends with `/`) contains anything beneath it.
    if let Some(dir) = outer.strip_suffix('/') {
        return inner == dir || inner.starts_with(&format!("{dir}/"));
    }
    // A non-directory prefix only contains a deeper path at a segment boundary,
    // e.g. `crates/x` contains `crates/x/mod.rs` but not `crates/xyz`.
    inner.starts_with(&format!("{outer}/"))
}

/// Lifecycle state of a slice claim.
///
/// Mirrors the disciplined state machine of `SessionStatus` in
/// `beater-os-core`: transitions are explicit and journaled.
#[derive(Clone, Copy, Debug, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum ClaimStatus {
    /// Work is claimed and in progress on a branch.
    Claimed,
    /// A pull request is open and under independent review.
    InReview,
    /// The merge gate authorized a non-author merger.
    Approved,
    /// The slice has been merged to the base branch.
    Merged,
    /// The claim was abandoned; its write scope is free again.
    Released,
}

impl fmt::Display for ClaimStatus {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        let text = match self {
            ClaimStatus::Claimed => "claimed",
            ClaimStatus::InReview => "in_review",
            ClaimStatus::Approved => "approved",
            ClaimStatus::Merged => "merged",
            ClaimStatus::Released => "released",
        };
        f.write_str(text)
    }
}

impl ClaimStatus {
    /// Whether a claim in this status still reserves its write scope.
    pub fn holds_write_scope(self) -> bool {
        !matches!(self, ClaimStatus::Released | ClaimStatus::Merged)
    }

    /// Whether `next` is a legal successor of `self`.
    pub fn can_transition_to(self, next: ClaimStatus) -> bool {
        use ClaimStatus::*;
        matches!(
            (self, next),
            (Claimed, InReview)
                | (Claimed, Released)
                | (InReview, Approved)
                | (InReview, Claimed)
                | (InReview, Released)
                | (Approved, Merged)
                | (Approved, InReview)
                | (Approved, Released)
        )
    }
}

/// A claim over one backlog slice, held by exactly one principal at a time.
///
/// Maps a `final.md` implementation slice to an accountable owner, a branch, a
/// bounded write scope, and its upstream dependencies. This is how the
/// "communication loop between agents" is made concrete and inspectable rather
/// than conversational.
#[derive(Clone, Debug, PartialEq, Eq, Serialize, Deserialize)]
pub struct SliceClaim {
    pub claim_id: String,
    /// Stable slice identifier, e.g. `"coordination-kernel"`.
    pub slice_id: String,
    /// `principal_id` of the author who owns this slice.
    pub claimant: String,
    /// Git branch the work lands on.
    pub branch: String,
    pub write_scope: WriteScope,
    /// Slice ids that must be `Merged` before this one may merge.
    #[serde(default)]
    pub depends_on: BTreeSet<String>,
    pub status: ClaimStatus,
    /// The exact commit the current `Approved` status was gated at, if any.
    ///
    /// Set when a merge gate authorizes a commit; cleared whenever the claim
    /// leaves `Approved` (e.g. new commits push it back to `InReview`). Binding
    /// approval to a commit is what stops a stale authorization from merging
    /// later, unreviewed code.
    #[serde(default)]
    pub approved_commit: Option<String>,
    pub reason: String,
    pub created_at: DateTime<Utc>,
    pub updated_at: DateTime<Utc>,
}
