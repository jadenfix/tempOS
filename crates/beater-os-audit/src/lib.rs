//! Independent audit surface for beaterOS.
//!
//! This crate is the reviewer-facing companion to `beater-os-core`. Where the
//! core owns deterministic admission and the primary chain, this crate owns
//! *independent* verification and human-legible presentation of what happened:
//!
//! - [`verify_snapshot`]: a second, independent implementation of the journal
//!   audit invariants (`final.md` §8.15 small verifiable TCB, §13.11
//!   tamper-evident logs, §26 "journal before side effects / receipts after").
//! - [`render_trace`]: a legible timeline of a run (`final.md` §25 step 9 trace
//!   viewer, §17.4 UX "what did it already do / what changed").
//! - [`compute_metrics`]: coverage numbers for reviewers (`final.md` §23.3
//!   observability metrics — trace/receipt completeness, denial explainability).
//! - [`build_bundle`]: a redaction-safe, digest-anchored export
//!   (`final.md` §6.6 verifiable side effects, §13.15 incident audit export).
//!
//! The crate has no model dependency and performs no I/O beyond what a caller
//! hands it, so it is safe to run inside a review or incident-response context.

mod bundle;
mod events;
mod metrics;
mod trace;
mod verify;

pub use bundle::{AuditBundle, RecordDigest, build_bundle, bundle_to_json};
pub use metrics::{AuditMetrics, Coverage, compute_metrics};
pub use trace::render_trace;
pub use verify::{
    AuditReport, CheckOutcome, CheckResult, GENESIS_HASH, snapshot_root_hash, verify_expected_root,
    verify_snapshot,
};

/// Version of the audit report/bundle format emitted by this crate.
pub const AUDIT_FORMAT_VERSION: &str = "0.1.0";
