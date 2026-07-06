#!/usr/bin/env python3
"""Run deterministic matrix checks for beaterOS bare-metal readiness."""

from __future__ import annotations

import argparse
import importlib.util
import io
import json
import sys
from contextlib import redirect_stdout
from pathlib import Path
from typing import Any

BASE_DIR = Path(__file__).resolve().parent
DEFAULT_MANIFEST = BASE_DIR.parent / "docs" / "engineering" / "bare-metal-readiness-manifest.json"
DEFAULT_MATRIX_PATH = BASE_DIR.parent / "docs" / "engineering" / "bare-metal-e2e-matrix.json"
CHECKER_PATH = BASE_DIR / "check-bare-metal-readiness.py"

if not CHECKER_PATH.exists():
    raise SystemExit(f"missing checker script: {CHECKER_PATH}")

SPEC = importlib.util.spec_from_file_location("check_bare_metal_readiness", str(CHECKER_PATH))
if SPEC is None or SPEC.loader is None:
    raise SystemExit(f"unable to load checker module from {CHECKER_PATH}")
checker = importlib.util.module_from_spec(SPEC)
sys.modules["check_bare_metal_readiness"] = checker
SPEC.loader.exec_module(checker)

DEFAULT_MATRIX: list[dict[str, Any]] = [
    {
        "name": "portable-control-plane-linux",
        "host": {
            "os": "linux",
            "arch": "x86_64",
            "cpu_cores": 8,
            "memory_gib": 16,
            "storage_iops": 6000,
            "residual_latency_ms": 120,
            "accelerators": ["cpu"],
        },
        "require_profile": "portable-host-control-plane",
        "require_control_plane_lane": True,
        "require_workload_routes": {
            "policy-admission": "portable-control-plane",
        },
        "require_workload_classes": ["policy-admission"],
    },
    {
        "name": "portable-control-plane-darwin",
        "host": {
            "os": "darwin",
            "arch": "arm64",
            "cpu_cores": 8,
            "memory_gib": 12,
            "storage_iops": 6000,
            "residual_latency_ms": 120,
            "accelerators": ["cpu"],
        },
        "require_profile": "portable-host-control-plane",
        "require_control_plane_lane": True,
        "require_workload_routes": {
            "policy-admission": "portable-control-plane",
        },
        "require_workload_classes": ["policy-admission"],
    },
    {
        "name": "linux-cuda-tooling-route",
        "host": {
            "os": "linux",
            "arch": "x86_64",
            "cpu_cores": 16,
            "memory_gib": 32,
            "storage_iops": 12000,
            "residual_latency_ms": 80,
            "accelerators": ["cpu", "cuda"],
            "gpu_mem_gib": 24,
            "pcie_bwl_gbps": 24,
        },
        "require_profile": "linux-cuda-scored-host",
        "require_workload_routes": {
            "tooling": "linux-cuda-lane",
        },
        "require_workload_classes": ["tooling"],
    },
    {
        "name": "apple-metal-media-route",
        "host": {
            "os": "darwin",
            "arch": "arm64",
            "cpu_cores": 12,
            "memory_gib": 16,
            "storage_iops": 12000,
            "residual_latency_ms": 80,
            "memory_bandwidth_gbps": 80,
            "gpu_temp_c": 70,
            "accelerators": ["cpu", "apple_gpu"],
        },
        "require_workload_routes": {
            "media": "apple-metal-lane",
        },
        "require_workload_classes": ["media"],
    },
    {
        "name": "portable-control-plane-linux-mandatory",
        "host": {
            "os": "linux",
            "arch": "x86_64",
            "cpu_cores": 4,
            "memory_gib": 4,
            "storage_iops": 6000,
            "residual_latency_ms": 120,
            "accelerators": ["cpu"],
        },
        "require_lane": "portable-control-plane",
        "require_workload_routes": {
            "policy-admission": "portable-control-plane",
        },
        "require_workload_classes": ["policy-admission"],
    },
    {
        "name": "portable-lane-not-runnable-on-weak-cuda-host",
        "host": {
            "os": "linux",
            "arch": "x86_64",
            "cpu_cores": 4,
            "memory_gib": 6,
            "storage_iops": 6000,
            "residual_latency_ms": 120,
            "accelerators": ["cpu", "cuda"],
        },
        "require_lane": "linux-cuda-lane",
        "expected_result": "fail",
        "expected_failure_contains": "required architecture lane is not runnable",
    },
]


def _load_case_payload(path: Path) -> Any:
    with path.open("r", encoding="utf-8") as handle:
        return json.load(handle)


def _coerce_cases(payload: Any) -> list[dict[str, Any]]:
    if isinstance(payload, list):
        cases = payload
    elif isinstance(payload, dict):
        if "cases" in payload:
            cases = payload["cases"]
        elif "matrix" in payload:
            cases = payload["matrix"]
        else:
            raise ValueError("matrix payload must include a 'cases' array")
    else:
        raise ValueError("matrix must be a JSON array or object")

    if not isinstance(cases, list):
        raise ValueError("matrix 'cases' must be a list")
    return [case for case in cases if isinstance(case, dict)]


def _normalize_require_workload_route(
    raw_routes: Any,
) -> dict[str, str]:
    if raw_routes is None:
        return {}
    if not isinstance(raw_routes, dict):
        raise ValueError("require_workload_routes must be an object")
    normalized = {str(k): str(v) for k, v in raw_routes.items()}
    return normalized


def _normalize_case_requirements(case: dict[str, Any]) -> dict[str, Any]:
    normalized = dict(case)

    if normalized.get("enabled") is None:
        normalized["enabled"] = True
    elif not isinstance(normalized["enabled"], bool):
        raise ValueError("enabled must be true or false if present")

    if normalized["enabled"] is False:
        return normalized

    require_workload_routes = _normalize_require_workload_route(
        normalized.get("require_workload_routes"),
    )
    normalized["require_workload_routes"] = require_workload_routes

    workload_classes = normalized.get("require_workload_classes", [])
    if workload_classes is None:
        normalized["require_workload_classes"] = []
    elif not isinstance(workload_classes, list) or any(not isinstance(item, str) for item in workload_classes):
        raise ValueError("require_workload_classes must be a list of strings")

    if normalized.get("expected_result") is None:
        normalized["expected_result"] = "pass"
    elif normalized["expected_result"] not in {"pass", "fail"}:
        raise ValueError("expected_result must be pass or fail")

    host = normalized.get("host")
    if not isinstance(host, dict):
        raise ValueError("host must be an object")

    return normalized


def _resolve_control_plane_lane(manifest: dict[str, Any]) -> str | None:
    architecture = manifest.get("architecture")
    if isinstance(architecture, dict):
        lane = architecture.get("control_plane_lane")
        if isinstance(lane, str) and lane.strip():
            return lane.strip()
    return None


def _collect_architecture_metadata(
    manifest: dict[str, Any],
) -> tuple[set[str], set[str], set[str]]:
    architecture = manifest.get("architecture", {})
    lanes = architecture.get("lanes", []) if isinstance(architecture, dict) else []
    lane_names: set[str] = set()
    known_workloads: set[str] = set()
    for lane in lanes:
        if not isinstance(lane, dict):
            continue
        lane_name = lane.get("name")
        if isinstance(lane_name, str):
            lane_names.add(lane_name)
        workload_classes = lane.get("workload_classes", [])
        if isinstance(workload_classes, list):
            for workload in workload_classes:
                if isinstance(workload, str):
                    known_workloads.add(workload)
    profile_names = {
        profile.get("name")
        for profile in manifest.get("profiles", [])
        if isinstance(profile, dict) and isinstance(profile.get("name"), str)
    }
    return (
        {profile_name for profile_name in profile_names if profile_name is not None},
        lane_names,
        known_workloads,
    )


def _validate_cases(case_matrix: list[dict[str, Any]], manifest: dict[str, Any]) -> list[str]:
    errors: list[str] = []
    profile_names, lane_names, workload_names = _collect_architecture_metadata(manifest)
    for index, case in enumerate(case_matrix):
        case_name = case.get("name", f"case[{index}]")
        try:
            normalized = _normalize_case_requirements(case)
        except ValueError as exc:
            errors.append(f"{case_name}: {exc}")
            continue

        raw_routes = normalized.get("require_workload_routes", {})
        try:
            _normalize_require_workload_route(raw_routes)
        except ValueError:
            errors.append(f"{case_name}: require_workload_routes must be an object of workload->lane")
            raw_routes = None

        require_workload_classes = normalized.get("require_workload_classes", [])
        if require_workload_classes is not None and not isinstance(require_workload_classes, list):
            errors.append(f"{case_name}: require_workload_classes must be a list of strings")
        elif isinstance(require_workload_classes, list):
            for workload in require_workload_classes:
                if not isinstance(workload, str):
                    errors.append(f"{case_name}: require_workload_classes contains a non-string value")
                    break
                if workload not in workload_names:
                    errors.append(f"{case_name}: unknown required workload class: {workload}")

        if normalized.get("require_profile") is not None and normalized.get("require_profile") not in profile_names:
            errors.append(
                f"{case_name}: require_profile does not match any manifest profile: {normalized.get('require_profile')}"
            )

        if normalized.get("require_lane") is not None and normalized.get("require_lane") not in lane_names:
            errors.append(
                f"{case_name}: require_lane does not match any manifest lane: {normalized.get('require_lane')}"
            )

        if normalized.get("require_control_plane_lane"):
            control_plane_lane = _resolve_control_plane_lane(manifest)
            if control_plane_lane is None:
                errors.append(f"{case_name}: manifest missing architecture.control_plane_lane")
            elif normalized.get("require_lane") is not None and normalized.get("require_lane") != control_plane_lane:
                errors.append(
                    f"{case_name}: require_lane={normalized.get('require_lane')} "
                    f"conflicts with require_control_plane_lane={control_plane_lane}"
                )

        migration_phase = normalized.get("require_migration_phase")
        if migration_phase is not None and migration_phase not in checker.KNOWN_MIGRATION_PHASES:
            errors.append(
                f"{case_name}: require_migration_phase must be one of: "
                f"{', '.join(sorted(checker.KNOWN_MIGRATION_PHASES))}",
            )

        if raw_routes is not None and isinstance(raw_routes, dict):
            for workload, lane in raw_routes.items():
                if not isinstance(workload, str) or not workload:
                    errors.append(f"{case_name}: workload route key must be a string")
                if not isinstance(lane, str) or not lane:
                    errors.append(f"{case_name}: workload route value for {workload!r} must be a string")
                elif lane not in lane_names:
                    errors.append(f"{case_name}: workload route target lane not in manifest: {lane}")

    return errors


def _run_case(case: dict[str, Any], manifest: dict[str, Any]) -> tuple[bool, str, dict[str, Any]]:
    normalized = _normalize_case_requirements(case)
    if not normalized.get("enabled", True):
        name = normalized.get("name", "matrix-case")
        return True, f"{name}: skipped", {"name": name, "expected": normalized["expected_result"], "result": "skipped"}

    host = normalized["host"]
    expected_pass = normalized["expected_result"] == "pass"
    expected_fragment = normalized.get("expected_failure_contains")
    name = normalized.get("name", "matrix-case")
    require_control_plane_lane = bool(normalized.get("require_control_plane_lane"))
    require_lane = normalized.get("require_lane")
    if require_control_plane_lane:
        cp_lane = _resolve_control_plane_lane(manifest)
        if cp_lane is None:
            return (
                False,
                f"{name}: manifest missing architecture.control_plane_lane",
                {
                    "name": name,
                    "expected": normalized["expected_result"],
                    "result": "error",
                    "error": "manifest missing architecture.control_plane_lane",
                },
            )
        if require_lane is not None and require_lane != cp_lane:
            return (
                False,
                f"{name}: conflicting require_lane={require_lane} and require_control_plane_lane",
                {
                    "name": name,
                    "expected": normalized["expected_result"],
                    "result": "error",
                    "error": "conflicting require_lane and require_control_plane_lane",
                },
            )
        require_lane = cp_lane

    with io.StringIO() as capture:
        with redirect_stdout(capture):
            result = checker.check(
                manifest,
                host_profile=host,
                host_check=True,
                strict_host_context=True,
                require_profile=normalized.get("require_profile"),
                require_lane=require_lane,
                require_workload_routes=(
                    normalized["require_workload_routes"] if normalized["require_workload_routes"] else None
                ),
                require_workload_classes=normalized.get("require_workload_classes"),
                require_migration_phase=normalized.get("require_migration_phase"),
                host_profile_is_authoritative=True,
            )
        output = capture.getvalue()

    payload: dict[str, Any] = {
        "name": name,
        "expected": normalized["expected_result"],
        "result": "pass" if result == 0 else "fail",
        "output": output.strip(),
    }

    if result == 0 and expected_pass:
        return True, f"{name}: ok", payload

    if result != 0 and not expected_pass:
        if expected_fragment is None or expected_fragment in output:
            return True, f"{name}: expected failure observed", payload
        return (
            False,
            f"{name}: expected failure containing '{expected_fragment}', got: {output!r}",
            {
                "name": name,
                "expected": normalized["expected_result"],
                "result": "fail",
                "expected_failure_contains": expected_fragment,
                "output": output.strip(),
            },
        )

    if expected_pass:
        return False, f"{name}: expected pass, got failure output: {output!r}", payload

    return False, f"{name}: expected fail, got pass", payload


def _load_cases_from_file(path: Path) -> list[dict[str, Any]]:
    payload = _load_case_payload(path)
    return _coerce_cases(payload)


def run_matrix(
    manifest_path: Path,
    case_matrix: list[dict[str, Any]],
    report_json: Path | None = None,
    *,
    validate_only: bool = False,
) -> int:
    manifest = checker.load_manifest(manifest_path)
    case_errors = _validate_cases(case_matrix, manifest)
    if case_errors:
        print("bare-metal matrix spec validation failed")
        for error in case_errors:
            print(f"  - {error}")
        return 1

    if validate_only:
        print(f"bare-metal matrix validation passed ({len(case_matrix)} cases)")
        if report_json is not None:
            summary = {
                "mode": "validate-only",
                "total": len(case_matrix),
                "passed": len(case_matrix),
                "failed": 0,
                "validated_only": True,
                "cases": [
                    {
                        "name": case.get("name", f"case[{index}]"),
                        "expected": case.get("expected_result", "pass"),
                        "result": "validated",
                    }
                    for index, case in enumerate(case_matrix)
                ],
            }
            with report_json.open("w", encoding="utf-8") as handle:
                json.dump(summary, handle, sort_keys=True, indent=2)
        return 0

    failures: list[str] = []
    results: list[dict[str, Any]] = []
    for case in case_matrix:
        name = case.get("name", "matrix-case")
        try:
            ok, message, payload = _run_case(case, manifest)
        except Exception as exc:  # pragma: no cover - defensive guard
            ok = False
            message = f"{name}: exception -> {exc}"
            payload = {
                "name": name,
                "expected": case.get("expected_result", "pass"),
                "result": "error",
                "error": str(exc),
            }
        results.append(payload)
        if not ok:
            failures.append(message)
        print(message)

    if report_json is not None:
        summary = {
            "total": len(case_matrix),
            "passed": len(case_matrix) - len(failures),
            "failed": len(failures),
            "cases": results,
        }
        with report_json.open("w", encoding="utf-8") as handle:
            json.dump(summary, handle, sort_keys=True, indent=2)

    if failures:
        print("bare-metal matrix check failed")
        for failure in failures:
            print(failure)
        return 1
    print(f"bare-metal matrix check passed ({len(case_matrix)} cases)")
    return 0


def main() -> int:
    parser = argparse.ArgumentParser(description=__doc__)
    parser.add_argument(
        "--manifest",
        type=Path,
        default=DEFAULT_MANIFEST,
        help="manifest path to evaluate",
    )
    parser.add_argument(
        "--matrix",
        type=Path,
        default=DEFAULT_MATRIX_PATH,
        help="path to matrix cases JSON (defaults to docs/engineering/bare-metal-e2e-matrix.json)",
    )
    parser.add_argument(
        "--report-json",
        type=Path,
        help="optional path for machine-readable matrix summary JSON",
    )
    parser.add_argument(
        "--print-summary",
        action="store_true",
        help="print a compact JSON summary in addition to textual logs",
    )
    parser.add_argument(
        "--validate-only",
        action="store_true",
        help=(
            "validate manifest and matrix case references only; do not run checker execution for each case"
        ),
    )
    args = parser.parse_args()

    try:
        if args.matrix.exists():
            cases = _load_cases_from_file(args.matrix)
        else:
            cases = DEFAULT_MATRIX
    except (OSError, ValueError, json.JSONDecodeError) as exc:
        print(f"ERROR: {exc}")
        return 1

    if not cases:
        print("ERROR: matrix has no cases")
        return 1

    code = run_matrix(args.manifest, cases, report_json=args.report_json, validate_only=args.validate_only)
    if args.print_summary:
        report = {
            "matrix": str(args.matrix),
            "manifest": str(args.manifest),
            "validate_only": args.validate_only,
            "total": len(cases),
            "code": code,
        }
        print(json.dumps(report, sort_keys=True))
    return code


if __name__ == "__main__":
    raise SystemExit(main())
