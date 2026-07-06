#!/usr/bin/env python3
"""Validate beaterOS bare-metal readiness manifest and host compatibility hints."""

from __future__ import annotations

import argparse
import json
import os
import platform
from dataclasses import dataclass
from pathlib import Path
from typing import Any


REPO_ROOT = Path(__file__).resolve().parent.parent
DEFAULT_MANIFEST_PATH = REPO_ROOT / "docs" / "engineering" / "bare-metal-readiness-manifest.json"
KNOWN_ACCELERATORS = frozenset(
    {
        "cpu",
        "cuda",
        "apple_gpu",
        "metal",
        "tpu",
        "npu",
        "lpu",
        "media_engine",
        "secure_enclave",
    },
)
KNOWN_STABILITY_TIERS = frozenset({"stable", "beta", "experimental"})
KNOWN_WORKLOAD_CLASSES = frozenset(
    {
        "control-plane",
        "policy-admission",
        "journaling",
        "tool-gateway",
        "benchmarks",
        "media",
        "apple-metal-integration",
        "batch-policy",
        "tooling",
        "cuda-scoring",
    },
)
KNOWN_MIGRATION_PHASES = frozenset({"runtime", "metal-ready"})
MIN_RESOURCE_CONSTRAINTS = {
    "min_cpu_cores": "cpu_cores",
    "min_memory_gib": "memory_gib",
    "min_storage_iops": "storage_iops",
    "min_memory_bandwidth_gbps": "memory_bandwidth_gbps",
    "min_gpu_mem_gib": "gpu_mem_gib",
    "min_pcie_bwl_gbps": "pcie_bwl_gbps",
}
MAX_RESOURCE_CONSTRAINTS = {
    "max_residual_latency_ms": "residual_latency_ms",
    "max_gpu_tolerance_temp_c": "gpu_temp_c",
}
ACCELERATOR_BOUND_CONSTRAINTS = {
    "apple_gpu": frozenset(
        {
            "min_gpu_mem_gib",
            "min_memory_bandwidth_gbps",
            "max_gpu_tolerance_temp_c",
        },
    ),
    "cuda": frozenset(
        {
            "min_gpu_mem_gib",
            "min_pcie_bwl_gbps",
            "max_gpu_tolerance_temp_c",
        },
    ),
}


def infer_migration_phase(
    migration_plan: list[dict[str, Any]],
    control_plane_lane: str | None = None,
) -> tuple[str, dict[str, Any]]:
    if not control_plane_lane:
        return "blocked", {
            "name": "blocked",
            "reason": "missing_control_plane_lane",
            "control_plane_ready": False,
            "ready_optional_lanes": [],
            "ready_mandatory_lanes": [],
            "mandatory_unrunnable_lanes": [
                entry.get("lane")
                for entry in migration_plan
                if entry.get("mandatory") and not entry.get("ready")
            ],
        }

    ready_optional_lanes: list[str] = []
    ready_mandatory_lanes: list[str] = []
    mandatory_unrunnable_lanes: list[str] = []
    control_plane_ready = False
    control_plane_found = False
    control_plane_blockers: list[str] = []

    for entry in migration_plan:
        lane_name = entry.get("lane")
        if not isinstance(lane_name, str):
            continue
        is_ready = bool(entry.get("ready", False))
        is_mandatory = bool(entry.get("mandatory", False))

        if is_ready and is_mandatory:
            ready_mandatory_lanes.append(lane_name)
        if is_ready and not is_mandatory:
            ready_optional_lanes.append(lane_name)
        if is_mandatory and not is_ready:
            mandatory_unrunnable_lanes.append(lane_name)

        if lane_name == control_plane_lane:
            control_plane_found = True
            control_plane_ready = is_ready
            blockers = entry.get("blockers")
            if isinstance(blockers, list):
                control_plane_blockers = [str(item) for item in blockers]

    if not control_plane_found:
        return "blocked", {
            "name": "blocked",
            "reason": "control_plane_lane_missing_in_plan",
            "control_plane_ready": False,
            "ready_optional_lanes": sorted(ready_optional_lanes),
            "ready_mandatory_lanes": sorted(ready_mandatory_lanes),
            "mandatory_unrunnable_lanes": sorted(mandatory_unrunnable_lanes),
        }

    if not control_plane_ready:
        return "blocked", {
            "name": "blocked",
            "reason": "control_plane_lane_not_ready",
            "control_plane_ready": False,
            "control_plane_blockers": control_plane_blockers,
            "ready_optional_lanes": sorted(ready_optional_lanes),
            "ready_mandatory_lanes": sorted(ready_mandatory_lanes),
            "mandatory_unrunnable_lanes": sorted(mandatory_unrunnable_lanes),
        }

    if ready_optional_lanes:
        return "metal-ready", {
            "name": "metal-ready",
            "reason": "at_least_one_non_mandatory_lane_ready",
            "control_plane_ready": True,
            "ready_optional_lanes": sorted(ready_optional_lanes),
            "ready_mandatory_lanes": sorted(ready_mandatory_lanes),
            "mandatory_unrunnable_lanes": sorted(mandatory_unrunnable_lanes),
        }

    return "runtime", {
        "name": "runtime",
        "reason": "control_plane_lane_ready_and_no_optional_lanes_runnable",
        "control_plane_ready": True,
        "ready_optional_lanes": [],
        "ready_mandatory_lanes": sorted(ready_mandatory_lanes),
        "mandatory_unrunnable_lanes": sorted(mandatory_unrunnable_lanes),
    }
OPTIMIZATION_WEIGHT = 0.01
OPTIMIZATION_SMALLER_BETTER_METRICS = frozenset(
    {
        "deadline_ms_p95",
        "max_batch_wait_ms_p95",
        "max_copy_gb_per_job_p95",
        "max_copy_gib_p95",
        "max_host_device_copy_ms_p95",
        "max_queue_delay_ms_p95",
        "max_queue_depth",
        "max_syscalls_p95",
        "max_thermal_throttle_duration_ms",
    },
)


def _ensure_non_empty_string_list(value: Any) -> list[str] | None:
    if not isinstance(value, list) or not value:
        return None
    if any(not isinstance(item, str) for item in value):
        return None
    if any(not item.strip() for item in value):  # pragma: no cover - defensive gate
        return None
    return [item for item in value]


def _collect_profile_lookup(profiles: list[Any]) -> dict[str, dict[str, Any]]:
    by_name: dict[str, dict[str, Any]] = {}
    for profile in profiles:
        if not isinstance(profile, dict):
            continue
        name = profile.get("name")
        if isinstance(name, str):
            by_name[name] = profile
    return by_name


@dataclass(frozen=True)
class HostContext:
    os_name: str
    arch: str
    cpu_cores: int | None = None
    memory_gib: float | None = None
    storage_iops: float | None = None
    memory_bandwidth_gbps: float | None = None
    gpu_mem_gib: float | None = None
    pcie_bwl_gbps: float | None = None
    residual_latency_ms: float | None = None
    gpu_temp_c: float | None = None
    accelerators: frozenset[str] = frozenset()


def _parse_depends_on(entry: Any) -> list[str] | None:
    if entry is None:
        return []
    if not isinstance(entry, list):
        return None
    if any(not isinstance(item, str) for item in entry):
        return None
    return [item for item in entry]


def parse_bool(value: object) -> bool:
    if isinstance(value, bool):
        return value
    if isinstance(value, str):
        lower = value.strip().lower()
        return lower in {"1", "true", "yes", "on"}
    return False


def parse_numeric(value: object) -> float | None:
    if isinstance(value, (int, float)) and not isinstance(value, bool):
        if value < 0:
            return None
        return float(value)
    if isinstance(value, str):
        stripped = value.strip()
        if not stripped:
            return None
        try:
            parsed = float(stripped)
        except ValueError:
            return None
        if parsed < 0:
            return None
        return parsed
    return None


def format_host_resource_value(value: float | int | None) -> str:
    return str(value) if value is not None else "unknown"


def _validate_resource_contract(profile_index: int, contract: Any) -> list[str]:
    errors: list[str] = []
    for key, _field in MIN_RESOURCE_CONSTRAINTS.items():
        if key not in contract:
            continue
        value = contract[key]
        if parse_numeric(value) is None:
            errors.append(
                f"profile[{profile_index}].resource_contract.{key} must be a non-negative number",
            )
    for key, _field in MAX_RESOURCE_CONSTRAINTS.items():
        if key not in contract:
            continue
        value = contract[key]
        if parse_numeric(value) is None:
            errors.append(
                f"profile[{profile_index}].resource_contract.{key} must be a non-negative number",
            )
    return errors


def _resource_contract_violations(
    contract: dict[str, Any],
    host: HostContext,
    *,
    strict_host_context: bool,
) -> list[str]:
    violations: list[str] = []
    for key, field_name in MIN_RESOURCE_CONSTRAINTS.items():
        required = contract.get(key)
        if required is None:
            continue
        required_value = parse_numeric(required)
        if required_value is None:
            violations.append(f"{key} invalid: {required!r}")
            continue
        actual = getattr(host, field_name)
        if actual is None:
            if strict_host_context:
                violations.append(f"{key} requires {field_name} but host did not report it")
            continue
        if actual < required_value:
            violations.append(
                f"{key} requires at least {required_value}, got {actual}",
            )

    for key, field_name in MAX_RESOURCE_CONSTRAINTS.items():
        required = contract.get(key)
        if required is None:
            continue
        required_value = parse_numeric(required)
        if required_value is None:
            violations.append(f"{key} invalid: {required!r}")
            continue
        actual = getattr(host, field_name)
        if actual is None:
            if strict_host_context:
                violations.append(f"{key} requires {field_name} but host did not report it")
            continue
        if actual > required_value:
            violations.append(
                f"{key} requires at most {required_value}, got {actual}",
            )

    return violations


def _optimization_metric_is_lower_better(metric: str) -> bool:
    if metric in OPTIMIZATION_SMALLER_BETTER_METRICS:
        return True
    return metric.startswith("max_") or metric.endswith("_ms_p95")


def _optimization_metric_is_higher_better(metric: str) -> bool:
    if metric in ("throughput_p95", "max_throughput_p95"):
        return True
    return metric.startswith("min_") or "throughput" in metric


def _lane_score(profile: dict[str, Any], host: HostContext) -> tuple[float, dict[str, Any]]:
    resource_contract = profile.get("resource_contract", {})
    resource_score = 0.0
    optimization_score = 0.0
    if not isinstance(resource_contract, dict):
        return 0.0, {"resource_score": 0.0, "optimization_score": 0.0, "penalties": []}

    penalties: list[str] = []
    for key, field_name in MIN_RESOURCE_CONSTRAINTS.items():
        required = parse_numeric(resource_contract.get(key))
        if required is None:
            continue
        actual = getattr(host, field_name)
        if actual in (None, 0):
            penalties.append(f"{key}: no host metric available")
            continue
        ratio = required / float(actual)
        resource_score += max(0.0, ratio)
        penalties.append(f"{key}: required={required}, host={actual}, penalty={ratio:.4f}")

    for key, field_name in MAX_RESOURCE_CONSTRAINTS.items():
        required = parse_numeric(resource_contract.get(key))
        if required is None:
            continue
        if required <= 0:
            continue
        actual = getattr(host, field_name)
        if actual is None:
            penalties.append(f"{key}: no host metric available")
            continue
        ratio = actual / required
        resource_score += max(0.0, ratio)
        penalties.append(f"{key}: actual={actual}, cap={required}, penalty={ratio:.4f}")

    optimization_targets = profile.get("optimization_targets", {})
    if isinstance(optimization_targets, dict):
        for key, value in optimization_targets.items():
            if key == "class":
                continue
            metric = parse_numeric(value)
            if metric is None:
                continue
            if _optimization_metric_is_lower_better(key):
                optimization_score += metric * OPTIMIZATION_WEIGHT
                penalties.append(f"{key}: lower-is-better={metric}, scaled={metric * OPTIMIZATION_WEIGHT:.4f}")
            elif _optimization_metric_is_higher_better(key):
                optimization_score += (1.0 / (1.0 + metric)) * OPTIMIZATION_WEIGHT
                penalties.append(
                    f"{key}: higher-is-better={metric}, scaled={1.0 / (1.0 + metric) * OPTIMIZATION_WEIGHT:.4f}",
                )

    total_score = resource_score + optimization_score
    return total_score, {
        "resource_score": resource_score,
        "optimization_score": optimization_score,
        "total_score": total_score,
        "penalties": penalties,
    }


def _lane_scores(manifest: dict[str, Any], host: HostContext) -> dict[str, dict[str, float | list[str]]]:
    profiles = manifest.get("profiles", [])
    profile_by_name = _collect_profile_lookup(profiles)
    architecture = manifest.get("architecture", {})
    lanes = architecture.get("lanes", []) if isinstance(architecture, dict) else []
    scores: dict[str, dict[str, float | list[str]]] = {}
    for lane in lanes:
        if not isinstance(lane, dict):
            continue
        lane_name = lane.get("name")
        if not isinstance(lane_name, str):
            continue
        profile_name = lane.get("profile")
        if not isinstance(profile_name, str):
            continue
        profile = profile_by_name.get(profile_name)
        if not isinstance(profile, dict):
            continue
        total_score, detail = _lane_score(profile, host)
        scores[lane_name] = {
            "score": total_score,
            "profile": profile_name,
            "resource_score": float(detail["resource_score"]),
            "optimization_score": float(detail["optimization_score"]),
            "total_score": float(detail["total_score"]),
            "penalties": detail["penalties"],  # type: ignore[arg-type]
        }
    return scores


def load_manifest(manifest_path: Path) -> dict[str, Any]:
    if not manifest_path.exists():
        raise ValueError(f"missing manifest: {manifest_path}")
    with manifest_path.open("r", encoding="utf-8") as handle:
        data = json.load(handle)
    if not isinstance(data, dict):
        raise TypeError("manifest must be a JSON object")
    return data


def load_host_profile(profile_path: Path | None = None) -> dict[str, Any]:
    if profile_path is None:
        return {}
    with profile_path.open("r", encoding="utf-8") as handle:
        payload = json.load(handle)
    if not isinstance(payload, dict):
        raise ValueError("host profile payload must be a JSON object")
    if "host" in payload and isinstance(payload["host"], dict):
        payload = payload["host"]
    if not isinstance(payload, dict):
        raise ValueError("host profile payload must be a JSON object")
    return payload


def _parse_profile_accelerators(raw: Any) -> frozenset[str]:
    accelerators = set()
    if isinstance(raw, dict):
        for key, value in raw.items():
            if parse_bool(value):
                accelerator = str(key).strip().lower()
                if accelerator:
                    accelerators.add(accelerator)
        return frozenset(accelerators)
    if isinstance(raw, (list, tuple)):
        for item in raw:
            if isinstance(item, str):
                accelerator = item.strip().lower()
                if accelerator:
                    accelerators.add(accelerator)
    elif isinstance(raw, str):
        for token in raw.split(","):
            accelerator = token.strip().lower()
            if accelerator:
                accelerators.add(accelerator)
    return frozenset(accelerators)


def validate_manifest(manifest: dict[str, Any]) -> list[str]:
    errors: list[str] = []

    if manifest.get("schema_version") != 1:
        errors.append("schema_version must be 1")

    profiles = manifest.get("profiles")
    if not isinstance(profiles, list) or not profiles:
        errors.append("profiles must be a non-empty array")
        return errors

    for index, profile in enumerate(profiles):
        if not isinstance(profile, dict):
            errors.append(f"profile[{index}] must be an object")
            continue

        required = {"name", "scope", "stability_tier", "target_os", "target_arch", "resource_contract", "accelerators"}
        missing = [name for name in required if name not in profile]
        if missing:
            errors.append(f"profile[{index}] missing required fields: {', '.join(sorted(missing))}")

        if "name" in profile and not isinstance(profile["name"], str):
            errors.append(f"profile[{index}].name must be a string")
        if "scope" in profile and not isinstance(profile["scope"], str):
            errors.append(f"profile[{index}].scope must be a string")
        stability = profile.get("stability_tier")
        if stability is not None and stability not in KNOWN_STABILITY_TIERS:
            errors.append(f"profile[{index}].stability_tier invalid: {stability}")

        target_os = profile.get("target_os")
        if not isinstance(target_os, list) or not target_os:
            errors.append(f"profile[{index}].target_os must be a non-empty array")
        elif any(not isinstance(item, str) for item in target_os):
            errors.append(f"profile[{index}].target_os must contain only strings")

        target_arch = profile.get("target_arch")
        if not isinstance(target_arch, list) or not target_arch:
            errors.append(f"profile[{index}].target_arch must be a non-empty array")
        elif any(not isinstance(item, str) for item in target_arch):
            errors.append(f"profile[{index}].target_arch must contain only strings")

        resource_contract = profile.get("resource_contract")
        if not isinstance(resource_contract, dict):
            errors.append(f"profile[{index}].resource_contract must be an object")
        elif not resource_contract:
            errors.append(f"profile[{index}].resource_contract must not be empty")
        else:
            errors.extend(_validate_resource_contract(index, resource_contract))

        accelerators = profile.get("accelerators")
        if not isinstance(accelerators, list) or not accelerators:
            errors.append(f"profile[{index}].accelerators must be a non-empty array")
            continue
        for accel_index, accel in enumerate(accelerators):
            if not isinstance(accel, dict):
                errors.append(f"profile[{index}].accelerators[{accel_index}] must be an object")
                continue
            kind = accel.get("kind")
            if not isinstance(kind, str):
                errors.append(
                    f"profile[{index}].accelerators[{accel_index}].kind must be a string",
                )
                continue
            if kind not in KNOWN_ACCELERATORS:
                errors.append(
                    f"profile[{index}].accelerators[{accel_index}].kind unknown: {kind}",
                )
            if "required" not in accel:
                errors.append(
                    f"profile[{index}].accelerators[{accel_index}] missing required field: required",
                )
            elif not isinstance(accel["required"], bool):
                errors.append(
                    f"profile[{index}].accelerators[{accel_index}].required must be boolean",
                )
            if not accel.get("fallback_strategy"):
                errors.append(
                    f"profile[{index}].accelerators[{accel_index}].fallback_strategy is required",
                )

        optimization_targets = profile.get("optimization_targets")
        if optimization_targets is not None and not isinstance(optimization_targets, dict):
            errors.append(f"profile[{index}].optimization_targets must be an object if present")

    profiles_by_name = _collect_profile_lookup(profiles)
    errors.extend(validate_architecture(manifest, profiles_by_name))

    return errors


def validate_architecture(manifest: dict[str, Any], profiles_by_name: dict[str, dict[str, Any]]) -> list[str]:
    errors: list[str] = []
    architecture = manifest.get("architecture")
    if not isinstance(architecture, dict):
        errors.append("architecture must be an object")
        return errors

    control_plane_lane = architecture.get("control_plane_lane")
    if not isinstance(control_plane_lane, str) or not control_plane_lane:
        errors.append("architecture.control_plane_lane must be a non-empty string")

    lanes = architecture.get("lanes")
    if not isinstance(lanes, list) or not lanes:
        errors.append("architecture.lanes must be a non-empty array")
        return errors

    migration_order = architecture.get("migration_order")
    if migration_order is None:
        errors.append("architecture.migration_order is required")
    elif not isinstance(migration_order, list) or not migration_order:
        errors.append("architecture.migration_order must be a non-empty array")

    lane_by_name: dict[str, dict[str, Any]] = {}
    lane_order_index_by_name: dict[str, int] = {}
    mandatory_lanes: set[str] = set()
    graph: dict[str, list[str]] = {}
    for index, lane in enumerate(lanes):
        if not isinstance(lane, dict):
            errors.append(f"architecture.lanes[{index}] must be an object")
            continue

        lane_name = lane.get("name")
        if not isinstance(lane_name, str) or not lane_name.strip():
            errors.append(f"architecture.lanes[{index}].name must be a non-empty string")
            continue
        if lane_name in lane_by_name:
            errors.append(f"architecture.lanes[{index}].name duplicated: {lane_name}")
            continue
        lane_by_name[lane_name] = lane
        lane_order_index_by_name[lane_name] = index

        profile_name = lane.get("profile")
        if not isinstance(profile_name, str) or not profile_name:
            errors.append(f"architecture.lanes[{index}].profile must be a non-empty string")
        elif profile_name not in profiles_by_name:
            errors.append(
                f"architecture.lanes[{index}].profile does not match any profile name: {profile_name}",
            )

        required_stability = lane.get("required_stability_tier")
        if required_stability is None:
            errors.append(f"architecture.lanes[{index}].required_stability_tier is required")
        elif required_stability not in KNOWN_STABILITY_TIERS:
            errors.append(
                f"architecture.lanes[{index}].required_stability_tier invalid: {required_stability}",
            )
        else:
            profile = profiles_by_name.get(profile_name)
            if isinstance(profile, dict):
                observed = profile.get("stability_tier")
                if observed != required_stability:
                    errors.append(
                        f"architecture.lanes[{index}].required_stability_tier "
                        f"({required_stability}) does not match profile.stability_tier ({observed})",
                    )

        workload_classes = _ensure_non_empty_string_list(lane.get("workload_classes"))
        if workload_classes is None:
            errors.append(
                f"architecture.lanes[{index}].workload_classes must be a non-empty array of non-empty strings",
            )
        else:
            unknown_classes = [entry for entry in workload_classes if entry not in KNOWN_WORKLOAD_CLASSES]
            if unknown_classes:
                errors.append(
                    f"architecture.lanes[{index}].workload_classes has unknown classes: "
                    f"{', '.join(sorted(unknown_classes))}",
                )

        if "mandatory" not in lane:
            errors.append(f"architecture.lanes[{index}] missing required field: mandatory")
        elif not isinstance(lane["mandatory"], bool):
            errors.append(f"architecture.lanes[{index}].mandatory must be boolean")
        elif lane["mandatory"] is True:
            mandatory_lanes.add(lane_name)

        depends_on = _parse_depends_on(lane.get("depends_on"))
        if depends_on is None:
            errors.append(f"architecture.lanes[{index}].depends_on must be an array of strings if present")
        else:
            graph[lane_name] = depends_on
            for dependency in depends_on:
                if dependency == lane_name:
                    errors.append(
                        f"architecture.lanes[{index}].depends_on cannot include self dependency: {dependency}",
                    )

        fallback_chain = _ensure_non_empty_string_list(lane.get("fallback_chain"))
        if fallback_chain is None:
            errors.append(
                f"architecture.lanes[{index}].fallback_chain must be a non-empty array of non-empty strings",
            )
        else:
            for fallback in fallback_chain:
                if fallback not in profiles_by_name:
                    errors.append(
                        f"architecture.lanes[{index}].fallback_chain references unknown profile: {fallback}",
                    )

    if isinstance(control_plane_lane, str) and control_plane_lane not in lane_by_name:
        errors.append(f"architecture.control_plane_lane does not match any lane name: {control_plane_lane}")

    if isinstance(migration_order, list):
        migration_order_seen: set[str] = set()
        for order_index, lane_name in enumerate(migration_order):
            if not isinstance(lane_name, str) or not lane_name:
                errors.append("architecture.migration_order entries must be non-empty strings")
                continue
            if lane_name in migration_order_seen:
                errors.append(f"architecture.migration_order[{order_index}] duplicates lane: {lane_name}")
                continue
            migration_order_seen.add(lane_name)
            if lane_name not in lane_by_name:
                errors.append(f"architecture.migration_order[{order_index}] references unknown lane: {lane_name}")
                continue

            for dependency in graph.get(lane_name, []):
                if dependency not in migration_order_seen:
                    errors.append(
                        f"architecture.migration_order violates dependency ordering: "
                        f"{lane_name} depends on {dependency} which appears later",
                    )

        for lane_name in lane_by_name:
            if lane_name not in migration_order_seen:
                errors.append(f"architecture.migration_order omits lane: {lane_name}")

        if migration_order:
            control_plane_order = control_plane_lane
            if control_plane_order in migration_order_seen and migration_order[0] != control_plane_order:
                errors.append(
                    "architecture.migration_order must start with architecture.control_plane_lane",
                )

    if not mandatory_lanes:
        errors.append("architecture requires at least one lane with mandatory=true")

    if isinstance(control_plane_lane, str) and control_plane_lane in lane_by_name:
        cp_lane = lane_by_name[control_plane_lane]
        if cp_lane.get("mandatory") is not True:
            errors.append("architecture.control_plane_lane must reference a mandatory lane")

    if control_plane_lane in lane_by_name and lane_by_name[control_plane_lane].get("mandatory") is True:
        cp_profile = lane_by_name[control_plane_lane].get("profile")
        cp_profile_entry = profiles_by_name.get(cp_profile, {}) if isinstance(cp_profile, str) else {}
        if cp_profile_entry.get("stability_tier") != "stable":
            errors.append("architecture.control_plane_lane profile must be stable")

    # Validate DAG dependencies after we've built graph.
    # Unknown dependencies are validated below to avoid false positives.
    for lane_name, depends in graph.items():
        for dependency in depends:
            if dependency not in lane_by_name:
                errors.append(f"architecture.lanes[{lane_order_index_by_name[lane_name]}].depends_on references unknown lane: {dependency}")

    visiting: dict[str, int] = {}
    def _has_cycle(node: str) -> bool:
        state = visiting.get(node, 0)
        if state == 1:
            return True
        if state == 2:
            return False
        visiting[node] = 1
        for dependency in graph.get(node, []):
            if _has_cycle(dependency):
                return True
        visiting[node] = 2
        return False

    for node in lane_by_name:
        if _has_cycle(node):
            errors.append("architecture.lanes contains dependency cycle")
            break

    return errors


def normalize_host_context(
    host_profile: dict[str, Any] | None = None,
    *,
    profile_only: bool = False,
) -> HostContext:
    # Keep deterministic local behavior for test envs where a test can inject
    # override markers in environment-like fixtures.
    # Use conservative values when runtime introspection is unavailable.
    host_profile = host_profile or {}
    raw_os = str(host_profile.get("os", "")).strip().lower()
    raw_arch = str(host_profile.get("arch", "")).strip().lower()
    if not raw_os:
        raw_os = "" if profile_only else os.environ.get("BEATEROS_HOST_OS", "").strip().lower()
    if not raw_arch:
        raw_arch = "" if profile_only else os.environ.get("BEATEROS_HOST_ARCH", "").strip().lower()
    if not raw_os:
        raw_os = "" if profile_only else platform.system().lower()
    if not raw_arch:
        raw_arch = "" if profile_only else platform.machine().lower()

    profile_accelerators = _parse_profile_accelerators(host_profile.get("accelerators"))
    profile_cpu_cores = parse_numeric(host_profile.get("cpu_cores"))
    if profile_cpu_cores is None:
        cpu_cores = None if profile_only else os.cpu_count()
    else:
        cpu_cores = int(profile_cpu_cores)

    profile_memory_gib = parse_numeric(host_profile.get("memory_gib"))
    profile_storage_iops = parse_numeric(host_profile.get("storage_iops"))
    profile_memory_bandwidth_gbps = parse_numeric(host_profile.get("memory_bandwidth_gbps"))
    profile_gpu_mem_gib = parse_numeric(host_profile.get("gpu_mem_gib"))
    profile_pcie_bwl_gbps = parse_numeric(host_profile.get("pcie_bwl_gbps"))
    profile_residual_latency_ms = parse_numeric(host_profile.get("residual_latency_ms"))
    profile_gpu_temp_c = parse_numeric(host_profile.get("gpu_temp_c"))

    memory_gib = profile_memory_gib
    if memory_gib is None and not profile_only:
        memory_gib = parse_numeric(os.environ.get("BEATEROS_HOST_MEMORY_GIB"))
    storage_iops = profile_storage_iops
    if storage_iops is None and not profile_only:
        storage_iops = parse_numeric(os.environ.get("BEATEROS_HOST_STORAGE_IOPS"))
    memory_bandwidth_gbps = profile_memory_bandwidth_gbps
    if memory_bandwidth_gbps is None and not profile_only:
        memory_bandwidth_gbps = parse_numeric(os.environ.get("BEATEROS_HOST_MEMORY_BANDWIDTH_GBPS"))
    gpu_mem_gib = profile_gpu_mem_gib
    if gpu_mem_gib is None and not profile_only:
        gpu_mem_gib = parse_numeric(os.environ.get("BEATEROS_HOST_GPU_MEM_GIB"))
    pcie_bwl_gbps = profile_pcie_bwl_gbps
    if pcie_bwl_gbps is None and not profile_only:
        pcie_bwl_gbps = parse_numeric(os.environ.get("BEATEROS_HOST_PCIE_BWL_GBPS"))
    residual_latency_ms = profile_residual_latency_ms
    if residual_latency_ms is None and not profile_only:
        residual_latency_ms = parse_numeric(os.environ.get("BEATEROS_HOST_RESIDUAL_LATENCY_MS"))
    gpu_temp_c = profile_gpu_temp_c
    if gpu_temp_c is None and not profile_only:
        gpu_temp_c = parse_numeric(os.environ.get("BEATEROS_HOST_GPU_TEMP_C"))

    accelerator_set = set()
    if profile_accelerators:
        accelerator_set.update(profile_accelerators)
    if not profile_only:
        if parse_bool(os.environ.get("BEATEROS_ACCELERATOR_CPU", "true")):
            accelerator_set.add("cpu")
        if parse_bool(os.environ.get("BEATEROS_ACCELERATOR_CUDA", "false")):
            accelerator_set.add("cuda")
        if parse_bool(os.environ.get("BEATEROS_ACCELERATOR_APPLE_GPU", "false")):
            accelerator_set.add("apple_gpu")
        if parse_bool(os.environ.get("BEATEROS_ACCELERATOR_TPU", "false")):
            accelerator_set.add("tpu")
        if parse_bool(os.environ.get("BEATEROS_ACCELERATOR_ENCLAVE", "false")):
            accelerator_set.add("secure_enclave")

    return HostContext(
        os_name=raw_os,
        arch=raw_arch,
        cpu_cores=cpu_cores,
        memory_gib=memory_gib,
        storage_iops=storage_iops,
        memory_bandwidth_gbps=memory_bandwidth_gbps,
        gpu_mem_gib=gpu_mem_gib,
        pcie_bwl_gbps=pcie_bwl_gbps,
        residual_latency_ms=residual_latency_ms,
        gpu_temp_c=gpu_temp_c,
        accelerators=frozenset(accelerator_set),
    )


def _resource_contract_allows_host(
    contract: dict[str, Any],
    host: HostContext,
    *,
    strict_host_context: bool,
) -> bool:
    return not _resource_contract_violations(
        contract,
        host,
        strict_host_context=strict_host_context,
    )


def profile_supports_host(
    profile: dict[str, Any],
    host: HostContext,
    *,
    strict_host_context: bool,
    detail: list[str] | None = None,
) -> bool:
    if host.os_name not in {os_name.lower() for os_name in profile.get("target_os", [])}:
        if detail is not None:
            detail.append(f"os={host.os_name} not in profile target_os")
        return False
    if host.arch not in {arch.lower() for arch in profile.get("target_arch", [])}:
        if detail is not None:
            detail.append(f"arch={host.arch} not in profile target_arch")
        return False

    resource_contract = profile.get("resource_contract", {})
    if not isinstance(resource_contract, dict):
        return False
    if not _resource_contract_allows_host(
        resource_contract,
        host,
        strict_host_context=strict_host_context,
    ):
        if detail is not None:
            detail.extend(_resource_contract_violations(resource_contract, host, strict_host_context=strict_host_context))
        return False

    for accel in profile.get("accelerators", []):
        kind = accel.get("kind")
        if not isinstance(kind, str):
            return False
        required = parse_bool(accel.get("required"))
        if required and kind not in host.accelerators:
            if detail is not None:
                detail.append(f"missing required accelerator: {kind}")
            return False
        constrained_keys = ACCELERATOR_BOUND_CONSTRAINTS.get(kind, frozenset())
        if not required and kind not in host.accelerators and constrained_keys.intersection(resource_contract):
            if detail is not None:
                detail.append(f"missing optional accelerator for constrained profile: {kind}")
            return False

    return True


def lane_supports_host(
    lane: dict[str, Any],
    host: HostContext,
    profile_by_name: dict[str, dict[str, Any]],
    *,
    strict_host_context: bool = False,
    detail: list[str] | None = None,
) -> bool:
    profile_name = lane.get("profile")
    if not isinstance(profile_name, str):
        if detail is not None:
            detail.append("lane missing required profile field")
        return False
    profile = profile_by_name.get(profile_name)
    if not isinstance(profile, dict):
        if detail is not None:
            detail.append(f"lane references unknown profile: {profile_name}")
        return False
    return profile_supports_host(
        profile,
        host,
        strict_host_context=strict_host_context,
        detail=detail,
    )


def build_migration_plan(
    manifest: dict[str, Any],
    host: HostContext,
    *,
    strict_host_context: bool = False,
) -> list[dict[str, Any]]:
    profiles = manifest.get("profiles", [])
    profile_lookup = _collect_profile_lookup(profiles)
    architecture = manifest.get("architecture", {})
    lanes = architecture.get("lanes", []) if isinstance(architecture, dict) else []
    migration_order = architecture.get("migration_order", [])
    lane_by_name = {
        lane.get("name"): lane
        for lane in lanes
        if isinstance(lane, dict) and isinstance(lane.get("name"), str)
    }

    readiness: list[dict[str, Any]] = []
    ready_status: dict[str, bool] = {}
    order = migration_order if isinstance(migration_order, list) else list(lane_by_name.keys())
    for lane_name in order:
        if not isinstance(lane_name, str):
            continue
        lane = lane_by_name.get(lane_name)
        if not isinstance(lane, dict):
            continue
        blockers: list[str] = []
        matches_host = lane_supports_host(
            lane,
            host,
            profile_by_name=profile_lookup,
            strict_host_context=strict_host_context,
            detail=blockers,
        )
        depends = _parse_depends_on(lane.get("depends_on"))
        if depends is None:
            depends = []
        dependencies_ready = all(ready_status.get(dependency, False) for dependency in depends)
        ready_status[lane_name] = bool(matches_host and dependencies_ready)
        readiness.append(
            {
                "lane": lane_name,
                "profile": lane.get("profile"),
                "mandatory": bool(lane.get("mandatory")),
                "workload_classes": lane.get("workload_classes", []),
                "depends_on": depends,
                "matches_host": matches_host,
                "ready": ready_status[lane_name],
                "blockers": blockers,
                "fallback_chain": lane.get("fallback_chain", []),
            },
        )

    return readiness


def lane_status_for_workloads(plan: list[dict[str, Any]]) -> dict[str, list[str]]:
    classes: dict[str, list[str]] = {}
    for lane in plan:
        if not lane.get("matches_host") or not lane.get("ready"):
            continue
        workload_classes = lane.get("workload_classes", [])
        if not isinstance(workload_classes, list):
            continue
        for workload in workload_classes:
            if not isinstance(workload, str) or not workload:
                continue
            classes.setdefault(workload, []).append(str(lane.get("lane")))
    return classes


def preferred_workload_routes(
    plan: list[dict[str, Any]],
    lane_scores: dict[str, float],
) -> dict[str, dict[str, Any]]:
    routing = lane_status_for_workloads(plan)
    lane_order: dict[str, int] = {
        str(entry.get("lane")): index for index, entry in enumerate(plan) if isinstance(entry.get("lane"), str)
    }
    routes: dict[str, dict[str, Any]] = {}
    for workload, lanes in routing.items():
        if not lanes:
            continue

        def _route_score(lane_name: str) -> tuple[float, int]:
            return (lane_scores.get(lane_name, float("inf")), lane_order.get(lane_name, 1_000))

        selected = min(lanes, key=_route_score)
        routes[workload] = {
            "lane": selected,
            "score": lane_scores.get(selected),
        }
    return routes


def check(
    manifest: dict[str, Any],
    host_profile: dict[str, Any] | None = None,
    host_check: bool = False,
    require_profile: str | None = None,
    require_lane: str | None = None,
    require_workload_classes: list[str] | None = None,
    require_workload_routes: dict[str, str] | None = None,
    require_migration_phase: str | None = None,
    *,
    strict_host_context: bool = False,
    host_profile_is_authoritative: bool = False,
    silent: bool = False,
) -> int:
    errors = validate_manifest(manifest)

    def _log(message: str) -> None:
        if not silent:
            print(message)

    if errors:
        for err in errors:
            _log(f"ERROR: {err}")
        return 1

    architecture = manifest.get("architecture")
    if not isinstance(architecture, dict):
        architecture = {}
    lanes = architecture.get("lanes", [])
    if not isinstance(lanes, list):
        lanes = []

    if not host_check:
        _log("PASS: bare-metal readiness manifest schema is valid")
        return 0

    effective_profile_only = host_profile_is_authoritative or bool(strict_host_context and host_profile)
    host = normalize_host_context(
        host_profile=host_profile,
        profile_only=effective_profile_only,
    )
    profiles = manifest.get("profiles", [])
    matching_profiles = [
        p["name"]
        for p in profiles
        if profile_supports_host(
            p,
            host,
            strict_host_context=strict_host_context,
        )
    ]
    profile_lookup = _collect_profile_lookup(profiles)
    lane_by_name = {lane.get("name"): lane for lane in lanes if isinstance(lane, dict) and isinstance(lane.get("name"), str)}
    migration_plan = build_migration_plan(
        manifest,
        host,
        strict_host_context=strict_host_context,
    )
    matching_lanes = [
        lane_name
        for lane_name in lane_by_name
        if lane_supports_host(
            lane_by_name[lane_name],
            host,
            profile_by_name=profile_lookup,
            strict_host_context=strict_host_context,
        )
    ]
    runnable_lanes = [entry["lane"] for entry in migration_plan if entry["ready"]]
    lane_scores = {lane_name: info["score"] for lane_name, info in _lane_scores(manifest, host).items() if isinstance(info["score"], float)}
    workload_routing = lane_status_for_workloads(migration_plan)
    preferred_routes = preferred_workload_routes(migration_plan, lane_scores)
    if not matching_profiles:
        _log("ERROR: no manifest profile supports the current host context")
        return 1

    architecture = manifest.get("architecture")
    control_plane_lane = None
    if isinstance(architecture, dict):
        candidate_lane = architecture.get("control_plane_lane")
        if isinstance(candidate_lane, str) and candidate_lane.strip():
            control_plane_lane = candidate_lane
    migration_phase, migration_phase_metadata = infer_migration_phase(migration_plan, control_plane_lane)

    if require_profile is not None and require_profile not in matching_profiles:
        _log(f"ERROR: required profile not supported on current host: {require_profile}")
        return 1

    if require_migration_phase == "runtime" and migration_phase != "runtime":
        _log(
            f"ERROR: required migration phase is runtime, but host is in '{migration_phase}' ("
            "expected runtime)",
        )
        return 1
    if require_migration_phase == "metal-ready" and migration_phase != "metal-ready":
        _log("ERROR: required migration phase is metal-ready, but no non-mandatory lane is runnable")
        return 1

    if require_lane is not None and require_lane not in runnable_lanes:
        _log(f"ERROR: required architecture lane is not runnable on current host: {require_lane}")
        return 1

    if require_workload_classes:
        for workload in require_workload_classes:
            if workload not in workload_routing:
                _log(f"ERROR: required workload class not routeable on current host: {workload}")
                return 1

    if require_workload_routes:
        for workload, required_lane in require_workload_routes.items():
            route = preferred_routes.get(workload)
            if route is None:
                _log(f"ERROR: workload route for {workload} is unavailable on current host")
                return 1
            if route.get("lane") != required_lane:
                _log(
                    f"ERROR: workload route for {workload} expects {required_lane}, "
                    f"but optimal host route is {route.get('lane')}",
                )
                return 1

    not_ready_mandatory = [entry["lane"] for entry in migration_plan if entry["mandatory"] and not entry["ready"]]
    if not_ready_mandatory:
        _log(
            "ERROR: mandatory architecture lane(s) are not runnable on current host: "
            + ", ".join(not_ready_mandatory),
        )
        return 1

    _log(f"INFO: host context = os={host.os_name}, arch={host.arch}, cpus={host.cpu_cores or 'unknown'}")
    _log(
        "INFO: host resource snapshot = "
        f"memory_gib={format_host_resource_value(host.memory_gib)}, "
        f"storage_iops={format_host_resource_value(host.storage_iops)}, "
        f"memory_bandwidth_gbps={format_host_resource_value(host.memory_bandwidth_gbps)}, "
        f"gpu_mem_gib={format_host_resource_value(host.gpu_mem_gib)}, "
        f"pcie_bwl_gbps={format_host_resource_value(host.pcie_bwl_gbps)}, "
        f"residual_latency_ms={format_host_resource_value(host.residual_latency_ms)}, "
        f"gpu_temp_c={format_host_resource_value(host.gpu_temp_c)}",
    )
    _log(f"INFO: matching profiles = {', '.join(matching_profiles)}")
    if matching_lanes:
        _log(f"INFO: matching lanes = {', '.join(matching_lanes)}")
    _log(f"INFO: migration phase = {migration_phase}")
    if migration_phase_metadata:
        optional_lanes = ", ".join(migration_phase_metadata.get("ready_optional_lanes", []))
        unrunnable_mandatory = ", ".join(migration_phase_metadata.get("mandatory_unrunnable_lanes", []))
        reason = migration_phase_metadata.get("reason", "unknown")
        _log(f"INFO: migration details: optional_ready=[{optional_lanes}], mandatory_unrunnable=[{unrunnable_mandatory}], reason={reason}")
    _log("PASS: host context is represented in the bare-metal readiness manifest")
    return 0


def run_and_dump_json(args: argparse.Namespace, manifest: dict[str, Any]) -> int:
    try:
        host_profile = load_host_profile(args.host_profile)
    except (OSError, ValueError, json.JSONDecodeError) as exc:
        print(f"ERROR: {exc}")
        return 1

    require_lane = args.require_lane
    if args.require_control_plane_lane:
        architecture = manifest.get("architecture")
        if not isinstance(architecture, dict):
            print("ERROR: architecture must be an object when --require-control-plane-lane is set")
            return 1
        control_plane_lane = architecture.get("control_plane_lane")
        if not isinstance(control_plane_lane, str) or not control_plane_lane:
            print("ERROR: architecture.control_plane_lane must be a non-empty string when --require-control-plane-lane is set")
            return 1
        require_lane = control_plane_lane

    host_profile_is_authoritative = bool(args.host_profile)
    strict_host_context = bool(args.strict_host_context or host_profile_is_authoritative)

    if not args.report:
        route_requirements: dict[str, str] = {}
        for requirement in args.require_workload_route:
            workload, sep, route = requirement.partition("=")
            if not sep or not workload or not route:
                print(f"ERROR: invalid --require-workload-route value: {requirement}")
                return 1
            route_requirements[workload] = route
        return check(
            manifest,
            host_profile=host_profile,
            strict_host_context=strict_host_context,
            host_profile_is_authoritative=host_profile_is_authoritative,
            host_check=args.check_host
            or args.require_profile is not None
            or args.require_lane is not None
            or args.require_control_plane_lane
            or bool(args.require_workload_class)
            or bool(args.require_workload_route),
            require_profile=args.require_profile,
            require_lane=require_lane,
            require_workload_routes=route_requirements,
            require_workload_classes=list(args.require_workload_class),
            require_migration_phase=args.require_migration_phase,
        )

    host = normalize_host_context(
        host_profile=host_profile,
        profile_only=host_profile_is_authoritative,
    )
    profiles = manifest.get("profiles", [])
    matching_profiles = [
        p["name"]
        for p in profiles
        if profile_supports_host(p, host, strict_host_context=strict_host_context)
    ]
    architecture = manifest.get("architecture", {})
    lanes = architecture.get("lanes", []) if isinstance(architecture, dict) else []
    profile_lookup = _collect_profile_lookup(profiles)
    matching_lanes = []
    for lane in lanes:
        if not isinstance(lane, dict):
            continue
        if lane_supports_host(
            lane,
            host,
            profile_by_name=profile_lookup,
            strict_host_context=strict_host_context,
        ):
            lane_name = lane.get("name")
            if isinstance(lane_name, str):
                matching_lanes.append(lane_name)

    route_requirements = {}
    for requirement in args.require_workload_route:
        workload, sep, route = requirement.partition("=")
        if not sep or not workload or not route:
            print(f"ERROR: invalid --require-workload-route value: {requirement}")
            return 1
        route_requirements[workload] = route

    payload = {
        "schema_version": manifest.get("schema_version"),
        "profiles": len(profiles),
        "supported_profiles": matching_profiles,
        "host": {
            "os": host.os_name,
            "arch": host.arch,
            "cpu_cores": host.cpu_cores,
            "memory_gib": host.memory_gib,
            "storage_iops": host.storage_iops,
            "memory_bandwidth_gbps": host.memory_bandwidth_gbps,
            "gpu_mem_gib": host.gpu_mem_gib,
            "pcie_bwl_gbps": host.pcie_bwl_gbps,
            "residual_latency_ms": host.residual_latency_ms,
            "gpu_temp_c": host.gpu_temp_c,
            "accelerators": sorted(host.accelerators),
        },
        "architecture": {
            "lanes": len(lanes),
            "supported_lanes": sorted(matching_lanes),
            "control_plane_lane": architecture.get("control_plane_lane") if isinstance(architecture, dict) else None,
            "mandatory_unrunnable_lanes": [],
            "migration_plan": [],
            "lane_scores": {},
            "workload_routing": {},
            "preferred_workload_routes": {},
        },
    }
    migration_plan = build_migration_plan(
        manifest,
        host,
        strict_host_context=strict_host_context,
    )
    lane_scores = _lane_scores(manifest, host)
    payload["architecture"]["migration_plan"] = migration_plan
    payload["architecture"]["lane_scores"] = lane_scores
    workload_routing = lane_status_for_workloads(migration_plan)
    payload["architecture"]["workload_routing"] = workload_routing
    payload["architecture"]["preferred_workload_routes"] = preferred_workload_routes(
        migration_plan,
        {lane_name: float(score["score"]) for lane_name, score in lane_scores.items()},
    )
    payload["architecture"]["mandatory_unrunnable_lanes"] = [
        entry["lane"] for entry in migration_plan if entry["mandatory"] and not entry["ready"]
    ]
    architecture_data = manifest.get("architecture")
    control_plane_lane = None
    if isinstance(architecture_data, dict):
        candidate_lane = architecture_data.get("control_plane_lane")
        if isinstance(candidate_lane, str) and candidate_lane.strip():
            control_plane_lane = candidate_lane
    phase, phase_metadata = infer_migration_phase(migration_plan, control_plane_lane)
    payload["architecture"]["migration_phase"] = {
        "name": phase,
        **phase_metadata,
    }
    print(json.dumps(payload, sort_keys=True))
    code = check(
        manifest,
        host_profile=host_profile,
        host_check=(
            args.check_host
            or args.require_profile is not None
            or args.require_lane is not None
            or args.require_control_plane_lane
            or bool(args.require_workload_class)
            or bool(args.require_workload_route)
        ),
        require_profile=args.require_profile,
        require_lane=require_lane,
        require_workload_routes=route_requirements,
        require_workload_classes=list(args.require_workload_class),
        require_migration_phase=args.require_migration_phase,
        strict_host_context=strict_host_context,
        host_profile_is_authoritative=host_profile_is_authoritative,
        silent=bool(args.report_only),
    )
    return code


def main() -> int:
    parser = argparse.ArgumentParser(description="Check bare-metal readiness manifest.")
    parser.add_argument(
        "--manifest",
        type=Path,
        default=DEFAULT_MANIFEST_PATH,
        help="path to the readiness manifest JSON",
    )
    parser.add_argument(
        "--check-host",
        action="store_true",
        help="require that at least one manifest profile supports the current host",
    )
    parser.add_argument(
        "--require-profile",
        metavar="NAME",
        help="require the specified profile to be compatible with the current host",
    )
    parser.add_argument(
        "--require-lane",
        metavar="NAME",
        help="require the specified architecture lane to be compatible with the current host",
    )
    parser.add_argument(
        "--require-workload-class",
        action="append",
        default=[],
        metavar="CLASS",
        help="require the specified workload class to have at least one ready lane",
    )
    parser.add_argument(
        "--require-workload-route",
        action="append",
        default=[],
        metavar="CLASS=LANE",
        help="require optimal route for workload class to target lane (e.g. policy-admission=portable-control-plane)",
    )
    parser.add_argument(
        "--host-profile",
        type=Path,
        help="read a host snapshot file and use it for readiness matching",
    )
    parser.add_argument(
        "--require-control-plane-lane",
        action="store_true",
        help="require the manifest's control_plane_lane to be compatible with the current host",
    )
    parser.add_argument(
        "--report",
        action="store_true",
        help="print JSON host-profile coverage report",
    )
    parser.add_argument(
        "--report-only",
        action="store_true",
        help="with --report, suppress text output and emit JSON only",
    )
    parser.add_argument(
        "--require-migration-phase",
        choices=sorted(KNOWN_MIGRATION_PHASES),
        help="require the inferred migration phase: runtime or metal-ready",
    )
    parser.add_argument(
        "--strict-host-context",
        action="store_true",
        help="treat missing host telemetry as a hard compatibility failure",
    )
    args = parser.parse_args()
    try:
        manifest = load_manifest(args.manifest)
    except (FileNotFoundError, ValueError, TypeError, json.JSONDecodeError) as exc:
        print(f"ERROR: {exc}")
        return 1
    return run_and_dump_json(args, manifest)


if __name__ == "__main__":
    raise SystemExit(main())
