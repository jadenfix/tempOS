"""Deterministic policy admission, ported from `beater-os-core`.

This is an independent, language-neutral re-implementation of the admission
semantics in `crates/beater-os-core/src/policy.rs` and `contracts.rs`
(`PolicyEngine::admit`, `CapabilityGrant::allows_manifest`, `Budget`). Keeping a
second implementation in a different language is deliberate: `final.md` 15.6
argues verifiers should be separable from executors, and this module lets the
conformance gate confirm that the golden traces and adversarial scenarios would
be admitted/denied the *same way* by any conformant implementation.

Nothing here trusts model output. Admission is a pure function of the manifest,
the active grants, and the approval/simulation evidence -- policy outside the
model (`final.md` 8.12, 13.1).
"""

from __future__ import annotations

from datetime import datetime
from typing import Any

# Enum orderings mirror the `#[derive(Ord)]` declaration order in contracts.rs.
RISK_ORDER = ["low", "medium", "high", "critical"]
DATA_CLASS_ORDER = [
    "public",
    "internal",
    "personal",
    "customer",
    "financial",
    "secret",
    "code",
    "binary",
    "untrusted_web",
    "untrusted_email",
    "untrusted_document",
    "tool_output",
]
UNTRUSTED_TAINT = {"untrusted_web", "untrusted_email", "untrusted_document"}
DANGEROUS_UNTRUSTED_ACTIONS = {"spend", "deploy", "delegate"}
# Side effects that are NOT considered external (final.md: local/memory stay local).
NON_EXTERNAL_SIDE_EFFECTS = {"none", "memory_write", "local_write"}
BUDGET_FIELDS = [
    "max_model_cents",
    "max_tool_calls",
    "max_wall_ms",
    "max_payment_minor_units",
]


class AdmissionError(ValueError):
    """Raised when a manifest/grant/context is structurally unusable."""


def _ts(value: str) -> datetime:
    return datetime.fromisoformat(value.replace("Z", "+00:00"))


def _risk_gt(a: str, b: str) -> bool:
    return RISK_ORDER.index(a) > RISK_ORDER.index(b)


def _risk_gte(a: str, b: str) -> bool:
    return RISK_ORDER.index(a) >= RISK_ORDER.index(b)


def _data_gt(a: str, b: str) -> bool:
    return DATA_CLASS_ORDER.index(a) > DATA_CLASS_ORDER.index(b)


# --- Budget ---------------------------------------------------------------


def _within_optional_limit(requested, limit) -> bool:
    if requested is not None and limit is not None:
        return requested <= limit
    if requested is not None and limit is None:
        return True
    if requested is None and limit is None:
        return True
    return False  # requested None, limit Some -> fail closed


def budget_fits_within(requested: dict, limit: dict) -> bool:
    requested = requested or {}
    limit = limit or {}
    return all(
        _within_optional_limit(requested.get(f), limit.get(f)) for f in BUDGET_FIELDS
    )


# --- Path / network selectors --------------------------------------------


def normalized_absolute_path(path: str) -> str | None:
    if not path.startswith("/"):
        return None
    parts: list[str] = []
    for part in path.split("/"):
        if part in ("", "."):
            continue
        if part == "..":
            return None  # reject traversal, matches Rust ParentDir -> None
        parts.append(part)
    return "/" if not parts else "/" + "/".join(parts)


def _path_inside_prefix(path: str, prefix: str) -> bool:
    if path == prefix:
        return True
    return path.startswith(prefix.rstrip("/") + "/")


def _network_host(endpoint: str) -> str:
    without_scheme = endpoint.split("://", 1)[-1]
    authority = without_scheme.split("/", 1)[0]
    authority = authority.split("@")[-1]
    return authority.split(":", 1)[0].lower()


def _host_matches_allowed(host: str, allowed: str) -> bool:
    allowed = allowed.lower()
    return host == allowed or host.endswith("." + allowed)


# --- Grant checks ---------------------------------------------------------


def _selector_matches(scope_sel: dict, target: dict) -> bool:
    return scope_sel["resource_kind"] == target["resource_kind"] and (
        scope_sel["resource_id"] == target["resource_id"]
        or scope_sel["resource_id"] == "*"
    )


def _scope_allows(scope: dict, target: dict, action: str) -> bool:
    return _selector_matches(scope["selector"], target) and action in scope["actions"]


def _effective_constraints(grant: dict) -> dict:
    """Return the grant's constraints, mirroring serde's defaulting.

    Rust `CapabilityGrant.constraints` is `#[serde(default)]`, and
    `GrantConstraints::default()` sets `max_risk = Some(Medium)` and
    `max_data_class = Some(Internal)`. So an *absent* `constraints` key means the
    default Medium/Internal ceilings (NOT unbounded). A *present* object keeps its
    fields as-is; an omitted `Option` field inside it is `None` (unbounded), which
    also matches serde field-level `#[serde(default)]` on `Option<..>`.
    """
    if grant.get("constraints") is not None:
        return grant["constraints"]
    return {"max_risk": "medium", "max_data_class": "internal"}


def grant_is_active_at(grant: dict, now: datetime) -> bool:
    return not grant.get("revoked", False) and _ts(grant["expires_at"]) > now


def grant_allows_manifest(grant: dict, manifest: dict, now: datetime, actor_id: str) -> bool:
    if not grant_is_active_at(grant, now):
        return False
    if grant["holder"] != actor_id or grant["session_id"] != manifest["session_id"]:
        return False
    action = manifest["action_kind"]
    if action in grant.get("denied_actions", []):
        return False
    if not _scope_allows(grant["scope"], manifest["target"], action):
        return False

    constraints = _effective_constraints(grant)
    max_risk = constraints.get("max_risk")
    if max_risk is not None and _risk_gt(manifest["risk_class"], max_risk):
        return False
    max_data = constraints.get("max_data_class")
    if max_data is not None and any(
        _data_gt(dc, max_data) for dc in manifest.get("data_classes", [])
    ):
        return False
    if not budget_fits_within(manifest.get("requested_budget", {}), constraints.get("budget", {})):
        return False
    if not _path_constraints_allow(grant, manifest, constraints):
        return False
    if not _network_constraints_allow(grant, manifest, constraints):
        return False
    return True


def _path_constraints_allow(grant: dict, manifest: dict, constraints: dict) -> bool:
    prefixes = constraints.get("path_prefixes", [])
    if manifest["target"]["resource_kind"] != "file_path" or not prefixes:
        return True
    resolved = manifest.get("resolved_target")
    if resolved is None or resolved["resource_kind"] != "file_path":
        return False
    requested_path = normalized_absolute_path(manifest["target"]["resource_id"])
    resolved_path = normalized_absolute_path(resolved["resource_id"])
    if requested_path is None or resolved_path is None:
        return False
    for prefix in prefixes:
        norm = normalized_absolute_path(prefix)
        if norm is None:
            continue
        if _path_inside_prefix(requested_path, norm) and _path_inside_prefix(resolved_path, norm):
            return True
    return False


def _network_constraints_allow(grant: dict, manifest: dict, constraints: dict) -> bool:
    allowlist = constraints.get("network_allowlist", [])
    if manifest["target"]["resource_kind"] != "network_endpoint" or not allowlist:
        return True
    host = _network_host(manifest["target"]["resource_id"])
    return any(_host_matches_allowed(host, allowed) for allowed in allowlist)


# --- Manifest helpers -----------------------------------------------------


def has_external_side_effect(manifest: dict) -> bool:
    return any(
        effect not in NON_EXTERNAL_SIDE_EFFECTS
        for effect in manifest.get("expected_side_effects", [])
    )


def _approval_from_reviewer(grant, manifest, ctx, now, reviewer_id) -> bool:
    # NOTE: binds on action_id, not the manifest body hash. The Rust core's HEAD
    # additionally binds evidence to a manifest_hash; adopting that here depends on
    # the canonical-hashing convergence tracked in tools/conformance/README.md.
    for ap in ctx.get("approvals", []):
        if (
            _ts(ap["approved_at"]) <= now
            and ap["action_id"] == manifest["action_id"]
            and ap["grant_id"] == grant["grant_id"]
            and ap["policy_version"] == ctx["policy_version"]
            and ap["reviewer_id"] == reviewer_id
        ):
            return True
    return False


def _has_approval_for_grant(grant, manifest, ctx, now) -> bool:
    approval = grant.get("approval", {}) or {}
    mode = approval.get("mode", "none")
    reviewers = approval.get("reviewer_ids", [])
    if mode == "none":
        return True
    if mode == "human":
        return any(_approval_from_reviewer(grant, manifest, ctx, now, r) for r in reviewers)
    if mode == "multi_party":
        return bool(reviewers) and all(
            _approval_from_reviewer(grant, manifest, ctx, now, r) for r in reviewers
        )
    raise AdmissionError(f"unknown approval mode: {mode}")


def _explicit_action_evidence_exists(grant, manifest, ctx, now) -> bool:
    """True iff SOME action-bound approval evidence exists for this grant.

    Reviewer-agnostic: used only for the `mode == none` untrusted case, where the
    grant configures no authorized reviewers but an untrusted dangerous action
    still needs explicit human sign-off bound to this exact action+grant+policy.
    Evidence binds on action_id (not manifest body hash) pending the cross-language
    canonical-hashing convergence tracked in tools/conformance/README.md.
    """
    return any(
        _ts(ap["approved_at"]) <= now
        and ap["action_id"] == manifest["action_id"]
        and ap["grant_id"] == grant["grant_id"]
        and ap["policy_version"] == ctx["policy_version"]
        for ap in ctx.get("approvals", [])
    )


def _untrusted_dangerous_approved(grant, manifest, ctx, now) -> bool:
    """Approval gate for untrusted spend/deploy/delegate (final.md §13.4/§13.5).

    Stricter than `_has_approval_for_grant` for `mode == none` (auto-pass is NOT
    allowed -- explicit evidence is required, matching the Rust core's HEAD), but
    it still enforces the SAME reviewer-authorization and multi-party rules for
    `human`/`multi_party`. It must never accept an approval from an unauthorized
    reviewer.

    Parity note: the Rust core's HEAD escalates `mode == none` unconditionally
    (no configured reviewer can sign off). This port is fail-safe but marginally
    more permissive: `mode == none` is admitted only if an explicit action-bound
    approval artifact exists. Both are strictly safer than the pinned commit
    3e5625a (which auto-passed `mode == none`). Kept intentionally; see PR #22
    review thread.
    """
    approval = grant.get("approval", {}) or {}
    mode = approval.get("mode", "none")
    reviewers = approval.get("reviewer_ids", [])
    if mode == "none":
        return _explicit_action_evidence_exists(grant, manifest, ctx, now)
    if mode == "human":
        return any(_approval_from_reviewer(grant, manifest, ctx, now, r) for r in reviewers)
    if mode == "multi_party":
        return bool(reviewers) and all(
            _approval_from_reviewer(grant, manifest, ctx, now, r) for r in reviewers
        )
    raise AdmissionError(f"unknown approval mode: {mode}")


def _has_passed_simulation(manifest, ctx, now) -> bool:
    for sim in ctx.get("simulations", []):
        if (
            _ts(sim["passed_at"]) <= now
            and sim["action_id"] == manifest["action_id"]
            and sim["policy_version"] == ctx["policy_version"]
        ):
            return True
    return False


def _dangerous_untrusted(manifest: dict) -> bool:
    tainted = any(t in UNTRUSTED_TAINT for t in manifest.get("taint", []))
    return tainted and manifest["action_kind"] in DANGEROUS_UNTRUSTED_ACTIONS


def _is_hex_64(value: Any) -> bool:
    return isinstance(value, str) and len(value) == 64 and all(c in "0123456789abcdef" for c in value)


def _is_payment_action(manifest: dict) -> bool:
    return "payment" in manifest.get("expected_side_effects", []) or manifest["action_kind"] == "spend"


def _counterparty_policy_allows(policy: str, counterparty_ref: str, binding_hash: str) -> bool:
    if policy == "any":
        return True
    if ":" not in policy:
        return False
    kind, value = policy.split(":", 1)
    if kind == "exact":
        return counterparty_ref == value
    if kind == "prefix":
        return counterparty_ref.startswith(value)
    if kind == "hash":
        return binding_hash == value
    if kind == "allowlist":
        return counterparty_ref in {item.strip() for item in value.split(",") if item.strip()}
    return False


def _payment_authorized_by_mandate(manifest: dict, ctx: dict, now: datetime) -> str | None:
    amount = (manifest.get("requested_budget") or {}).get("max_payment_minor_units")
    if amount is None:
        return "payment action must declare its amount in requested_budget.max_payment_minor_units"

    intent = manifest.get("payment_intent")
    if intent is None:
        return "payment actions require a payment_intent"

    target = manifest.get("target") or {}
    if target.get("resource_kind") != "payment_rail":
        return "payment intent target must be a payment_rail"
    if target.get("resource_id") != intent.get("rail"):
        return "payment intent rail must match the manifest payment_rail target"
    if intent.get("amount_minor_units") == 0:
        return "payment intent amount must be non-zero"
    if intent.get("amount_minor_units") != amount:
        return "payment intent amount must match requested_budget.max_payment_minor_units"
    if manifest.get("idempotency_key") != intent.get("payment_idempotency_key"):
        return "payment intent idempotency key must match the manifest idempotency key"

    for field in (
        "mandate_id",
        "rail",
        "adapter_id",
        "asset",
        "counterparty_ref",
        "purpose",
        "payment_idempotency_key",
        "envelope_format",
    ):
        if not intent.get(field):
            return "payment intent fields must be non-empty"

    if not _is_hex_64(intent.get("counterparty_binding_hash")) or not _is_hex_64(intent.get("envelope_hash")):
        return "payment intent hashes must be lowercase 32-byte hex"
    if intent.get("envelope_expires_at") and _ts(intent["envelope_expires_at"]) <= now:
        return "payment intent envelope is expired"

    matching = [
        m for m in ctx.get("mandates", [])
        if m.get("mandate_id") == intent["mandate_id"]
        and m.get("session_id") == ctx["session_id"]
        and m.get("holder") == ctx["actor_id"]
    ]
    if not matching:
        return "payment requires an active PaymentMandate covering the amount for this session and holder"
    if len(matching) > 1:
        return "payment intent mandate_id must select exactly one mandate"

    mandate = matching[0]
    if _ts(mandate["expires_at"]) <= now:
        return "payment mandate is expired"
    if intent["rail"] != mandate.get("rail"):
        return "payment intent rail does not match mandate rail"
    if intent["asset"] != mandate.get("asset"):
        return "payment intent asset does not match mandate asset"
    if intent["amount_minor_units"] > mandate.get("max_minor_units", -1):
        return "payment intent amount exceeds mandate ceiling"
    if intent["purpose"] != mandate.get("purpose"):
        return "payment intent purpose does not match mandate purpose"
    if not _counterparty_policy_allows(
        mandate.get("counterparty_policy", ""),
        intent["counterparty_ref"],
        intent["counterparty_binding_hash"],
    ):
        return "payment intent counterparty is not allowed by mandate"
    if intent["payment_idempotency_key"] != mandate.get("idempotency_key"):
        return "payment intent idempotency key does not match mandate"
    allowed_adapters = mandate.get("allowed_adapter_ids") or []
    if allowed_adapters and intent["adapter_id"] not in allowed_adapters:
        return "payment intent adapter is not allowed by mandate"
    allowed_formats = mandate.get("allowed_envelope_formats") or []
    if allowed_formats and intent["envelope_format"] not in allowed_formats:
        return "payment intent envelope format is not allowed by mandate"

    return None


def admit(manifest: dict, ctx: dict) -> dict:
    """Return {'result', 'matched_rules', 'explanation'} for a manifest.

    Faithful port of PolicyEngine::admit. `ctx` carries now, actor_id,
    session_id, policy_version, grants[], approvals[], simulations[].
    """
    now = _ts(ctx["now"])
    matched: list[str] = []

    def deny(reason: str, result: str = "denied") -> dict:
        return {"result": result, "matched_rules": list(matched), "explanation": reason}

    if manifest["session_id"] != ctx["session_id"]:
        return deny("action manifest session does not match admission context session")
    matched.append("manifest_bound_to_context_session")

    required = manifest.get("required_grants", [])
    if not required:
        return deny("action manifests must name at least one required grant")
    matched.append("required_grants_present")

    if has_external_side_effect(manifest) and manifest.get("idempotency_key") is None:
        return deny("external side effects require an idempotency key before execution")
    matched.append("external_side_effect_idempotency")

    if "payment" in manifest.get("expected_side_effects", []) and manifest["action_kind"] != "spend":
        return deny("payment side effects must use the spend action kind")

    if _is_payment_action(manifest):
        reason = _payment_authorized_by_mandate(manifest, ctx, now)
        if reason:
            return deny(reason)
        matched.append("payment_authorized_by_mandate")

    matching = [g for g in ctx.get("grants", []) if g["grant_id"] in required]
    if len(matching) != len(required):
        return deny("one or more required grants are missing from the admission context")
    matched.append("required_grants_available")

    actor_id = ctx["actor_id"]
    if not all(grant_allows_manifest(g, manifest, now, actor_id) for g in matching):
        return deny(
            "available grants do not allow this action, target, risk, data class, or time window",
            result="needs_narrowed_grant",
        )
    matched.append("all_required_capabilities_allow_action")

    if _dangerous_untrusted(manifest) and not all(
        _untrusted_dangerous_approved(g, manifest, ctx, now) for g in matching
    ):
        return deny(
            "untrusted content cannot directly authorize spend, deploy, or delegation "
            "actions without action-bound approval",
            result="needs_approval",
        )
    matched.append("untrusted_instruction_policy_checked")

    for g in matching:
        approval = g.get("approval", {}) or {}
        if approval.get("mode", "none") != "none" and _risk_gte(
            manifest["risk_class"], approval.get("threshold_risk", "critical")
        ):
            if not _has_approval_for_grant(g, manifest, ctx, now):
                return deny(
                    "grant policy requires human approval for this risk class",
                    result="needs_approval",
                )
    matched.append("grant_approval_policy_checked")

    if (
        _risk_gte(manifest["risk_class"], "high")
        and has_external_side_effect(manifest)
        and not _has_passed_simulation(manifest, ctx, now)
    ):
        return deny(
            "high-risk external side effects require a passed simulation before execution",
            result="needs_simulation",
        )

    matched.append("admitted_by_capability_policy")
    return {
        "result": "allowed",
        "matched_rules": matched,
        "explanation": "action admitted by explicit active capability grant",
    }
