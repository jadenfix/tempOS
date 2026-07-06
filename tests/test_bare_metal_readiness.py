"""Tests for `scripts/check-bare-metal-readiness.py`."""

from __future__ import annotations

import importlib.util
import json
import io
import os
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
                    {"kind": "cuda", "required": False, "fallback_strategy": "cpu"},
                ],
            },
        ],
    }


def _clear_host_env() -> list[tuple[str, str | None]]:
    keys = [
        "BEATEROS_HOST_OS",
        "BEATEROS_HOST_ARCH",
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

    def test_report_mode_emits_machine_coverage_json(self) -> None:
        manifest = _valid_manifest()
        with tempfile.TemporaryDirectory() as td:
            path = _write_manifest(Path(td), manifest)
            loaded = MODULE.load_manifest(path)
            os.environ["BEATEROS_HOST_OS"] = "linux"
            os.environ["BEATEROS_HOST_ARCH"] = "x86_64"
            os.environ["BEATEROS_ACCELERATOR_CPU"] = "1"
            args = type(
                "Args",
                (),
                {
                    "check_host": True,
                    "require_profile": None,
                    "report": True,
                },
            )()
            buf = io.StringIO()
            with redirect_stdout(buf):
                MODULE.run_and_dump_json(args, loaded)
            lines = [line for line in buf.getvalue().splitlines() if line.strip()]
            payload = json.loads(lines[0])
            self.assertEqual(payload["host"]["os"], "linux")
            self.assertIn("test-host", payload["supported_profiles"])
