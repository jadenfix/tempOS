#!/usr/bin/env python3
"""Exercise beater-osd-http's scheduler claim/complete routes."""

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
from datetime import datetime, timezone
from pathlib import Path
from typing import Any

REPO_ROOT = Path(__file__).resolve().parent.parent
TOKEN = "beateros-http-claims-smoke-token"
SESSION_ID = "http-claims-smoke-session"
GRANT_ID = f"{SESSION_ID}-root-grant"
REGISTER_ACTION_ID = "http-claims-register-tool"
CLAIM_ACTION_ID = "http-claims-worker-action"
LEASE_ID = "lease-http-claims-worker"
WORKER_INPUT_DIGEST = "1111111111111111111111111111111111111111111111111111111111111111"


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
            "agent:http-claims-smoke",
            "--workspace",
            "workspace:http-claims-smoke",
            "--goal",
            "prove daemon HTTP claim and completion routes",
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
            "daemon HTTP claim smoke",
        ],
    )


def register_shell_tool(root: Path, token_file: Path, workdir: Path) -> tuple[str, str, str]:
    body = {
        "action_id": REGISTER_ACTION_ID,
        "tool": "shell",
        "command": "sh",
        "args": ["-c", "printf claim-smoke > claim-register.txt"],
        "cwd": str(workdir),
        "grants": [GRANT_ID],
        "side_effects": ["local_write"],
        "timeout_secs": 30,
    }
    status, payload = one_shot_request(
        root,
        token_file,
        f"/v1/sessions/{SESSION_ID}/actions/execute-local-shell",
        body,
        token=TOKEN,
    )
    if status != 200:
        raise RuntimeError(f"expected 200 from registration execution, got {status}: {payload}")
    evidence = payload.get("evidence") or {}
    tool_ref = evidence.get("tool_ref", "")
    if "@" not in tool_ref or "#" not in tool_ref:
        raise RuntimeError(f"execution evidence did not include pinned tool_ref: {payload}")
    tool_id, rest = tool_ref.split("@", 1)
    version, digest = rest.split("#", 1)
    if tool_id != "shell" or not version or not digest:
        raise RuntimeError(f"unexpected tool_ref: {tool_ref}")
    return version, digest, tool_ref


def propose_claimable_action(root: Path, workdir: Path, worker_input_digest: str) -> tuple[str, str]:
    cargo_bin(
        "beaterosctl",
        [
            "--home",
            str(root),
            "action",
            "propose",
            "--session",
            SESSION_ID,
            "--action-id",
            CLAIM_ACTION_ID,
            "--tool",
            "shell",
            "--kind",
            "execute",
            "--target-kind",
            "file_path",
            "--target",
            str(workdir),
            "--grants",
            GRANT_ID,
            "--side-effects",
            "local_write",
            "--inputs-digest",
            worker_input_digest,
            "--max-wall-ms",
            "30000",
            "--summary",
            "claimed local-shell worker action",
        ],
    )
    export = cargo_bin(
        "beaterosctl",
        ["--home", str(root), "trace", "export", "--session", SESSION_ID],
        capture=True,
    ).stdout
    bundle = json.loads(export)
    decisions = [decision for decision in bundle["decisions"] if decision["action_id"] == CLAIM_ACTION_ID]
    if len(decisions) != 1:
        raise RuntimeError(f"expected one decision for {CLAIM_ACTION_ID}: {bundle}")
    decision = decisions[0]
    if decision["result"] != "allowed":
        raise RuntimeError(f"expected allowed claim action decision: {decision}")
    return decision["decision_id"], decision["manifest_hash"]


def utc_now() -> str:
    return datetime.now(timezone.utc).isoformat().replace("+00:00", "Z")


def run_smoke(root: Path, *, as_json: bool) -> int:
    workdir = root / "work"
    workdir.mkdir(parents=True, exist_ok=True)
    token_file = root / "token"
    token_file.write_text(TOKEN, encoding="utf-8")
    setup_store(root, workdir)

    tool_version, tool_digest, tool_ref = register_shell_tool(root, token_file, workdir)
    decision_id, manifest_hash = propose_claimable_action(root, workdir, WORKER_INPUT_DIGEST)

    claim_path = f"/v1/sessions/{SESSION_ID}/actions/{CLAIM_ACTION_ID}/claims"
    bad_claim = {
        "expected_manifest_hash": manifest_hash,
        "expected_decision_id": decision_id,
        "expected_tool_version": tool_version,
        "expected_tool_digest": "0" * 64,
        "lease_id": LEASE_ID,
    }
    bad_status, bad_payload = one_shot_request(root, token_file, claim_path, bad_claim, token=TOKEN)
    if bad_status != 403:
        raise RuntimeError(f"expected 403 for wrong tool digest, got {bad_status}: {bad_payload}")

    claim_body = {
        "expected_manifest_hash": manifest_hash,
        "expected_decision_id": decision_id,
        "expected_tool_version": tool_version,
        "expected_tool_digest": tool_digest,
        "lease_id": LEASE_ID,
    }
    claim_status, claim = one_shot_request(root, token_file, claim_path, claim_body, token=TOKEN)
    if claim_status != 201:
        raise RuntimeError(f"expected 201 from claim route, got {claim_status}: {claim}")
    if claim.get("lease_id") != LEASE_ID or claim.get("tool_ref") != tool_ref:
        raise RuntimeError(f"claim response did not preserve lease/tool authority: {claim}")
    if claim.get("required_grants") != [GRANT_ID]:
        raise RuntimeError(f"claim response did not return derived grants: {claim}")
    if claim.get("requested_budget", {}).get("max_wall_ms") != 30000:
        raise RuntimeError(f"claim response did not return derived wall budget: {claim}")

    receipt_body = {
        "receipt_id": "receipt-http-claims-worker",
        "action_id": CLAIM_ACTION_ID,
        "tool_id": "shell",
        "target": claim["target"],
        "started_at": claim["leased_at"],
        "finished_at": utc_now(),
        "status": "ok",
        "input_digest": WORKER_INPUT_DIGEST,
        "output_digest": "http-claims-worker-output-digest",
        "side_effect_summary": "claim smoke completed without spawning a second process",
        "side_effects": [],
        "external_ids": [f"tool_ref={tool_ref}", f"lease_id={LEASE_ID}"],
        "artifact_refs": [],
    }
    wrong_path = f"/v1/sessions/{SESSION_ID}/actions/{CLAIM_ACTION_ID}/claims/{LEASE_ID}-wrong/complete"
    wrong_status, wrong_payload = one_shot_request(root, token_file, wrong_path, receipt_body, token=TOKEN)
    if wrong_status != 403:
        raise RuntimeError(f"expected 403 for wrong lease id, got {wrong_status}: {wrong_payload}")

    complete_path = f"/v1/sessions/{SESSION_ID}/actions/{CLAIM_ACTION_ID}/claims/{LEASE_ID}/complete"
    complete_status, complete = one_shot_request(root, token_file, complete_path, receipt_body, token=TOKEN)
    if complete_status != 200:
        raise RuntimeError(f"expected 200 from complete route, got {complete_status}: {complete}")
    if complete.get("lease_id") != LEASE_ID or complete.get("receipt_id") != receipt_body["receipt_id"]:
        raise RuntimeError(f"completion response mismatch: {complete}")

    verify = cargo_bin(
        "beaterosctl",
        ["--home", str(root), "journal", "verify", "--session", SESSION_ID],
        capture=True,
    ).stdout
    if "journal OK" not in verify or "receipts:      2" not in verify:
        raise RuntimeError(f"journal verification did not prove two receipts:\n{verify}")

    report = {
        "command": "beater-osd-http-claims-smoke",
        "session_id": SESSION_ID,
        "claim_action_id": CLAIM_ACTION_ID,
        "lease_id": LEASE_ID,
        "lease_hash": claim["lease_hash"],
        "receipt_id": complete["receipt_id"],
        "receipt_hash": complete["receipt_hash"],
        "tool_ref": tool_ref,
    }
    if as_json:
        print(json.dumps(report, indent=2, sort_keys=True))
    else:
        print("beater-osd-http claim smoke OK")
        print(f"  session: {SESSION_ID}")
        print(f"  lease:   {LEASE_ID} hash={claim['lease_hash']}")
        print(f"  receipt: {complete['receipt_id']} hash={complete['receipt_hash']}")
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

    with tempfile.TemporaryDirectory(prefix="beater-osd-http-claims-smoke-") as temporary:
        root = Path(temporary).resolve()
        try:
            return run_smoke(root, as_json=args.json)
        finally:
            if args.keep_root:
                stable = root.parent / f"kept-{root.name}"
                if stable.exists():
                    shutil.rmtree(stable)
                shutil.copytree(root, stable)
                print(f"beater-osd-http claim smoke root preserved at: {stable}")


if __name__ == "__main__":
    try:
        raise SystemExit(main())
    except Exception as error:  # noqa: BLE001 - script-level diagnostic boundary.
        print(f"beater-osd-http claim smoke failed: {error}", file=sys.stderr)
        raise SystemExit(1)
