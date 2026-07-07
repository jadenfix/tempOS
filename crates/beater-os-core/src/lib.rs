//! Core contracts for beaterOS.
//!
//! This crate is the contract-first foundation from `final.md`: sessions,
//! capabilities, action manifests, policy decisions, receipts, and append-only
//! journals. It intentionally has no model dependency. Models can propose
//! actions, but this crate owns deterministic admission and audit primitives.

mod contracts;
mod error;
mod hash;
mod journal;
mod policy;
mod receipt;

pub use contracts::{
    ActionKind, ActionManifest, AgentIdentity, AgentSession, ApprovalEvidence, ApprovalMode,
    ApprovalRequirement, Budget, CapabilityGrant, CapabilityScope, CapabilitySelector, DataClass,
    DecisionResult, DelegationMode, ExecutionLease, ExecutionLeaseReconciliation,
    ExecutionLeaseResolution, GrantConstraints, HumanReviewRequest, MemoryRecord, ModelPolicy,
    PaymentIntent, PaymentMandate, PolicyDecision, ResourceKind, RiskClass, ScenarioManifest,
    SessionStatus, SideEffectClass, SimulationEvidence, TaintLabel, ToolManifest,
};
pub use error::{BeaterOsError, BeaterOsResult};
pub use hash::{HashValue, hash_json};
pub use journal::{
    InMemoryJournal, JournalEvent, JournalRecord, JournalSnapshot, JournalVerificationReport,
};
pub use policy::{AdmissionContext, PolicyEngine, derived_risk_floor};
pub use receipt::{
    CapabilityReceipt, CapabilityReceiptInput, PaymentReceiptEvidence, PaymentSettlementStatus,
    ReceiptLedger,
};
