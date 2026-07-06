"""Tests for `scripts/check-bare-metal-readiness.py`."""

from __future__ import annotations

import importlib.util
import json
import io
import os
import sys
import tempfile
import unittest
from contextlib import redirect_stdout
from pathlib import Path

SPEC = importlib.util.spec_from_file_location(
    "bare_metal_readiness",
    str(Path(__file__).resolve().parent.parent / "scripts" / "check-bare-metal-readiness.py"),
)
assert SPEC and SPEC.loader
MODULE = importlib.util.module_from_spec(SPEC)
sys.modules["bare_metal_readiness"] = MODULE
SPEC.loader.exec_module(MODULE)


def _write_manifest(tmpdir: Path, payload: dict[str, object]) -> Path:
    target = tmpdir / "manifest.json"
    target.write_text(json.dumps(payload), encoding="utf-8")
    return target


def _valid_manifest() -> dict[str, object]:
    return {
        "schema_version": 1,
        "profiles": [
            {
                "name": "test-host",
                "scope": "compatibility",
                "stability_tier": "stable",
                "target_os": ["linux"],
                "target_arch": ["x86_64"],
                "resource_contract": {"min_cpu_cores": 2},
                "accelerators": [
                    {"kind": "cpu", "required": True, "fallback_strategy": "cpu"},
                ],
            },
            {
                "name": "test-cuda-host",
                "scope": "compatibility",
                "stability_tier": "stable",
                "target_os": ["linux"],
                "target_arch": ["x86_64"],
                "resource_contract": {
                    "min_cpu_cores": 2,
                    "min_gpu_mem_gib": 1,
                    "min_pcie_bwl_gbps": 1,
                },
                "accelerators": [
                    {"kind": "cuda", "required": False, "fallback_strategy": "cpu"},
                    {"kind": "cpu", "required": True, "fallback_strategy": "cpu"},
                ],
            },
        ],
        "architecture": {
            "control_plane_lane": "portable-control-plane",
            "migration_order": ["portable-control-plane", "linux-cuda-lane"],
            "lanes": [
                {
                    "name": "portable-control-plane",
                    "profile": "test-host",
                    "mandatory": True,
                    "required_stability_tier": "stable",
                    "workload_classes": ["control-plane", "policy-admission"],
                    "depends_on": [],
                    "fallback_chain": ["test-host"],
                },
                {
                    "name": "linux-cuda-lane",
                    "profile": "test-cuda-host",
                    "mandatory": False,
                    "depends_on": ["portable-control-plane"],
                    "required_stability_tier": "stable",
                    "workload_classes": ["benchmarks", "tooling"],
                    "fallback_chain": ["test-cuda-host", "test-host"],
                },
            ],
        },
    }


def _clear_host_env() -> list[tuple[str, str | None]]:
    keys = [
        "BEATEROS_HOST_OS",
        "BEATEROS_HOST_ARCH",
        "BEATEROS_HOST_MEMORY_GIB",
        "BEATEROS_HOST_STORAGE_IOPS",
        "BEATEROS_HOST_MEMORY_BANDWIDTH_GBPS",
        "BEATEROS_HOST_GPU_MEM_GIB",
        "BEATEROS_HOST_PCIE_BWL_GBPS",
        "BEATEROS_HOST_RESIDUAL_LATENCY_MS",
        "BEATEROS_HOST_GPU_TEMP_C",
        "BEATEROS_ACCELERATOR_CPU",
        "BEATEROS_ACCELERATOR_CUDA",
        "BEATEROS_ACCELERATOR_APPLE_GPU",
        "BEATEROS_ACCELERATOR_TPU",
        "BEATEROS_ACCELERATOR_ENCLAVE",
    ]
    before = []
    for key in keys:
        before.append((key, os.environ.get(key)))
        os.environ.pop(key, None)
    return before


class TestBareMetalReadiness(unittest.TestCase):
    def setUp(self) -> None:
        self._previous_env = _clear_host_env()

    def tearDown(self) -> None:
        for key, value in self._previous_env:
            if value is None:
                os.environ.pop(key, None)
            else:
                os.environ[key] = value

    def test_manifest_validation_passes_on_valid_input(self) -> None:
        errors = MODULE.validate_manifest(_valid_manifest())
        self.assertFalse(errors)

    def test_manifest_validation_fails_for_unknown_accelerator(self) -> None:
        manifest = _valid_manifest()
        manifest["profiles"][0]["accelerators"][0]["kind"] = "warp-core-quantum"
        errors = MODULE.validate_manifest(manifest)
        self.assertEqual(
            errors,
            ["profile[0].accelerators[0].kind unknown: warp-core-quantum"],
        )

    def test_architecture_validation_rejects_cycle(self) -> None:
        manifest = _valid_manifest()
        manifest["architecture"]["lanes"][0]["depends_on"] = ["linux-cuda-lane"]
        errors = MODULE.validate_manifest(manifest)
        self.assertIn("architecture.lanes contains dependency cycle", errors)

    def test_architecture_validation_rejects_unknown_migration_target(self) -> None:
        manifest = _valid_manifest()
        manifest["architecture"]["migration_order"].append("ghost-lane")
        errors = MODULE.validate_manifest(manifest)
        self.assertIn("architecture.migration_order[2] references unknown lane: ghost-lane", errors)

    def test_architecture_validation_rejects_unknown_workload_class(self) -> None:
        manifest = _valid_manifest()
        manifest["architecture"]["lanes"][0]["workload_classes"] = ["control-plane", "quantum-workload"]
        errors = MODULE.validate_manifest(manifest)
        self.assertIn("architecture.lanes[0].workload_classes has unknown classes: quantum-workload", errors)

    def test_architecture_validation_rejects_missing_required_stability_tier(self) -> None:
        manifest = _valid_manifest()
        del manifest["architecture"]["lanes"][0]["required_stability_tier"]
        errors = MODULE.validate_manifest(manifest)
        self.assertIn("architecture.lanes[0].required_stability_tier is required", errors)

    def test_architecture_validation_rejects_bad_migration_order(self) -> None:
        manifest = _valid_manifest()
        manifest["architecture"]["migration_order"].append("portable-control-plane")
        errors = MODULE.validate_manifest(manifest)
        self.assertIn("architecture.migration_order[2] duplicates lane: portable-control-plane", errors)

    def test_architecture_validation_rejects_migration_order_not_starting_with_control_plane(self) -> None:
        manifest = _valid_manifest()
        manifest["architecture"]["migration_order"] = ["linux-cuda-lane", "portable-control-plane"]
        errors = MODULE.validate_manifest(manifest)
        self.assertIn(
            "architecture.migration_order must start with architecture.control_plane_lane",
            errors,
        )

    def test_missing_schema_version_rejected(self) -> None:
        manifest = _valid_manifest()
        manifest["schema_version"] = 0
        errors = MODULE.validate_manifest(manifest)
        self.assertEqual(errors, ["schema_version must be 1"])

    def test_host_check_matches_expected_profile(self) -> None:
        manifest = _valid_manifest()
        with tempfile.TemporaryDirectory() as td:
            path = _write_manifest(Path(td), manifest)
            os.environ["BEATEROS_HOST_OS"] = "linux"
            os.environ["BEATEROS_HOST_ARCH"] = "x86_64"
            os.environ["BEATEROS_ACCELERATOR_CPU"] = "1"
            loaded = MODULE.load_manifest(path)
            self.assertEqual(MODULE.check(loaded, host_check=True), 0)

    def test_host_check_fails_without_matching_profile(self) -> None:
        manifest = _valid_manifest()
        with tempfile.TemporaryDirectory() as td:
            path = _write_manifest(Path(td), manifest)
            loaded = MODULE.load_manifest(path)
            os.environ["BEATEROS_HOST_OS"] = "darwin"
            os.environ["BEATEROS_HOST_ARCH"] = "arm64"
            os.environ["BEATEROS_ACCELERATOR_CPU"] = "0"
            self.assertEqual(MODULE.check(loaded, host_check=True), 1)

    def test_host_check_fails_when_resource_contract_exceeded(self) -> None:
        manifest = _valid_manifest()
        manifest["profiles"][0]["resource_contract"]["min_memory_gib"] = 16
        with tempfile.TemporaryDirectory() as td:
            path = _write_manifest(Path(td), manifest)
            loaded = MODULE.load_manifest(path)
            os.environ["BEATEROS_HOST_OS"] = "linux"
            os.environ["BEATEROS_HOST_ARCH"] = "x86_64"
            os.environ["BEATEROS_HOST_MEMORY_GIB"] = "8"
            os.environ["BEATEROS_ACCELERATOR_CPU"] = "1"
            self.assertEqual(MODULE.check(loaded, host_check=True), 1)

    def test_profile_supports_host_enforces_all_reported_resource_constraints(self) -> None:
        profile = {
            "name": "resource-heavy",
            "target_os": ["linux"],
            "target_arch": ["x86_64"],
            "resource_contract": {
                "min_cpu_cores": 8,
                "min_memory_gib": 32,
                "min_storage_iops": 10000,
                "min_memory_bandwidth_gbps": 40,
                "min_gpu_mem_gib": 12,
                "min_pcie_bwl_gbps": 16,
                "max_residual_latency_ms": 50,
                "max_gpu_tolerance_temp_c": 80,
            },
            "accelerators": [
                {"kind": "cpu", "required": True, "fallback_strategy": "cpu"},
                {"kind": "cuda", "required": True, "fallback_strategy": "cpu"},
            ],
        }
        supported_host = MODULE.HostContext(
            os_name="linux",
            arch="x86_64",
            cpu_cores=16,
            memory_gib=64,
            storage_iops=20000,
            memory_bandwidth_gbps=80,
            gpu_mem_gib=24,
            pcie_bwl_gbps=32,
            residual_latency_ms=20,
            gpu_temp_c=70,
            accelerators=frozenset({"cpu", "cuda"}),
        )
        self.assertTrue(
            MODULE.profile_supports_host(
                profile,
                supported_host,
                strict_host_context=True,
            ),
        )

        constrained_fields = {
            "cpu_cores": 4,
            "memory_gib": 16,
            "storage_iops": 5000,
            "memory_bandwidth_gbps": 20,
            "gpu_mem_gib": 6,
            "pcie_bwl_gbps": 8,
            "residual_latency_ms": 100,
            "gpu_temp_c": 90,
        }
        for field_name, value in constrained_fields.items():
            host_values = supported_host.__dict__.copy()
            host_values[field_name] = value
            self.assertFalse(
                MODULE.profile_supports_host(
                    profile,
                    MODULE.HostContext(**host_values),
                    strict_host_context=True,
                ),
                field_name,
            )

    def test_optional_cuda_does_not_make_non_cuda_host_support_constrained_profile(self) -> None:
        profile = {
            "name": "cuda-scored",
            "target_os": ["linux"],
            "target_arch": ["x86_64"],
            "resource_contract": {
                "min_cpu_cores": 8,
                "min_memory_gib": 16,
                "min_gpu_mem_gib": 6,
                "min_pcie_bwl_gbps": 16,
            },
            "accelerators": [
                {"kind": "cuda", "required": False, "fallback_strategy": "cpu"},
                {"kind": "cpu", "required": True, "fallback_strategy": "cpu"},
            ],
        }
        non_cuda_host = MODULE.HostContext(
            os_name="linux",
            arch="x86_64",
            cpu_cores=16,
            memory_gib=64,
            gpu_mem_gib=12,
            pcie_bwl_gbps=32,
            accelerators=frozenset({"cpu"}),
        )
        self.assertFalse(
            MODULE.profile_supports_host(
                profile,
                non_cuda_host,
                strict_host_context=False,
            ),
        )

        cuda_host = MODULE.HostContext(
            os_name="linux",
            arch="x86_64",
            cpu_cores=16,
            memory_gib=64,
            gpu_mem_gib=12,
            pcie_bwl_gbps=32,
            accelerators=frozenset({"cpu", "cuda"}),
        )
        self.assertTrue(
            MODULE.profile_supports_host(
                profile,
                cuda_host,
                strict_host_context=False,
            ),
        )

    def test_authoritative_host_profile_does_not_fall_back_to_environment(self) -> None:
        manifest = _valid_manifest()
        manifest["profiles"][0]["resource_contract"]["min_storage_iops"] = 10000
        with tempfile.TemporaryDirectory() as td:
            path = _write_manifest(Path(td), manifest)
            profile_path = Path(td) / "host.json"
            profile_path.write_text(
                json.dumps(
                    {
                        "os": "linux",
                        "arch": "x86_64",
                        "cpu_cores": 8,
                        "memory_gib": 32,
                        "accelerators": ["cpu"],
                    },
                ),
                encoding="utf-8",
            )
            os.environ["BEATEROS_HOST_STORAGE_IOPS"] = "20000"
            args = type(
                "Args",
                (),
                {
                    "check_host": True,
                    "host_profile": profile_path,
                    "require_profile": None,
                    "require_lane": None,
                    "require_control_plane_lane": False,
                    "require_workload_class": [],
                    "require_workload_route": [],
                    "require_migration_phase": None,
                    "report": False,
                    "report_only": False,
                    "strict_host_context": False,
                },
            )()
            loaded = MODULE.load_manifest(path)
            self.assertEqual(MODULE.run_and_dump_json(args, loaded), 1)

    def test_host_profile_overrides_schema_works(self) -> None:
        manifest = _valid_manifest()
        with tempfile.TemporaryDirectory() as td:
            path = _write_manifest(Path(td), manifest)
            loaded = MODULE.load_manifest(path)
            profile = {
                "os": "linux",
                "arch": "x86_64",
                "cpu_cores": 1,
                "memory_gib": 128,
                "accelerators": ["cpu"],
            }
            self.assertEqual(
                MODULE.check(loaded, host_check=True, host_profile=profile),
                1,
            )
            profile["cpu_cores"] = 4
            profile["memory_gib"] = 4
            self.assertEqual(
                MODULE.check(loaded, host_check=True, host_profile=profile),
                0,
            )

    def test_check_requires_matching_lane(self) -> None:
        manifest = _valid_manifest()
        with tempfile.TemporaryDirectory() as td:
            path = _write_manifest(Path(td), manifest)
            loaded = MODULE.load_manifest(path)
            os.environ["BEATEROS_HOST_OS"] = "linux"
            os.environ["BEATEROS_HOST_ARCH"] = "x86_64"
            os.environ["BEATEROS_ACCELERATOR_CPU"] = "1"
            self.assertEqual(
                MODULE.check(
                    loaded,
                    host_check=True,
                    require_lane="portable-control-plane",
                ),
                0,
            )
            self.assertEqual(
                MODULE.check(
                    loaded,
                    host_check=True,
                    require_lane="apple-metal-lane",
                ),
                1,
            )

    def test_check_requires_lane_to_be_runnable(self) -> None:
        manifest = _valid_manifest()
        manifest["profiles"].append(
            {
                "name": "darwin-only",
                "scope": "compatibility",
                "stability_tier": "stable",
                "target_os": ["darwin"],
                "target_arch": ["arm64"],
                "resource_contract": {"min_cpu_cores": 2},
                "accelerators": [
                    {"kind": "cpu", "required": True, "fallback_strategy": "cpu"},
                ],
            },
        )
        manifest["architecture"]["lanes"].append(
            {
                "name": "darwin-only-lane",
                "profile": "darwin-only",
                "mandatory": False,
                "depends_on": ["portable-control-plane"],
                "required_stability_tier": "stable",
                "workload_classes": ["policy-admission"],
                "fallback_chain": ["darwin-only"],
            },
        )
        manifest["architecture"]["migration_order"].append("darwin-only-lane")
        with tempfile.TemporaryDirectory() as td:
            path = _write_manifest(Path(td), manifest)
            loaded = MODULE.load_manifest(path)
            os.environ["BEATEROS_HOST_OS"] = "linux"
            os.environ["BEATEROS_HOST_ARCH"] = "x86_64"
            os.environ["BEATEROS_ACCELERATOR_CPU"] = "1"
            self.assertEqual(
                MODULE.check(
                    loaded,
                    host_check=True,
                    require_lane="darwin-only-lane",
                ),
                1,
            )
            self.assertEqual(
                MODULE.check(
                    loaded,
                    host_check=True,
                    require_lane="portable-control-plane",
                ),
                0,
            )

    def test_check_requires_workload_routing(self) -> None:
        manifest = _valid_manifest()
        with tempfile.TemporaryDirectory() as td:
            path = _write_manifest(Path(td), manifest)
            loaded = MODULE.load_manifest(path)
            os.environ["BEATEROS_HOST_OS"] = "linux"
            os.environ["BEATEROS_HOST_ARCH"] = "x86_64"
            os.environ["BEATEROS_ACCELERATOR_CPU"] = "1"
            self.assertEqual(
                MODULE.check(
                    loaded,
                    host_check=True,
                    require_profile="test-host",
                    require_lane="portable-control-plane",
                    require_workload_classes=["tooling"],
                ),
                1,
            )
            self.assertEqual(
                MODULE.check(
                    loaded,
                    host_check=True,
                    require_profile="test-host",
                    require_lane="portable-control-plane",
                    require_workload_classes=["policy-admission"],
                ),
                0,
            )
            self.assertEqual(
                MODULE.check(
                    loaded,
                    host_check=True,
                    require_profile="test-host",
                    require_lane="portable-control-plane",
                    require_workload_routes={"policy-admission": "portable-control-plane"},
                ),
                0,
            )
            self.assertEqual(
                MODULE.check(
                    loaded,
                    host_check=True,
                    require_profile="test-host",
                    require_lane="portable-control-plane",
                    require_workload_routes={"policy-admission": "linux-cuda-lane"},
                ),
                1,
            )

    def test_preferred_workload_route_chooses_lowest_score(self) -> None:
        manifest = _valid_manifest()
        manifest["profiles"].append(
            {
                "name": "test-host-fast",
                "scope": "compatibility",
                "stability_tier": "stable",
                "target_os": ["linux"],
                "target_arch": ["x86_64"],
                "resource_contract": {"min_cpu_cores": 2, "min_memory_gib": 2},
                "optimization_targets": {"max_queue_depth": 1},
                "accelerators": [
                    {"kind": "cpu", "required": True, "fallback_strategy": "cpu"},
                ],
            },
        )
        manifest["architecture"]["lanes"].append(
            {
                "name": "policy-optimized",
                "profile": "test-host-fast",
                "mandatory": False,
                "depends_on": ["portable-control-plane"],
                "required_stability_tier": "stable",
                "workload_classes": ["policy-admission", "tooling"],
                "fallback_chain": ["test-host-fast"],
            },
        )
        manifest["architecture"]["migration_order"].append("policy-optimized")
        manifest["profiles"][0]["optimization_targets"] = {"max_queue_depth": 32}
        with tempfile.TemporaryDirectory() as td:
            path = _write_manifest(Path(td), manifest)
            loaded = MODULE.load_manifest(path)
            os.environ["BEATEROS_HOST_OS"] = "linux"
            os.environ["BEATEROS_HOST_ARCH"] = "x86_64"
            os.environ["BEATEROS_ACCELERATOR_CPU"] = "1"
            os.environ["BEATEROS_HOST_MEMORY_GIB"] = "16"
            self.assertEqual(
                MODULE.check(
                    loaded,
                    host_check=True,
                    require_profile="test-host",
                    require_lane="portable-control-plane",
                    require_workload_routes={"policy-admission": "policy-optimized"},
                ),
                0,
            )
            self.assertEqual(
                MODULE.check(
                    loaded,
                    host_check=True,
                    require_profile="test-host",
                    require_lane="portable-control-plane",
                    require_workload_routes={"policy-admission": "portable-control-plane"},
                ),
                1,
            )

    def test_preferred_workload_route_tie_breaks_by_migration_order(self) -> None:
        manifest = _valid_manifest()
        manifest["profiles"].append(
            {
                "name": "policy-tie-profile",
                "scope": "compatibility",
                "stability_tier": "stable",
                "target_os": ["linux"],
                "target_arch": ["x86_64"],
                "resource_contract": {"min_cpu_cores": 2, "min_memory_gib": 4},
                "accelerators": [
                    {"kind": "cpu", "required": True, "fallback_strategy": "cpu"},
                ],
            },
        )
        manifest["architecture"]["lanes"].append(
            {
                "name": "policy-tie-lane",
                "profile": "policy-tie-profile",
                "mandatory": False,
                "depends_on": ["portable-control-plane"],
                "required_stability_tier": "stable",
                "workload_classes": ["policy-admission", "tooling"],
                "fallback_chain": ["policy-tie-profile"],
            },
        )
        manifest["architecture"]["migration_order"].append("policy-tie-lane")
        os.environ["BEATEROS_HOST_OS"] = "linux"
        os.environ["BEATEROS_HOST_ARCH"] = "x86_64"
        os.environ["BEATEROS_ACCELERATOR_CPU"] = "1"
        os.environ["BEATEROS_HOST_MEMORY_GIB"] = "64"
        with tempfile.TemporaryDirectory() as td:
            path = _write_manifest(Path(td), manifest)
            loaded = MODULE.load_manifest(path)
            self.assertEqual(
                MODULE.check(
                    loaded,
                    host_check=True,
                    require_profile="test-host",
                    require_lane="portable-control-plane",
                    require_workload_routes={"policy-admission": "portable-control-plane"},
                ),
                0,
            )
            self.assertEqual(
                MODULE.check(
                    loaded,
                    host_check=True,
                    require_profile="test-host",
                    require_lane="portable-control-plane",
                    require_workload_routes={"policy-admission": "policy-tie-lane"},
                ),
                1,
            )

    def test_report_mode_emits_machine_coverage_json(self) -> None:
        manifest = _valid_manifest()
        with tempfile.TemporaryDirectory() as td:
            path = _write_manifest(Path(td), manifest)
            loaded = MODULE.load_manifest(path)
            os.environ["BEATEROS_HOST_OS"] = "linux"
            os.environ["BEATEROS_HOST_ARCH"] = "x86_64"
            os.environ["BEATEROS_ACCELERATOR_CPU"] = "1"
            os.environ["BEATEROS_HOST_MEMORY_GIB"] = "32"
            args = type(
                "Args",
                (),
                {
                    "check_host": True,
                    "require_profile": None,
                    "require_lane": None,
                    "require_control_plane_lane": False,
                    "require_workload_class": [],
                    "require_workload_route": [],
                    "host_profile": None,
                    "report": True,
                    "report_only": False,
                    "strict_host_context": False,
                    "require_migration_phase": None,
                },
            )()
            buf = io.StringIO()
            with redirect_stdout(buf):
                MODULE.run_and_dump_json(args, loaded)
            lines = [line for line in buf.getvalue().splitlines() if line.strip()]
            payload = json.loads(lines[0])
            self.assertEqual(payload["host"]["os"], "linux")
            self.assertEqual(payload["host"]["memory_gib"], 32.0)
            self.assertIn("test-host", payload["supported_profiles"])
            self.assertIn("portable-control-plane", payload["architecture"]["supported_lanes"])
            self.assertIn("portable-control-plane", payload["architecture"]["migration_plan"][0]["lane"])
            self.assertIn("policy-admission", payload["architecture"]["workload_routing"])
            self.assertEqual(
                payload["architecture"]["preferred_workload_routes"]["policy-admission"]["lane"],
                "portable-control-plane",
            )
            self.assertIn(
                "lane_scores",
                payload["architecture"],
            )
            self.assertEqual(payload["architecture"]["mandatory_unrunnable_lanes"], [])
