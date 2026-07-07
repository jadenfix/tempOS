#!/usr/bin/env python3
"""Exercise beater-osd-http's token-gated local-shell execution route."""

from __future__ import annotations

import argparse
import http.client
import json
import shutil
import socket
import subprocess
import sys
import tempfile
import time
from pathlib import Path
from typing import Any

REPO_ROOT = Path(__file__).resolve().parent.parent
TOKEN = "beateros-http-smoke-token"
SESSION_ID = "http-exec-smoke-session"
GRANT_ID = "http-exec-smoke-grant"
ACTION_ID = "http-exec-smoke-action"


def run(command: list[str], *, capture: bool = False) -> subprocess.CompletedProcess[str]:
    return subprocess.run(
        command,
        cwd=REPO_ROOT,
        check=True,
        text=True,
        capture_output=capture,
    )


def cargo_bin(package: str, args: list[str], *, capture: bool = False) -> subprocess.CompletedProcess[str]:
    return run(["cargo", "run", "-q", "-p", package, "--", *args], capture=capture)


def free_loopback_port() -> int:
    with socket.socket(socket.AF_INET, socket.SOCK_STREAM) as sock:
        sock.bind(("127.0.0.1", 0))
        return int(sock.getsockname()[1])


def start_server(root: Path, token_file: Path, port: int) -> subprocess.Popen[str]:
    return subprocess.Popen(
        [
            "cargo",
            "run",
            "-q",
            "-p",
            "beater-osd-http",
            "--",
            "serve",
            "--root",
            str(root),
            "--token-file",
            str(token_file),
            "--bind",
            f"127.0.0.1:{port}",
            "--once",
        ],
        cwd=REPO_ROOT,
        text=True,
        stdout=subprocess.PIPE,
        stderr=subprocess.PIPE,
    )


def request(port: int, path: str, body: dict[str, Any], *, token: str | None) -> tuple[int, dict[str, Any]]:
    headers = {"content-type": "application/json"}
    if token is not None:
        headers["authorization"] = f"Bearer {token}"
    encoded = json.dumps(body).encode("utf-8")
    connection = http.client.HTTPConnection("127.0.0.1", port, timeout=45)
    try:
        connection.request("POST", path, body=encoded, headers=headers)
        response = connection.getresponse()
        payload = json.loads(response.read().decode("utf-8"))
        return response.status, payload
    finally:
        connection.close()


def wait_server(process: subprocess.Popen[str]) -> None:
    stdout, stderr = process.communicate(timeout=30)
    if process.returncode != 0:
        raise RuntimeError(
            f"beater-osd-http exited {process.returncode}\nSTDOUT:\n{stdout}\nSTDERR:\n{stderr}"
        )


def stop_server(process: subprocess.Popen[str]) -> None:
    if process.poll() is not None:
        return
    process.terminate()
    try:
        process.communicate(timeout=5)
    except subprocess.TimeoutExpired:
        process.kill()
        process.communicate(timeout=5)


def one_shot_request(
    root: Path,
    token_file: Path,
    path: str,
    body: dict[str, Any],
    *,
    token: str | None,
) -> tuple[int, dict[str, Any]]:
    port = free_loopback_port()
    server = start_server(root, token_file, port)
    try:
        deadline = time.monotonic() + 15
        last_error: Exception | None = None
        while time.monotonic() < deadline:
            if server.poll() is not None:
                break
            try:
                response = request(port, path, body, token=token)
                wait_server(server)
                return response
            except (ConnectionRefusedError, TimeoutError, OSError) as error:
                last_error = error
                time.sleep(0.1)
        if server.poll() is not None:
            stdout, stderr = server.communicate(timeout=1)
            raise RuntimeError(
                f"beater-osd-http exited before request; return={server.returncode}\nSTDOUT:\n{stdout}\nSTDERR:\n{stderr}"
            )
        raise RuntimeError(f"beater-osd-http did not accept request: {last_error}")
    except Exception:
        stop_server(server)
        raise


def setup_store(root: Path, workdir: Path) -> None:
    cargo_bin(
        "beaterosctl",
        [
            "--home",
            str(root),
            "session",
            "create",
            "--session",
            SESSION_ID,
            "--agent",
            "agent:http-smoke",
            "--workspace",
            "workspace:http-smoke",
            "--goal",
            "prove daemon HTTP execution route",
        ],
    )
    cargo_bin(
        "beaterosctl",
        [
            "--home",
            str(root),
            "grant",
            "issue",
            "--session",
            SESSION_ID,
            "--grant-id",
            GRANT_ID,
            "--resource-kind",
            "file_path",
            "--actions",
            "execute",
            "--path-prefix",
            str(workdir),
            "--reason",
            "daemon HTTP execution smoke",
        ],
    )


def run_smoke(root: Path, *, as_json: bool) -> int:
    workdir = root / "work"
    workdir.mkdir(parents=True, exist_ok=True)
    token_file = root / "token"
    token_file.write_text(TOKEN, encoding="utf-8")
    setup_store(root, workdir)

    body = {
        "action_id": ACTION_ID,
        "tool": "shell",
        "command": "sh",
        "args": ["-c", "printf http-smoke > http-out.txt"],
        "cwd": str(workdir),
        "grants": [GRANT_ID],
        "side_effects": ["local_write"],
        "timeout_secs": 30,
    }
    path = f"/v1/sessions/{SESSION_ID}/actions/execute-local-shell"

    unauth_status, unauth_payload = one_shot_request(root, token_file, path, body, token=None)
    if unauth_status != 401:
        raise RuntimeError(f"expected 401 without bearer token, got {unauth_status}: {unauth_payload}")

    wrong_status, wrong_payload = one_shot_request(root, token_file, path, body, token="wrong-token")
    if wrong_status != 401:
        raise RuntimeError(f"expected 401 with wrong bearer token, got {wrong_status}: {wrong_payload}")

    pre_show = cargo_bin(
        "beaterosctl",
        ["--home", str(root), "session", "show", "--session", SESSION_ID],
        capture=True,
    ).stdout
    if "actions:    0" not in pre_show or "receipts:   0" not in pre_show:
        raise RuntimeError(f"unauthorized requests changed runtime state:\\n{pre_show}")

    status, payload = one_shot_request(root, token_file, path, body, token=TOKEN)
    if status != 200:
        raise RuntimeError(f"expected 200 from execution route, got {status}: {payload}")
    if payload.get("decision") != "allowed":
        raise RuntimeError(f"expected allowed decision: {payload}")
    if payload.get("execution", {}).get("status") != "ok":
        raise RuntimeError(f"expected ok execution: {payload}")
    receipt = payload.get("receipt")
    if not receipt or not receipt.get("receipt_hash"):
        raise RuntimeError(f"expected receipt with hash: {payload}")
    output_path = workdir / "http-out.txt"
    if output_path.read_text(encoding="utf-8") != "http-smoke":
        raise RuntimeError(f"unexpected output file content at {output_path}")

    verify = cargo_bin(
        "beaterosctl",
        ["--home", str(root), "journal", "verify", "--session", SESSION_ID],
        capture=True,
    ).stdout
    if "journal OK" not in verify or "receipts:      1" not in verify:
        raise RuntimeError(f"journal verification did not prove one receipt:\n{verify}")

    report = {
        "command": "beater-osd-http-execute-smoke",
        "session_id": SESSION_ID,
        "status": payload["execution"]["status"],
        "decision": payload["decision"],
        "receipt_id": receipt["receipt_id"],
        "receipt_hash": receipt["receipt_hash"],
        "output": str(output_path),
    }
    if as_json:
        print(json.dumps(report, indent=2, sort_keys=True))
    else:
        print("beater-osd-http execution smoke OK")
        print(f"  session: {SESSION_ID}")
        print(f"  decision: {payload['decision']}")
        print(f"  execution: {payload['execution']['status']}")
        print(f"  receipt: {receipt['receipt_id']} hash={receipt['receipt_hash']}")
    return 0


def main() -> int:
    parser = argparse.ArgumentParser(description=__doc__)
    parser.add_argument("--json", action="store_true", help="emit a machine-readable smoke report")
    parser.add_argument("--root", type=Path, help="runtime root; defaults to a temporary directory")
    parser.add_argument("--keep-root", action="store_true", help="preserve a temporary runtime root")
    args = parser.parse_args()

    if args.root is not None:
        args.root.mkdir(parents=True, exist_ok=True)
        return run_smoke(args.root.resolve(), as_json=args.json)

    with tempfile.TemporaryDirectory(prefix="beater-osd-http-smoke-") as temporary:
        root = Path(temporary).resolve()
        try:
            return run_smoke(root, as_json=args.json)
        finally:
            if args.keep_root:
                stable = root.parent / f"kept-{root.name}"
                if stable.exists():
                    shutil.rmtree(stable)
                shutil.copytree(root, stable)
                print(f"beater-osd-http smoke root preserved at: {stable}")


if __name__ == "__main__":
    try:
        raise SystemExit(main())
    except Exception as error:  # noqa: BLE001 - script-level diagnostic boundary.
        print(f"beater-osd-http execution smoke failed: {error}", file=sys.stderr)
        raise SystemExit(1)
