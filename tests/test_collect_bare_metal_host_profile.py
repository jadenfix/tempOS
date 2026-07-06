"""Tests for `scripts/collect-bare-metal-host-profile.py`."""

from __future__ import annotations

import importlib.util
import json
import os
import sys
import tempfile
import unittest
from pathlib import Path

SPEC = importlib.util.spec_from_file_location(
    "collect_host_profile",
    str(Path(__file__).resolve().parent.parent / "scripts" / "collect-bare-metal-host-profile.py"),
)
assert SPEC and SPEC.loader
MODULE = importlib.util.module_from_spec(SPEC)
sys.modules["collect_host_profile"] = MODULE
SPEC.loader.exec_module(MODULE)


def _write_profile(path: Path) -> Path:
    path.write_text(
        json.dumps(
            {
                "schema_version": 1,
                "host": {
                    "os": "linux",
                    "arch": "x86_64",
                    "cpu_cores": 16,
                    "memory_gib": 64,
                    "accelerators": ["cpu", "cuda"],
                },
            },
        ),
        encoding="utf-8",
    )
    return path


class TestCollectHostProfile(unittest.TestCase):
    def setUp(self) -> None:
        self._previous = {}
        for key in [
            "BEATEROS_HOST_OS",
            "BEATEROS_HOST_ARCH",
            "BEATEROS_HOST_MEMORY_GIB",
            "BEATEROS_ACCELERATOR_CPU",
            "BEATEROS_ACCELERATOR_CUDA",
            "BEATEROS_ACCELERATOR_APPLE_GPU",
            "BEATEROS_ACCELERATOR_TPU",
            "BEATEROS_ACCELERATOR_ENCLAVE",
        ]:
            self._previous[key] = os.environ.get(key)
            os.environ.pop(key, None)

    def tearDown(self) -> None:
        for key, value in self._previous.items():
            if value is None:
                os.environ.pop(key, None)
            else:
                os.environ[key] = value

    def test_load_profile_accepts_nested_host_object(self) -> None:
        with tempfile.TemporaryDirectory() as td:
            path = _write_profile(Path(td) / "profile.json")
            loaded = MODULE.load_profile(path)
            self.assertEqual(loaded["os"], "linux")
            self.assertEqual(loaded["arch"], "x86_64")
            self.assertEqual(loaded["cpu_cores"], 16)

    def test_load_profile_accepts_flat_host_object(self) -> None:
        payload = {"os": "darwin", "arch": "arm64", "cpu_cores": 8}
        with tempfile.TemporaryDirectory() as td:
            path = Path(td) / "profile.json"
            path.write_text(json.dumps(payload), encoding="utf-8")
            loaded = MODULE.load_profile(path)
            self.assertEqual(loaded["os"], "darwin")
            self.assertEqual(loaded["arch"], "arm64")
            self.assertEqual(loaded["cpu_cores"], 8)

    def test_collect_profile_tracks_cpu_override(self) -> None:
        with tempfile.TemporaryDirectory() as td:
            os.environ["BEATEROS_HOST_OS"] = "linux"
            os.environ["BEATEROS_HOST_ARCH"] = "x86_64"
            os.environ["BEATEROS_HOST_MEMORY_GIB"] = "24.5"
            os.environ["BEATEROS_ACCELERATOR_CPU"] = "true"
            collected = MODULE.collect_profile()
            host = MODULE.to_payload(collected)
            self.assertEqual(host["host"]["os"], "linux")
            self.assertEqual(host["host"]["arch"], "x86_64")
            self.assertAlmostEqual(host["host"]["memory_gib"], 24.5)
            self.assertIn("cpu", host["host"]["accelerators"])
