use std::collections::{BTreeMap, BTreeSet};
use std::path::{Component, Path};

use chrono::{DateTime, Utc};
use serde::{Deserialize, Serialize};

use crate::error::BeaterOsResult;
use crate::hash::{HashValue, hash_json};

#[derive(Clone, Debug, PartialEq, Eq, PartialOrd, Ord, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum ActionKind {
    Read,
    Write,
    Execute,
    Navigate,
    Submit,
    Communicate,
    Spend,
    Deploy,
    Remember,
    Delegate,
    AskHuman,
}

#[derive(Clone, Debug, PartialEq, Eq, PartialOrd, Ord, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum ResourceKind {
    Workspace,
    FilePath,
    NetworkEndpoint,
    BrowserOrigin,
    Tool,
    Memory,
    ModelRoute,
    PaymentRail,
    CloudResource,
    HumanRecipient,
    Scenario,
    Other,
}

#[derive(Clone, Copy, Debug, PartialEq, Eq, PartialOrd, Ord, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum RiskClass {
    Low,
    Medium,
    High,
    Critical,
}

#[derive(Clone, Copy, Debug, PartialEq, Eq, PartialOrd, Ord, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum DataClass {
    Public,
    Internal,
    Personal,
    Customer,
    Financial,
    Secret,
    Code,
    Binary,
    UntrustedWeb,
    UntrustedEmail,
    UntrustedDocument,
    ToolOutput,
}

#[derive(Clone, Copy, Debug, PartialEq, Eq, PartialOrd, Ord, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum SideEffectClass {
    None,
    LocalWrite,
    NetworkWrite,
    BrowserSubmit,
    HumanCommunication,
    Payment,
    CloudMutation,
    Deployment,
    MemoryWrite,
    Delegation,
}

#[derive(Clone, Debug, PartialEq, Eq, PartialOrd, Ord, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum TaintLabel {
    TrustedUserInstruction,
    SystemPolicy,
    DeveloperInstruction,
    UntrustedWeb,
    UntrustedEmail,
    UntrustedDocument,
    ToolOutput,
    Secret,
    PersonalData,
    CustomerData,
    FinancialData,
    Code,
    Binary,
    PaymentInstruction,
}

#[derive(Clone, Debug, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum SessionStatus {
    Created,
    Running,
    WaitingForApproval,
    Paused,
    Completed,
    Failed,
    Canceled,
}

#[derive(Clone, Debug, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum DecisionResult {
    Allowed,
    Denied,
    NeedsApproval,
    NeedsSimulation,
    NeedsNarrowedGrant,
}

#[derive(Clone, Debug, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum DelegationMode {
    None,
    AttenuatedOnly,
    SameScope,
}

#[derive(Clone, Debug, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum ApprovalMode {
    None,
    Human,
    MultiParty,
}

#[derive(Clone, Debug, PartialEq, Eq, Serialize, Deserialize)]
pub struct ApprovalRequirement {
    pub mode: ApprovalMode,
    pub threshold_risk: RiskClass,
    #[serde(default)]
    pub reviewer_ids: Vec<String>,
}

impl Default for ApprovalRequirement {
    fn default() -> Self {
        Self {
            mode: ApprovalMode::None,
            threshold_risk: RiskClass::Critical,
            reviewer_ids: Vec::new(),
        }
    }
}

#[derive(Clone, Debug, PartialEq, Eq, Serialize, Deserialize)]
pub struct ApprovalEvidence {
    pub review_id: String,
    pub action_id: String,
    pub manifest_hash: HashValue,
    pub grant_id: String,
    pub reviewer_id: String,
    pub approved_at: DateTime<Utc>,
    pub policy_version: String,
}

#[derive(Clone, Debug, Default, PartialEq, Eq, Serialize, Deserialize)]
pub struct Budget {
    #[serde(default)]
    pub max_model_cents: Option<u64>,
    #[serde(default)]
    pub max_tool_calls: Option<u64>,
    #[serde(default)]
    pub max_wall_ms: Option<u64>,
    #[serde(default)]
    pub max_payment_minor_units: Option<u64>,
}

impl Budget {
    pub fn fits_within(&self, limit: &Budget) -> bool {
        within_optional_limit(self.max_model_cents, limit.max_model_cents)
            && within_optional_limit(self.max_tool_calls, limit.max_tool_calls)
            && within_optional_limit(self.max_wall_ms, limit.max_wall_ms)
            && within_optional_limit(self.max_payment_minor_units, limit.max_payment_minor_units)
    }
}

fn within_optional_limit(requested: Option<u64>, limit: Option<u64>) -> bool {
    match (requested, limit) {
        (Some(requested), Some(limit)) => requested <= limit,
        (Some(_), None) | (None, None) => true,
        (None, Some(_)) => false,
    }
}

#[derive(Clone, Debug, PartialEq, Eq, Serialize, Deserialize)]
pub struct ModelPolicy {
    #[serde(default)]
    pub allowed_routes: BTreeSet<String>,
    #[serde(default)]
    pub local_only: bool,
    #[serde(default = "default_max_data_class")]
    pub max_data_class: Option<DataClass>,
}

impl Default for ModelPolicy {
    fn default() -> Self {
        Self {
            allowed_routes: BTreeSet::new(),
            local_only: false,
            max_data_class: default_max_data_class(),
        }
    }
}

#[derive(Clone, Debug, PartialEq, Eq, Serialize, Deserialize)]
pub struct AgentIdentity {
    pub agent_id: String,
    pub human_owner: String,
    #[serde(default)]
    pub organization_owner: Option<String>,
    pub agent_type: String,
    pub version: String,
    #[serde(default)]
    pub signing_key_ref: Option<String>,
    pub default_policy_profile: String,
    #[serde(default)]
    pub revoked: bool,
}

#[derive(Clone, Debug, PartialEq, Eq, Serialize, Deserialize)]
pub struct AgentSession {
    pub session_id: String,
    pub created_at: DateTime<Utc>,
    pub created_by: String,
    pub agent_id: String,
    pub workspace_id: String,
    pub goal: String,
    #[serde(default)]
    pub constraints: Vec<String>,
    pub policy_profile: String,
    #[serde(default)]
    pub initial_capability_ids: BTreeSet<String>,
    #[serde(default)]
    pub budget: Budget,
    #[serde(default)]
    pub model_policy: ModelPolicy,
    #[serde(default)]
    pub memory_scope: Option<String>,
    pub journal_root: String,
    pub status: SessionStatus,
}

#[derive(Clone, Debug, PartialEq, Eq, Serialize, Deserialize)]
pub struct CapabilitySelector {
    pub resource_kind: ResourceKind,
    pub resource_id: String,
}

impl CapabilitySelector {
    pub fn matches(&self, other: &CapabilitySelector) -> bool {
        self.resource_kind == other.resource_kind
            && (self.resource_id == other.resource_id || self.resource_id == "*")
    }
}

#[derive(Clone, Debug, PartialEq, Eq, Serialize, Deserialize)]
pub struct CapabilityScope {
    pub selector: CapabilitySelector,
    pub actions: BTreeSet<ActionKind>,
}

impl CapabilityScope {
    pub fn allows(&self, selector: &CapabilitySelector, action: &ActionKind) -> bool {
        self.selector.matches(selector) && self.actions.contains(action)
    }
}

/// Default risk ceiling for a grant. An absent `max_risk` must fail closed at a
/// bounded ceiling, never at "unlimited". Callers that genuinely want no ceiling
/// opt in explicitly with `max_risk: null`.
fn default_max_risk() -> Option<RiskClass> {
    Some(RiskClass::Medium)
}

/// Default data-class ceiling for model routes and grants. See
/// [`default_max_risk`]: absence must fail closed at a bounded class, not at
/// "unlimited".
fn default_max_data_class() -> Option<DataClass> {
    Some(DataClass::Internal)
}

/// Constraints attached to a capability grant.
///
/// Two defaulting models coexist here, and the asymmetry is deliberate:
///
/// - `max_risk` / `max_data_class` are **safety ceilings**. An omitted field
///   fails closed at the bounded cap (see [`default_max_risk`] /
///   [`default_max_data_class`]); "no ceiling" must be an explicit, auditable
///   `null`. This is why they do not use a plain `#[serde(default)]`, which for
///   `Option<T>` would be `None` (unbounded) — a fail-open on a partial object.
/// - `budget`, `network_allowlist`, and `path_prefixes` are **additive
///   restrictions**, not safety caps. An omitted/empty value adds no restriction
///   from this grant: an empty allowlist/prefix set means "this grant imposes no
///   extra network/path bound", and an all-`None` budget defers to the session
///   budget. Absence here is intentionally permissive, not a fail-open, so a
///   plain `#[serde(default)]` is correct for them.
#[derive(Clone, Debug, PartialEq, Eq, Serialize, Deserialize)]
pub struct GrantConstraints {
    #[serde(default = "default_max_risk")]
    pub max_risk: Option<RiskClass>,
    #[serde(default = "default_max_data_class")]
    pub max_data_class: Option<DataClass>,
    #[serde(default)]
    pub budget: Budget,
    #[serde(default)]
    pub network_allowlist: BTreeSet<String>,
    #[serde(default)]
    pub path_prefixes: BTreeSet<String>,
}

impl Default for GrantConstraints {
    fn default() -> Self {
        Self {
            max_risk: default_max_risk(),
            max_data_class: default_max_data_class(),
            budget: Budget::default(),
            network_allowlist: BTreeSet::new(),
            path_prefixes: BTreeSet::new(),
        }
    }
}

#[derive(Clone, Debug, PartialEq, Eq, Serialize, Deserialize)]
pub struct CapabilityGrant {
    pub grant_id: String,
    pub issuer: String,
    pub holder: String,
    pub session_id: String,
    /// Grant this one was delegated (attenuated) from, if any. A delegated grant
    /// is authority *indirected* through its parent: it is only exercisable
    /// while the whole ancestor chain is live, so revoking a parent transitively
    /// revokes every descendant (`final.md` §6.2; issue #10). A root grant
    /// issued directly to a principal has no parent.
    #[serde(default)]
    pub parent_grant_id: Option<String>,
    pub scope: CapabilityScope,
    #[serde(default)]
    pub denied_actions: BTreeSet<ActionKind>,
    #[serde(default)]
    pub constraints: GrantConstraints,
    pub expires_at: DateTime<Utc>,
    pub delegation: DelegationMode,
    #[serde(default)]
    pub approval: ApprovalRequirement,
    pub revocation_handle: String,
    pub policy_version: String,
    pub reason: String,
    #[serde(default)]
    pub revoked: bool,
}

impl CapabilityGrant {
    pub fn is_active_at(&self, now: DateTime<Utc>) -> bool {
        !self.revoked && self.expires_at > now
    }

    /// Whether this grant admits `manifest` at the given `effective_risk`.
    ///
    /// `effective_risk` is the kernel-derived effective risk (see
    /// `policy::derived_risk_floor`), never the raw agent-asserted
    /// `manifest.risk_class`: the `max_risk` ceiling is checked against it so the
    /// agent can only raise risk above the kernel floor, never lower it below.
    pub fn allows_manifest(
        &self,
        manifest: &ActionManifest,
        effective_risk: RiskClass,
        now: DateTime<Utc>,
        actor_id: &str,
    ) -> bool {
        if !self.is_active_at(now) {
            return false;
        }
        if self.holder != actor_id || self.session_id != manifest.session_id {
            return false;
        }
        if self.denied_actions.contains(&manifest.action_kind) {
            return false;
        }
        if !self.scope.allows(&manifest.target, &manifest.action_kind) {
            return false;
        }
        if let Some(max_risk) = self.constraints.max_risk
            && effective_risk > max_risk
        {
            return false;
        }
        if let Some(max_data_class) = self.constraints.max_data_class
            && manifest
                .data_classes
                .iter()
                .any(|class| *class > max_data_class)
        {
            return false;
        }
        if !manifest
            .requested_budget
            .fits_within(&self.constraints.budget)
        {
            return false;
        }
        if !self.path_constraints_allow(manifest) {
            return false;
        }
        if !self.network_constraints_allow(manifest) {
            return false;
        }
        true
    }

    fn path_constraints_allow(&self, manifest: &ActionManifest) -> bool {
        if manifest.target.resource_kind != ResourceKind::FilePath
            || self.constraints.path_prefixes.is_empty()
        {
            return true;
        }
        let Some(resolved_target) = &manifest.resolved_target else {
            return false;
        };
        if resolved_target.resource_kind != ResourceKind::FilePath {
            return false;
        }
        let Some(requested_path) = normalized_absolute_path(&manifest.target.resource_id) else {
            return false;
        };
        let Some(resolved_path) = normalized_absolute_path(&resolved_target.resource_id) else {
            return false;
        };
        self.constraints.path_prefixes.iter().any(|prefix| {
            normalized_absolute_path(prefix)
                .map(|normalized_prefix| {
                    path_is_inside_prefix(&requested_path, &normalized_prefix)
                        && path_is_inside_prefix(&resolved_path, &normalized_prefix)
                })
                .unwrap_or(false)
        })
    }

    fn network_constraints_allow(&self, manifest: &ActionManifest) -> bool {
        if manifest.target.resource_kind != ResourceKind::NetworkEndpoint
            || self.constraints.network_allowlist.is_empty()
        {
            return true;
        }
        let host = network_host(&manifest.target.resource_id);
        self.constraints
            .network_allowlist
            .iter()
            .any(|allowed| host_matches_allowed(&host, allowed))
    }
}

fn path_is_inside_prefix(path: &str, prefix: &str) -> bool {
    if path == prefix {
        return true;
    }
    let mut normalized_prefix = prefix.trim_end_matches('/').to_string();
    normalized_prefix.push('/');
    path.starts_with(&normalized_prefix)
}

fn normalized_absolute_path(path: &str) -> Option<String> {
    let path = Path::new(path);
    if !path.is_absolute() {
        return None;
    }
    let mut parts = Vec::new();
    for component in path.components() {
        match component {
            Component::RootDir => {}
            Component::Normal(part) => parts.push(part.to_string_lossy().to_string()),
            Component::CurDir | Component::ParentDir | Component::Prefix(_) => return None,
        }
    }
    if parts.is_empty() {
        Some("/".to_string())
    } else {
        Some(format!("/{}", parts.join("/")))
    }
}

fn network_host(endpoint: &str) -> String {
    let without_scheme = endpoint
        .split_once("://")
        .map(|(_, rest)| rest)
        .unwrap_or(endpoint);
    let authority = without_scheme.split('/').next().unwrap_or(without_scheme);
    authority
        .split('@')
        .next_back()
        .unwrap_or(authority)
        .split(':')
        .next()
        .unwrap_or(authority)
        .to_ascii_lowercase()
}

fn host_matches_allowed(host: &str, allowed: &str) -> bool {
    let allowed = allowed.to_ascii_lowercase();
    host == allowed || host.ends_with(&format!(".{allowed}"))
}

#[derive(Clone, Debug, PartialEq, Eq, Serialize, Deserialize)]
pub struct ActionManifest {
    pub action_id: String,
    pub session_id: String,
    pub tool_id: String,
    pub action_kind: ActionKind,
    pub target: CapabilitySelector,
    #[serde(default)]
    pub resolved_target: Option<CapabilitySelector>,
    pub inputs_digest: String,
    pub inputs_summary: String,
    #[serde(default)]
    pub expected_outputs: Vec<String>,
    #[serde(default)]
    pub expected_side_effects: BTreeSet<SideEffectClass>,
    #[serde(default)]
    pub required_grants: BTreeSet<String>,
    #[serde(default)]
    pub requested_budget: Budget,
    pub risk_class: RiskClass,
    #[serde(default)]
    pub data_classes: BTreeSet<DataClass>,
    #[serde(default)]
    pub taint: BTreeSet<TaintLabel>,
    #[serde(default)]
    pub idempotency_key: Option<String>,
    #[serde(default)]
    pub payment_intent: Option<PaymentIntent>,
    #[serde(default)]
    pub compensation_plan: Option<String>,
    pub human_explanation: String,
}

impl ActionManifest {
    pub fn has_external_side_effect(&self) -> bool {
        self.expected_side_effects.iter().any(|effect| {
            !matches!(
                effect,
                SideEffectClass::None | SideEffectClass::MemoryWrite | SideEffectClass::LocalWrite
            )
        })
    }

    pub fn digest(&self) -> BeaterOsResult<HashValue> {
        hash_json(self)
    }
}

#[derive(Clone, Debug, PartialEq, Eq, Serialize, Deserialize)]
pub struct PaymentIntent {
    pub mandate_id: String,
    pub rail: String,
    pub adapter_id: String,
    #[serde(default)]
    pub adapter_version: Option<String>,
    pub asset: String,
    pub amount_minor_units: u64,
    pub counterparty_ref: String,
    pub counterparty_binding_hash: HashValue,
    pub purpose: String,
    pub payment_idempotency_key: String,
    pub envelope_format: String,
    pub envelope_hash: HashValue,
    #[serde(default)]
    pub envelope_expires_at: Option<DateTime<Utc>>,
}

#[derive(Clone, Debug, PartialEq, Eq, Serialize, Deserialize)]
pub struct PolicyDecision {
    pub decision_id: String,
    pub action_id: String,
    pub manifest_hash: HashValue,
    pub policy_version: String,
    pub result: DecisionResult,
    #[serde(default)]
    pub matched_rules: Vec<String>,
    pub explanation: String,
    #[serde(default)]
    pub required_review: Option<String>,
    #[serde(default)]
    pub required_simulation: Option<String>,
    pub created_at: DateTime<Utc>,
}

#[derive(Clone, Debug, PartialEq, Eq, Serialize, Deserialize)]
pub struct MemoryRecord {
    pub memory_id: String,
    pub source_event_id: String,
    pub source_digest: String,
    pub writer: String,
    pub created_at: DateTime<Utc>,
    pub kind: String,
    pub content_ref: String,
    pub summary: String,
    pub confidence_basis_points: u16,
    pub sensitivity: DataClass,
    #[serde(default)]
    pub expires_at: Option<DateTime<Utc>>,
    pub access_policy: String,
}

#[derive(Clone, Debug, PartialEq, Eq, Serialize, Deserialize)]
pub struct PaymentMandate {
    pub mandate_id: String,
    pub issuer: String,
    pub holder: String,
    pub session_id: String,
    pub rail: String,
    pub asset: String,
    pub max_minor_units: u64,
    pub counterparty_policy: String,
    pub purpose: String,
    pub expires_at: DateTime<Utc>,
    pub approval_threshold_minor_units: u64,
    pub idempotency_key: String,
    pub receipt_requirement: String,
    #[serde(default)]
    pub allowed_adapter_ids: BTreeSet<String>,
    #[serde(default)]
    pub allowed_envelope_formats: BTreeSet<String>,
}

#[derive(Clone, Debug, PartialEq, Eq, Serialize, Deserialize)]
pub struct ScenarioManifest {
    pub scenario_id: String,
    pub goal: String,
    pub environment: String,
    #[serde(default)]
    pub fixtures: BTreeMap<String, String>,
    #[serde(default)]
    pub allowed_tools: BTreeSet<String>,
    #[serde(default)]
    pub forbidden_actions: BTreeSet<ActionKind>,
    pub oracle: String,
    #[serde(default)]
    pub success_criteria: Vec<String>,
    #[serde(default)]
    pub risk_traps: Vec<String>,
    #[serde(default)]
    pub budget: Budget,
    #[serde(default)]
    pub expected_trace_properties: Vec<String>,
}

#[derive(Clone, Debug, PartialEq, Eq, Serialize, Deserialize)]
pub struct ToolManifest {
    pub tool_id: String,
    pub publisher: String,
    pub version: String,
    pub transport: String,
    #[serde(default)]
    pub required_capabilities: Vec<CapabilityScope>,
    #[serde(default)]
    pub side_effects: BTreeSet<SideEffectClass>,
    pub risk_class: RiskClass,
    #[serde(default)]
    pub sandbox_required: bool,
}

#[derive(Clone, Debug, PartialEq, Eq, Serialize, Deserialize)]
pub struct HumanReviewRequest {
    pub review_id: String,
    pub action_id: String,
    pub risk_class: RiskClass,
    pub preview_ref: String,
    pub required_decision: String,
    pub reviewer_id: String,
    pub created_at: DateTime<Utc>,
}

#[derive(Clone, Debug, PartialEq, Eq, Serialize, Deserialize)]
pub struct SimulationEvidence {
    pub simulation_id: String,
    pub action_id: String,
    pub manifest_hash: HashValue,
    pub scenario_id: String,
    pub passed_at: DateTime<Utc>,
    pub policy_version: String,
}
