"""Tests for `scripts/run-bare-metal-e2e-matrix.py`."""

from __future__ import annotations

import importlib.util
import json
import tempfile
from pathlib import Path

SPEC = importlib.util.spec_from_file_location(
    "run_bare_metal_e2e_matrix",
    str(Path(__file__).resolve().parent.parent / "scripts" / "run-bare-metal-e2e-matrix.py"),
)
assert SPEC and SPEC.loader
MODULE = importlib.util.module_from_spec(SPEC)
SPEC.loader.exec_module(MODULE)


def _manifest() -> dict[str, object]:
    return {
        "schema_version": 1,
        "profiles": [
            {
                "name": "portable-host-control-plane",
                "scope": "compatibility",
                "stability_tier": "stable",
                "target_os": ["linux", "darwin", "windows"],
                "target_arch": ["x86_64", "arm64"],
                "resource_contract": {
                    "min_cpu_cores": 2,
                    "min_memory_gib": 4,
                },
                "accelerators": [
                    {
                        "kind": "cpu",
                        "required": True,
                        "fallback_strategy": "default-host-execution",
                    },
                    {
                        "kind": "cuda",
                        "required": False,
                        "fallback_strategy": "cpu",
                    },
                ],
            },
            {
                "name": "linux-cuda-scored-host",
                "scope": "experimental",
                "stability_tier": "beta",
                "target_os": ["linux"],
                "target_arch": ["x86_64"],
                "resource_contract": {
                    "min_cpu_cores": 8,
                    "min_memory_gib": 16,
                    "min_gpu_mem_gib": 6,
                },
                "accelerators": [
                    {
                        "kind": "cpu",
                        "required": True,
                        "fallback_strategy": "cpu-only-core-path",
                    },
                    {
                        "kind": "cuda",
                        "required": False,
                        "fallback_strategy": "cpu-or-microvm",
                    },
                ],
            },
        ],
        "architecture": {
            "control_plane_lane": "portable-control-plane",
            "migration_order": ["portable-control-plane", "linux-cuda-lane"],
            "lanes": [
                {
                    "name": "portable-control-plane",
                    "profile": "portable-host-control-plane",
                    "mandatory": True,
                    "required_stability_tier": "stable",
                    "workload_classes": ["policy-admission", "media", "tooling"],
                    "depends_on": [],
                    "fallback_chain": ["portable-host-control-plane"],
                },
                {
                    "name": "linux-cuda-lane",
                    "profile": "linux-cuda-scored-host",
                    "mandatory": False,
                    "required_stability_tier": "beta",
                    "depends_on": ["portable-control-plane"],
                    "workload_classes": ["tooling"],
                    "fallback_chain": ["portable-host-control-plane"],
                },
            ],
        },
    }


def _write_manifest(tmpdir: Path) -> Path:
    target = tmpdir / "manifest.json"
    target.write_text(json.dumps(_manifest()), encoding="utf-8")
    return target


def test_load_matrix_file_and_coerce_cases() -> None:
    payload = {
        "cases": [
            {
                "name": "ok",
                "host": {"os": "linux", "arch": "x86_64"},
                "require_profile": "portable-host-control-plane",
            }
        ]
    }
    with tempfile.TemporaryDirectory() as td:
        path = Path(td) / "matrix.json"
        path.write_text(json.dumps(payload), encoding="utf-8")
        cases = MODULE._load_cases_from_file(path)
        assert len(cases) == 1
        assert cases[0]["name"] == "ok"


def test_matrix_run_pass_fail() -> None:
    with tempfile.TemporaryDirectory() as td:
        manifest = _write_manifest(Path(td))

        cases = [
            {
                "name": "portable-control-plane",
                "host": {"os": "linux", "arch": "x86_64", "cpu_cores": 8, "memory_gib": 8, "accelerators": ["cpu"]},
                "require_profile": "portable-host-control-plane",
                "require_lane": "portable-control-plane",
                "require_workload_routes": {"policy-admission": "portable-control-plane"},
            },
            {
                "name": "weak-cuda-fail",
                "host": {"os": "linux", "arch": "x86_64", "cpu_cores": 4, "memory_gib": 2, "accelerators": ["cpu", "cuda"], "gpu_mem_gib": 24},
                "require_control_plane_lane": True,
                "require_profile": "linux-cuda-scored-host",
                "expected_result": "fail",
                "expected_failure_contains": "required profile not supported",
            },
        ]

        report = Path(td) / "report.json"
        code = MODULE.run_matrix(manifest, cases, report_json=report)
        assert code == 0
        payload = json.loads(report.read_text(encoding="utf-8"))
        assert payload["total"] == 2
        assert payload["failed"] == 0


def test_matrix_case_with_invalid_spec_fails_fast() -> None:
    with tempfile.TemporaryDirectory() as td:
        manifest = _write_manifest(Path(td))
        bad_case = {"name": "invalid-host", "host": "linux", "expected_result": "pass"}
        code = MODULE.run_matrix(manifest, [bad_case], report_json=None)
        assert code == 1


def test_matrix_case_with_unknown_manifest_reference_fails_fast() -> None:
    with tempfile.TemporaryDirectory() as td:
        manifest = _write_manifest(Path(td))
        bad_cases = [
            {
                "name": "bad-profile",
                "host": {"os": "linux", "arch": "x86_64"},
                "require_profile": "does-not-exist",
                "expected_result": "pass",
            },
            {
                "name": "bad-lane",
                "host": {"os": "linux", "arch": "x86_64"},
                "require_lane": "not-a-lane",
                "require_profile": "portable-host-control-plane",
                "expected_result": "pass",
            },
            {
                "name": "bad-workload-route",
                "host": {"os": "linux", "arch": "x86_64"},
                "require_profile": "portable-host-control-plane",
                "require_workload_routes": {"policy-admission": "missing-lane"},
                "expected_result": "pass",
            },
            {
                "name": "bad-route-key",
                "host": {"os": "linux", "arch": "x86_64"},
                "require_profile": "portable-host-control-plane",
                "require_workload_classes": ["policy-admission", "not-a-workload"],
                "expected_result": "pass",
            },
        ]
        code = MODULE.run_matrix(manifest, bad_cases, report_json=None)
        assert code == 1


def test_matrix_validate_only_bypasses_case_execution() -> None:
    with tempfile.TemporaryDirectory() as td:
        manifest = _write_manifest(Path(td))
        cases = [
            {
                "name": "barely-fast-fail-on-exec",
                "host": {
                    "os": "linux",
                    "arch": "x86_64",
                    "cpu_cores": 4,
                    "memory_gib": 6,
                    "accelerators": ["cpu", "cuda"],
                    "gpu_mem_gib": 8,
                },
                "require_lane": "linux-cuda-lane",
                "expected_result": "pass",
            },
        ]
        code = MODULE.run_matrix(manifest, cases, report_json=None, validate_only=True)
        assert code == 0


def test_matrix_validate_only_with_invalid_case_fails_fast() -> None:
    with tempfile.TemporaryDirectory() as td:
        manifest = _write_manifest(Path(td))
        code = MODULE.run_matrix(
            manifest,
            [{"name": "bad", "host": "linux", "require_profile": "portable-host-control-plane"}],
            report_json=None,
            validate_only=True,
        )
        assert code == 1


def test_disabled_matrix_case_can_validate_without_host() -> None:
    with tempfile.TemporaryDirectory() as td:
        manifest = _write_manifest(Path(td))
        code = MODULE.run_matrix(
            manifest,
            [
                {
                    "name": "disabled-no-host",
                    "enabled": False,
                    "require_lane": "portable-control-plane",
                    "require_profile": "portable-host-control-plane",
                },
            ],
            report_json=None,
            validate_only=True,
        )
        assert code == 0
