#!/usr/bin/env python3
"""Collect a deterministic beaterOS bare-metal host profile."""

from __future__ import annotations

import argparse
import json
import os
import platform
from dataclasses import dataclass
from pathlib import Path
from typing import Any


HOST_RESOURCE_KEYS: dict[str, str] = {
    "memory_gib": "BEATEROS_HOST_MEMORY_GIB",
    "storage_iops": "BEATEROS_HOST_STORAGE_IOPS",
    "memory_bandwidth_gbps": "BEATEROS_HOST_MEMORY_BANDWIDTH_GBPS",
    "gpu_mem_gib": "BEATEROS_HOST_GPU_MEM_GIB",
    "pcie_bwl_gbps": "BEATEROS_HOST_PCIE_BWL_GBPS",
    "residual_latency_ms": "BEATEROS_HOST_RESIDUAL_LATENCY_MS",
    "gpu_temp_c": "BEATEROS_HOST_GPU_TEMP_C",
}


@dataclass(frozen=True)
class HostProfile:
    os_name: str
    arch: str
    cpu_cores: int | None
    memory_gib: float | None
    storage_iops: float | None
    memory_bandwidth_gbps: float | None
    gpu_mem_gib: float | None
    pcie_bwl_gbps: float | None
    residual_latency_ms: float | None
    gpu_temp_c: float | None
    accelerators: list[str]


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


def _safe_cpu_count() -> int | None:
    count = os.cpu_count()
    if count is None or count <= 0:
        return None
    return count


def _safe_memory_gib() -> float | None:
    page_size = getattr(os, "sysconf", None)
    if page_size is None:
        return None
    try:
        page_size = os.sysconf("SC_PAGE_SIZE")
        pages = os.sysconf("SC_PHYS_PAGES")
    except (AttributeError, ValueError, OSError):
        return None
    if not page_size or not pages:
        return None
    return (page_size * pages) / (1024 ** 3)


def collect_profile() -> HostProfile:
    os_name = os.environ.get("BEATEROS_HOST_OS", "").strip().lower() or platform.system().lower()
    arch = os.environ.get("BEATEROS_HOST_ARCH", "").strip().lower() or platform.machine().lower()

    override_accelerators = {
        "cpu": parse_bool(os.environ.get("BEATEROS_ACCELERATOR_CPU", "true")),
        "cuda": parse_bool(os.environ.get("BEATEROS_ACCELERATOR_CUDA", "false")),
        "apple_gpu": parse_bool(os.environ.get("BEATEROS_ACCELERATOR_APPLE_GPU", "false")),
        "tpu": parse_bool(os.environ.get("BEATEROS_ACCELERATOR_TPU", "false")),
        "secure_enclave": parse_bool(os.environ.get("BEATEROS_ACCELERATOR_ENCLAVE", "false")),
    }
    accelerators = [name for name, enabled in override_accelerators.items() if enabled]

    values = {name: parse_numeric(os.environ.get(env_var, "")) for name, env_var in HOST_RESOURCE_KEYS.items()}

    profile = HostProfile(
        os_name=os_name,
        arch=arch,
        cpu_cores=_safe_cpu_count(),
        memory_gib=parse_numeric(values["memory_gib"]) if values["memory_gib"] is not None else _safe_memory_gib(),
        storage_iops=values["storage_iops"],
        memory_bandwidth_gbps=values["memory_bandwidth_gbps"],
        gpu_mem_gib=values["gpu_mem_gib"],
        pcie_bwl_gbps=values["pcie_bwl_gbps"],
        residual_latency_ms=values["residual_latency_ms"],
        gpu_temp_c=values["gpu_temp_c"],
        accelerators=accelerators,
    )
    return profile


def to_payload(profile: HostProfile) -> dict[str, Any]:
    return {
        "schema_version": 1,
        "host": {
            "os": profile.os_name,
            "arch": profile.arch,
            "cpu_cores": profile.cpu_cores,
            "memory_gib": profile.memory_gib,
            "storage_iops": profile.storage_iops,
            "memory_bandwidth_gbps": profile.memory_bandwidth_gbps,
            "gpu_mem_gib": profile.gpu_mem_gib,
            "pcie_bwl_gbps": profile.pcie_bwl_gbps,
            "residual_latency_ms": profile.residual_latency_ms,
            "gpu_temp_c": profile.gpu_temp_c,
            "accelerators": sorted(profile.accelerators),
        },
        "metadata": {
            "source": "runtime-env",
            "collect_method": "collect-bare-metal-host-profile.py",
        },
    }


def parse_profile(payload: dict[str, Any]) -> dict[str, Any]:
    if "host" in payload and isinstance(payload["host"], dict):
        host = payload["host"]
    elif isinstance(payload, dict):
        host = payload
    else:
        raise ValueError("invalid host profile shape")

    host = dict(host)
    return host


def load_profile(path: Path) -> dict[str, Any]:
    with path.open("r", encoding="utf-8") as handle:
        payload = json.load(handle)
    if not isinstance(payload, dict):
        raise ValueError("host profile payload must be an object")
    return parse_profile(payload)


def main() -> int:
    parser = argparse.ArgumentParser(description="Collect deterministic host profile for bare-metal checks.")
    parser.add_argument(
        "--print-json",
        action="store_true",
        help="print profile JSON payload",
    )
    parser.add_argument(
        "--pretty",
        action="store_true",
        help="pretty-print JSON (implies --print-json)",
    )
    parser.add_argument(
        "--out",
        type=Path,
        help="write profile payload to path",
    )
    parser.add_argument(
        "--load",
        type=Path,
        help="validate and re-emit a profile",
    )

    args = parser.parse_args()
    try:
        if args.load is not None:
            profile = load_profile(args.load)
            payload = {"schema_version": 1, "host": profile, "metadata": {"source": "load-rewrite"}}
        else:
            profile = collect_profile()
            payload = to_payload(profile)
    except (OSError, ValueError, json.JSONDecodeError) as exc:
        print(f"ERROR: {exc}")
        return 1

    if args.out is not None:
        args.out.parent.mkdir(parents=True, exist_ok=True)
        with args.out.open("w", encoding="utf-8") as handle:
            json.dump(payload, handle, sort_keys=True, indent=2 if args.pretty else None)
    if args.print_json or args.pretty:
        print(json.dumps(payload, sort_keys=True, indent=2 if args.pretty else None))
    if not args.print_json and not args.pretty and args.out is None:
        print(json.dumps(payload["host"], sort_keys=True))
    return 0


if __name__ == "__main__":
    raise SystemExit(main())
