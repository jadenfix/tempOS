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


@dataclass(frozen=True)
class HostContext:
    os_name: str
    arch: str
    cpu_cores: int | None = None
    accelerators: frozenset[str] = frozenset()


def parse_bool(value: object) -> bool:
    if isinstance(value, bool):
        return value
    if isinstance(value, str):
        lower = value.strip().lower()
        return lower in {"1", "true", "yes", "on"}
    return False


def load_manifest(manifest_path: Path) -> dict[str, Any]:
    if not manifest_path.exists():
        raise ValueError(f"missing manifest: {manifest_path}")
    with manifest_path.open("r", encoding="utf-8") as handle:
        data = json.load(handle)
    if not isinstance(data, dict):
        raise TypeError("manifest must be a JSON object")
    return data


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

    return errors


def normalize_host_context() -> HostContext:
    # Keep deterministic local behavior for test envs where a test can inject
    # override markers in environment-like fixtures.
    # Use conservative values when runtime introspection is unavailable.
    raw_os = os.environ.get("BEATEROS_HOST_OS", "").strip().lower()
    raw_arch = os.environ.get("BEATEROS_HOST_ARCH", "").strip().lower()
    if not raw_os:
        raw_os = platform.system().lower()
    if not raw_arch:
        raw_arch = platform.machine().lower()
    accelerator_set = set()
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

    cpu_cores = os.cpu_count()
    return HostContext(os_name=raw_os, arch=raw_arch, cpu_cores=cpu_cores, accelerators=frozenset(accelerator_set))


def profile_supports_host(profile: dict[str, Any], host: HostContext) -> bool:
    if host.os_name not in {os_name.lower() for os_name in profile.get("target_os", [])}:
        return False
    if host.arch not in {arch.lower() for arch in profile.get("target_arch", [])}:
        return False

    resource_contract = profile.get("resource_contract", {})
    min_cpu_cores = resource_contract.get("min_cpu_cores")
    if isinstance(min_cpu_cores, int) and host.cpu_cores is not None and host.cpu_cores < min_cpu_cores:
        return False

    for accel in profile.get("accelerators", []):
        required = parse_bool(accel.get("required"))
        if required and accel.get("kind") not in host.accelerators:
            return False

    return True


def check(manifest: dict[str, Any], host_check: bool = False, require_profile: str | None = None) -> int:
    errors = validate_manifest(manifest)
    if errors:
        for err in errors:
            print(f"ERROR: {err}")
        return 1

    if not host_check:
        print("PASS: bare-metal readiness manifest schema is valid")
        return 0

    host = normalize_host_context()
    profiles = manifest.get("profiles", [])
    matching_profiles = [p["name"] for p in profiles if profile_supports_host(p, host)]
    if not matching_profiles:
        print("ERROR: no manifest profile supports the current host context")
        return 1

    if require_profile is not None and require_profile not in matching_profiles:
        print(f"ERROR: required profile not supported on current host: {require_profile}")
        return 1

    print(f"INFO: host context = os={host.os_name}, arch={host.arch}, cpus={host.cpu_cores or 'unknown'}")
    print(f"INFO: matching profiles = {', '.join(matching_profiles)}")
    print("PASS: host context is represented in the bare-metal readiness manifest")
    return 0


def run_and_dump_json(args: argparse.Namespace, manifest: dict[str, Any]) -> int:
    if not args.report:
        return check(
            manifest,
            host_check=args.check_host or args.require_profile is not None,
            require_profile=args.require_profile,
        )

    host = normalize_host_context()
    profiles = manifest.get("profiles", [])
    matching_profiles = [p["name"] for p in profiles if profile_supports_host(p, host)]
    payload = {
        "schema_version": manifest.get("schema_version"),
        "profiles": len(profiles),
        "supported_profiles": matching_profiles,
        "host": {
            "os": host.os_name,
            "arch": host.arch,
            "cpu_cores": host.cpu_cores,
            "accelerators": sorted(host.accelerators),
        },
    }
    print(json.dumps(payload, sort_keys=True))
    code = check(
        manifest,
        host_check=args.check_host or args.require_profile is not None,
        require_profile=args.require_profile,
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
        "--report",
        action="store_true",
        help="print JSON host-profile coverage report",
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
