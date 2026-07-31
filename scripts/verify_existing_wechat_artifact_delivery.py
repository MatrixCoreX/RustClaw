#!/usr/bin/env python3
"""Verify an existing real task artifact without sending a new channel message."""

from __future__ import annotations

import argparse
import hashlib
import json
import os
from pathlib import Path
import sqlite3
import subprocess
import sys
from typing import Any
from urllib.error import HTTPError
from urllib.request import Request, urlopen

try:
    import tomllib
except ModuleNotFoundError as error:  # pragma: no cover - Python version gate
    raise SystemExit("Python 3.11+ is required") from error


ROOT = Path(__file__).resolve().parents[1]
DEFAULT_CONFIG = ROOT / "configs" / "config.toml"
DEFAULT_LOG = ROOT / "logs" / "wechatd.log"


class VerificationError(RuntimeError):
    pass


def parse_args(argv: list[str] | None = None) -> argparse.Namespace:
    parser = argparse.ArgumentParser(
        description=(
            "Verify an existing succeeded WeChat task artifact through its immutable "
            "manifest, UI URLs, current authenticated download, and prior channel upload log. "
            "This command never sends a new message."
        )
    )
    parser.add_argument("--task-id", help="specific existing task; otherwise select the newest proof")
    parser.add_argument("--base-url", default="http://127.0.0.1:8787")
    parser.add_argument("--config", type=Path, default=DEFAULT_CONFIG)
    parser.add_argument("--wechat-log", type=Path, default=DEFAULT_LOG)
    parser.add_argument("--output", type=Path, required=True)
    return parser.parse_args(argv)


def sha256_bytes(content: bytes) -> str:
    return hashlib.sha256(content).hexdigest()


def command_stdout(command: list[str]) -> str | None:
    completed = subprocess.run(
        command,
        cwd=ROOT,
        check=False,
        capture_output=True,
        text=True,
    )
    output = completed.stdout.strip()
    return output if completed.returncode == 0 and output else None


def repo_path(path: Path) -> str:
    return os.path.relpath(path.resolve(), ROOT)


def database_path(config_path: Path) -> Path:
    config = tomllib.loads(config_path.read_text(encoding="utf-8"))
    configured = config.get("database", {}).get("sqlite_path")
    if not isinstance(configured, str) or not configured.strip():
        raise VerificationError("database.sqlite_path is not configured")
    return (ROOT / configured).resolve()


def enabled_admin_key(connection: sqlite3.Connection) -> str:
    row = connection.execute(
        """
        SELECT user_key
        FROM auth_keys
        WHERE role = 'admin' AND enabled = 1
        ORDER BY created_at
        LIMIT 1
        """
    ).fetchone()
    key = str(row[0] if row else "").strip()
    if not key:
        raise VerificationError("no enabled admin key is available for local verification")
    return key


def result_artifacts(raw_result: str | None) -> list[dict[str, Any]]:
    try:
        result = json.loads(raw_result or "{}")
    except json.JSONDecodeError:
        return []
    artifacts = result.get("artifacts") if isinstance(result, dict) else None
    if not isinstance(artifacts, list):
        return []
    return [
        artifact
        for artifact in artifacts
        if isinstance(artifact, dict)
        and artifact.get("kind") == "image"
        and str(artifact.get("download_url") or "").startswith("/v1/tasks/")
        and str(artifact.get("preview_url") or "").startswith("/v1/tasks/")
    ]


def delivery_artifact_path(task_id: str, artifact: dict[str, Any]) -> Path:
    artifact_id = str(artifact.get("id") or "").strip()
    raw_filename = str(artifact.get("filename") or "").strip()
    filename = Path(raw_filename).name
    if (
        not artifact_id
        or not filename
        or Path(task_id).name != task_id
        or Path(artifact_id).name != artifact_id
        or filename != raw_filename
    ):
        raise VerificationError("artifact manifest is missing id or filename")
    path = (
        ROOT
        / ".agent-runtime"
        / "artifacts"
        / "delivery"
        / task_id
        / artifact_id
        / filename
    ).resolve()
    delivery_root = (ROOT / ".agent-runtime" / "artifacts" / "delivery").resolve()
    if delivery_root not in path.parents:
        raise VerificationError("artifact manifest resolved outside the delivery root")
    return path


def logged_wechat_delivery(
    log_text: str,
    artifact_path: Path,
) -> dict[str, Any] | None:
    expected = str(artifact_path)
    matched_line_number: int | None = None
    matched_line = ""
    error_after_upload = False
    for line_number, line in enumerate(log_text.splitlines(), start=1):
        if (
            "wechat-ilink: outbound image uploaded path=" in line
            and expected in line
        ):
            matched_line_number = line_number
            matched_line = line
            error_after_upload = False
            continue
        if matched_line_number is not None and expected in line and " err=" in line:
            error_after_upload = True
    if matched_line_number is None:
        return None
    timestamp = matched_line.split(maxsplit=1)[0] if matched_line else None
    return {
        "log_path": repo_path(DEFAULT_LOG),
        "line_number": matched_line_number,
        "timestamp": timestamp,
        "event_sha256": sha256_bytes(matched_line.encode()),
        "outbound_encrypted_upload_observed": True,
        "later_delivery_error_observed": error_after_upload,
    }


def http_json(base_url: str, path: str, admin_key: str) -> dict[str, Any]:
    request = Request(
        base_url.rstrip("/") + path,
        method="GET",
        headers={"X-Agent-Key": admin_key},
    )
    try:
        with urlopen(request, timeout=30) as response:
            return json.loads(response.read().decode("utf-8"))
    except HTTPError as error:
        payload = error.read().decode(errors="replace")
        raise VerificationError(f"GET {path} returned HTTP {error.code}: {payload}") from error


def http_bytes(base_url: str, path: str, admin_key: str) -> bytes:
    request = Request(
        base_url.rstrip("/") + path,
        method="GET",
        headers={"X-Agent-Key": admin_key},
    )
    try:
        with urlopen(request, timeout=30) as response:
            return response.read()
    except HTTPError as error:
        payload = error.read().decode(errors="replace")
        raise VerificationError(f"GET {path} returned HTTP {error.code}: {payload}") from error


def select_existing_proof(
    connection: sqlite3.Connection,
    log_text: str,
    task_id: str | None,
) -> tuple[str, dict[str, Any], Path, dict[str, Any]]:
    if task_id:
        rows = connection.execute(
            """
            SELECT task_id, result_json
            FROM tasks
            WHERE task_id = ? AND channel = 'wechat' AND status = 'succeeded'
            """,
            (task_id,),
        ).fetchall()
    else:
        rows = connection.execute(
            """
            SELECT task_id, result_json
            FROM tasks
            WHERE channel = 'wechat' AND status = 'succeeded'
            ORDER BY CAST(created_at AS INTEGER) DESC
            LIMIT 500
            """
        ).fetchall()
    for selected_task_id, raw_result in rows:
        for artifact in result_artifacts(raw_result):
            artifact_path = delivery_artifact_path(str(selected_task_id), artifact)
            logged = logged_wechat_delivery(log_text, artifact_path)
            if artifact_path.is_file() and logged and not logged["later_delivery_error_observed"]:
                return str(selected_task_id), artifact, artifact_path, logged
    if task_id:
        raise VerificationError(f"task {task_id} has no complete WeChat artifact proof")
    raise VerificationError("no existing complete WeChat artifact proof was found")


def verify(args: argparse.Namespace) -> dict[str, Any]:
    config_path = args.config.expanduser().resolve()
    log_path = args.wechat_log.expanduser().resolve()
    connection = sqlite3.connect(database_path(config_path))
    try:
        admin_key = enabled_admin_key(connection)
        log_text = log_path.read_text(encoding="utf-8", errors="replace")
        task_id, manifest, local_path, log_evidence = select_existing_proof(
            connection,
            log_text,
            args.task_id,
        )
    finally:
        connection.close()
    log_evidence["log_path"] = repo_path(log_path)

    list_response = http_json(
        args.base_url,
        f"/v1/tasks/{task_id}/artifacts",
        admin_key,
    )
    listed = ((list_response.get("data") or {}).get("artifacts") or [])
    listed_manifest = next(
        (
            item
            for item in listed
            if isinstance(item, dict) and item.get("id") == manifest.get("id")
        ),
        None,
    )
    if not list_response.get("ok") or not listed_manifest:
        raise VerificationError("artifact is missing from the authenticated task manifest API")
    content = http_bytes(args.base_url, str(manifest["download_url"]), admin_key)
    local_content = local_path.read_bytes()
    content_digest = sha256_bytes(content)
    expected_digest = str(manifest.get("sha256") or "")
    if (
        content != local_content
        or len(content) != int(manifest.get("size_bytes") or -1)
        or content_digest != expected_digest
        or listed_manifest.get("sha256") != expected_digest
    ):
        raise VerificationError("artifact bytes do not match the immutable task manifest")

    source_commit = command_stdout(["git", "rev-parse", "HEAD"])
    report = {
        "schema_version": 1,
        "status": "pass",
        "verification_mode": "existing_task_read_only_no_new_message",
        "source_commit": source_commit,
        "source_commit_pushed": (
            source_commit is not None
            and source_commit == command_stdout(["git", "rev-parse", "origin/main"])
        ),
        "task": {
            "task_id": task_id,
            "channel": "wechat",
            "status": "succeeded",
        },
        "immutable_manifest": {
            "artifact_id": manifest["id"],
            "kind": manifest["kind"],
            "mime_type": manifest["mime_type"],
            "filename": manifest["filename"],
            "size_bytes": len(content),
            "sha256": content_digest,
            "local_path": repo_path(local_path),
            "local_bytes_match": True,
            "authenticated_download_match": True,
        },
        "ui": {
            "manifest_api": f"/v1/tasks/{task_id}/artifacts",
            "download_url": manifest["download_url"],
            "preview_url": manifest["preview_url"],
            "current_api_listed": True,
            "current_download_verified": True,
        },
        "communication_delivery": log_evidence,
        "redaction": {
            "status": "pass",
            "credentials_embedded": False,
            "external_user_ids_embedded": False,
            "message_content_embedded": False,
            "new_message_sent": False,
            "scanner": "scripts/nl_tests/secret_scan.py",
        },
    }
    sys.path.insert(0, str(ROOT / "scripts" / "nl_tests"))
    from secret_scan import secret_scan_findings

    findings = secret_scan_findings(report)
    if findings:
        raise VerificationError(f"report secret scan failed with {len(findings)} finding(s)")
    return report


def main(argv: list[str] | None = None) -> int:
    args = parse_args(argv)
    try:
        report = verify(args)
        output = args.output.expanduser().resolve()
        output.parent.mkdir(parents=True, exist_ok=True)
        output.write_text(
            json.dumps(report, ensure_ascii=False, indent=2, sort_keys=True) + "\n",
            encoding="utf-8",
        )
        print(
            json.dumps(
                {
                    "status": report["status"],
                    "task_id": report["task"]["task_id"],
                    "artifact_id": report["immutable_manifest"]["artifact_id"],
                    "source_commit": report["source_commit"],
                    "output": repo_path(output),
                    "new_message_sent": False,
                },
                ensure_ascii=False,
                sort_keys=True,
            )
        )
        return 0
    except (OSError, sqlite3.Error, VerificationError, json.JSONDecodeError) as error:
        print(f"WECHAT_ARTIFACT_VERIFICATION_FAIL {error}", file=sys.stderr)
        return 1


if __name__ == "__main__":
    raise SystemExit(main())
