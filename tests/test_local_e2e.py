"""Tests for scripts/local-e2e.py."""

from __future__ import annotations

import importlib.util
import subprocess
import sys
import unittest
from pathlib import Path

REPO_ROOT = Path(__file__).resolve().parent.parent
_SCRIPT = REPO_ROOT / "scripts" / "local-e2e.py"

_spec = importlib.util.spec_from_file_location("local_e2e", _SCRIPT)
assert _spec and _spec.loader
local_e2e = importlib.util.module_from_spec(_spec)
sys.modules["local_e2e"] = local_e2e
_spec.loader.exec_module(local_e2e)


class PlanTest(unittest.TestCase):
    def test_plan_runs_existing_gates_in_stable_order(self) -> None:
        plan = local_e2e.build_plan("python3", branch_base_available=True)
        self.assertEqual(
            [(gate.name, gate.command) for gate in plan],
            [
                ("worktree-whitespace", ("git", "diff", "HEAD", "--check")),
                ("branch-whitespace", ("git", "diff", "--check", "origin/main...HEAD")),
                ("final-integrity", ("python3", "scripts/check-final-integrity.py")),
                (
                    "governance-ledger",
                    (
                        "python3",
                        "scripts/check-governance.py",
                        "docs/governance/coordination-ledger.md",
                    ),
                ),
                (
                    "beater-osd-runtime-smoke",
                    ("python3", "scripts/run-beater-osd-runtime-smoke.py", "--json"),
                ),
                (
                    "beater-os-runtime-smoke",
                    ("python3", "scripts/run-beater-os-runtime-smoke.py", "--json"),
                ),
                (
                    "beater-os-runtime-worker-smoke",
                    ("python3", "scripts/run-beater-os-runtime-worker-smoke.py", "--json"),
                ),
                (
                    "beater-osd-http-execute-smoke",
                    ("python3", "scripts/run-beater-osd-http-execute-smoke.py", "--json"),
                ),
                (
                    "beater-osd-http-claims-smoke",
                    ("python3", "scripts/run-beater-osd-http-claims-smoke.py", "--json"),
                ),
                (
                    "bare-metal-readiness",
                    (
                        "python3",
                        "scripts/check-bare-metal-readiness.py",
                        "--check-host",
                        "--require-control-plane-lane",
                        "--require-workload-class",
                        "policy-admission",
                        "--require-workload-route",
                        "policy-admission=portable-control-plane",
                        "--require-migration-phase",
                        "runtime",
                    ),
                ),
                ("bare-metal-e2e-matrix", ("python3", "scripts/run-bare-metal-e2e-matrix.py")),
                ("optimization-docs", ("python3", "scripts/check-optimization-docs.py")),
                ("python-unit-tests", ("python3", "-m", "unittest", "discover", "-s", "tests")),
                ("spec-conformance", ("python3", "spec/conformance/validate.py", "--quiet")),
                ("conformance-selftest", ("python3", "tools/conformance/selftest.py")),
                (
                    "fixture-freshness",
                    ("python3", "tools/conformance/build_fixtures.py", "--check"),
                ),
                ("trace-and-scenario-conformance", ("python3", "tools/conformance/validate.py")),
                ("rustfmt", ("cargo", "fmt", "--all", "--", "--check")),
                ("rust-tests", ("cargo", "test", "--workspace", "--locked")),
                (
                    "rust-clippy",
                    (
                        "cargo",
                        "clippy",
                        "--workspace",
                        "--all-targets",
                        "--locked",
                        "--",
                        "-D",
                        "warnings",
                    ),
                ),
            ],
        )

    def test_plan_includes_host_profile(self) -> None:
        profile_path = Path("/tmp/beateros-host-profile.json")
        plan = local_e2e.build_plan("python3", branch_base_available=True, host_profile=profile_path)
        bare_metal = next(gate for gate in plan if gate.name == "bare-metal-readiness")
        self.assertEqual(
            bare_metal.command[-3:],
            ("--strict-host-context", "--host-profile", str(profile_path)),
        )

    def test_branch_whitespace_uses_origin_main_when_available(self) -> None:
        plan = local_e2e.build_plan("python3", branch_base_available=True)
        branch = next(gate for gate in plan if gate.name == "branch-whitespace")
        self.assertEqual(branch.command, ("git", "diff", "--check", "origin/main...HEAD"))

    def test_branch_whitespace_skips_when_origin_main_missing(self) -> None:
        # #62: a fresh/shallow clone has no origin/main ref; the gate must degrade
        # to a passing skip rather than error on the unknown ref.
        plan = local_e2e.build_plan("python3", branch_base_available=False)
        branch = next(gate for gate in plan if gate.name == "branch-whitespace")
        self.assertEqual(branch.command[:2], ("python3", "-c"))
        completed = subprocess.run(branch.command, capture_output=True, check=False)
        self.assertEqual(completed.returncode, 0)
        self.assertIn(b"skipped", completed.stdout)


class RunnerTest(unittest.TestCase):
    def test_fail_fast_stops_after_first_failure(self) -> None:
        plan = [
            local_e2e.Gate("ok", ("true",)),
            local_e2e.Gate("bad", ("false",)),
            local_e2e.Gate("later", ("true",)),
        ]
        seen: list[str] = []

        def execute(gate: local_e2e.Gate) -> local_e2e.GateResult:
            seen.append(gate.name)
            code = 7 if gate.name == "bad" else 0
            return local_e2e.GateResult(gate=gate, returncode=code, elapsed_s=0.0)

        results = local_e2e.run_plan(plan, full_report=False, execute=execute)
        self.assertEqual(seen, ["ok", "bad"])
        self.assertEqual(local_e2e.exit_code(results), 1)

    def test_full_report_runs_after_failure(self) -> None:
        plan = [
            local_e2e.Gate("ok", ("true",)),
            local_e2e.Gate("bad", ("false",)),
            local_e2e.Gate("later", ("true",)),
        ]
        seen: list[str] = []

        def execute(gate: local_e2e.Gate) -> local_e2e.GateResult:
            seen.append(gate.name)
            code = 7 if gate.name == "bad" else 0
            return local_e2e.GateResult(gate=gate, returncode=code, elapsed_s=0.0)

        results = local_e2e.run_plan(plan, full_report=True, execute=execute)
        self.assertEqual(seen, ["ok", "bad", "later"])
        self.assertEqual(local_e2e.exit_code(results), 1)

    def test_success_exit_code(self) -> None:
        gate = local_e2e.Gate("ok", ("true",))
        results = [local_e2e.GateResult(gate=gate, returncode=0, elapsed_s=0.0)]
        self.assertEqual(local_e2e.exit_code(results), 0)


if __name__ == "__main__":
    unittest.main()
