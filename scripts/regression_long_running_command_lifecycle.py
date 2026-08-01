#!/usr/bin/env python3
"""Run real clawd command-lifecycle regressions against an isolated workspace."""

from __future__ import annotations

import argparse
from datetime import datetime, timezone
import hashlib
import json
import os
from pathlib import Path
import platform
import re
import shutil
import signal
import socket
import subprocess
import sys
import tempfile
import time
from typing import Any
from urllib.error import HTTPError
from urllib.request import Request, urlopen


ROOT = Path(__file__).resolve().parents[1]
CLAWD_BIN = Path(os.environ.get("CLAWD_BIN", ROOT / "target/debug/clawd"))
AUTO_BUILD = os.environ.get("AUTO_BUILD", "1") == "1"
SUBMIT_VIA_NL = os.environ.get("SUBMIT_VIA_NL", "0") == "1"
WAIT_SECONDS = int(os.environ.get("WAIT_SECONDS", "180"))
POLL_SECONDS = float(os.environ.get("POLL_SECONDS", "1"))
LOG_DIR = Path(
    os.environ.get(
        "LOG_DIR",
        ROOT / "target" / f"long_running_command_lifecycle_{time.strftime('%Y%m%d_%H%M%S')}",
    )
)
TERMINAL = {"succeeded", "failed", "timeout", "canceled"}
NORMAL_CASE_IDS = (
    "heartbeat_70s_crosses_poll_windows",
    "concurrent_health_while_long_command_runs",
    "silent_35s_survives_5s_poll_and_idle_hint",
    "concurrent_stdout_stderr_large_utf8_artifacts",
    "explicit_5s_deadline_stops_90s_command",
    "cancel_is_idempotent_and_removes_process_group",
)
SLOW_CASE_ID = "durable_3705s_has_no_implicit_runtime_deadline"
CASE_IDS = (*NORMAL_CASE_IDS, SLOW_CASE_ID)
SLOW_DURATION_SECONDS = 3_705
SLOW_RETENTION_SECONDS = 7_800
SLOW_POLL_SECONDS = 15
SENSITIVE_ENV_MARKERS = ("API_KEY", "TOKEN", "SECRET", "PASSWORD", "COOKIE", "AUTHORIZATION")


class RegressionFailure(RuntimeError):
    pass


def parse_args(argv: list[str] | None = None) -> argparse.Namespace:
    parser = argparse.ArgumentParser(
        description="Run deterministic long-running command lifecycle cases against an isolated clawd.",
    )
    parser.add_argument(
        "--list-cases",
        action="store_true",
        help="List deterministic case identifiers without building or starting clawd.",
    )
    parser.add_argument(
        "--no-build",
        action="store_true",
        help="Use the selected existing clawd binary without building it first.",
    )
    parser.add_argument(
        "--binary",
        type=Path,
        default=CLAWD_BIN,
        help=f"clawd binary to execute (default: {CLAWD_BIN}).",
    )
    parser.add_argument(
        "--log-dir",
        type=Path,
        default=LOG_DIR,
        help=f"evidence directory (default: {LOG_DIR}).",
    )
    slow_group = parser.add_mutually_exclusive_group()
    slow_group.add_argument(
        "--include-slow",
        action="store_true",
        help="Run the opt-in 3705-second no-implicit-deadline case after the normal matrix.",
    )
    slow_group.add_argument(
        "--slow-only",
        action="store_true",
        help="Run only the opt-in 3705-second no-implicit-deadline release acceptance case.",
    )
    return parser.parse_args(argv)


def configure_from_args(args: argparse.Namespace) -> None:
    global AUTO_BUILD, CLAWD_BIN, LOG_DIR
    CLAWD_BIN = args.binary.expanduser().resolve()
    LOG_DIR = args.log_dir.expanduser().resolve()
    if args.no_build:
        AUTO_BUILD = False


def utc_now() -> str:
    return datetime.now(timezone.utc).isoformat().replace("+00:00", "Z")


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


def repo_path_ref(path: Path) -> str:
    return os.path.relpath(path.resolve(), ROOT)


def sha256_file(path: Path) -> str | None:
    if not path.is_file():
        return None
    digest = hashlib.sha256()
    with path.open("rb") as handle:
        for chunk in iter(lambda: handle.read(1024 * 1024), b""):
            digest.update(chunk)
    return digest.hexdigest()


def sha256_tree(path: Path) -> str | None:
    if not path.is_dir():
        return None
    digest = hashlib.sha256()
    files = sorted(candidate for candidate in path.rglob("*") if candidate.is_file())
    for candidate in files:
        digest.update(candidate.relative_to(path).as_posix().encode())
        digest.update(b"\0")
        file_digest = sha256_file(candidate)
        if file_digest is not None:
            digest.update(file_digest.encode())
        digest.update(b"\0")
    return digest.hexdigest()


def worktree_summary() -> dict[str, Any]:
    raw = command_stdout(["git", "status", "--porcelain", "--untracked-files=all"]) or ""
    changes = [line for line in raw.splitlines() if line.strip()]
    return {
        "status": "clean" if not changes else "dirty",
        "changed_path_count": len(changes),
    }


def evidence_paths() -> list[str]:
    paths: list[str] = []
    for candidate in sorted(LOG_DIR.rglob("*")):
        if candidate.is_file():
            paths.append(candidate.relative_to(LOG_DIR).as_posix())
    if "summary.json" not in paths:
        paths.append("summary.json")
    return sorted(paths)


def sensitive_values(admin_key: str | None) -> list[str]:
    values = [admin_key] if admin_key else []
    for name, value in os.environ.items():
        if value and len(value) >= 12 and any(marker in name.upper() for marker in SENSITIVE_ENV_MARKERS):
            values.append(value)
    return sorted({value for value in values if value}, key=len, reverse=True)


def redact_text(text: str, values: list[str]) -> str:
    redacted = text
    for value in values:
        redacted = redacted.replace(value, "[REDACTED]")
    return redacted


def scan_evidence(values: list[str]) -> list[dict[str, str]]:
    sys.path.insert(0, str(ROOT / "scripts" / "nl_tests"))
    from secret_scan import SECRET_VALUE_PATTERNS, secret_scan_findings

    findings: list[dict[str, str]] = []
    for path in sorted(LOG_DIR.rglob("*")):
        if not path.is_file() or path.name == "summary.json":
            continue
        relative = path.relative_to(LOG_DIR).as_posix()
        try:
            text = path.read_text(encoding="utf-8")
        except (OSError, UnicodeDecodeError):
            continue
        if any(value in text for value in values):
            findings.append({"path": relative, "kind": "known_secret_value"})
        if path.suffix == ".json":
            try:
                payload = json.loads(text)
            except json.JSONDecodeError:
                payload = None
            if payload is not None and secret_scan_findings(payload):
                findings.append({"path": relative, "kind": "structured_secret_contract"})
        for kind, pattern in SECRET_VALUE_PATTERNS:
            if pattern.search(text):
                findings.append({"path": relative, "kind": f"secret_like_value:{kind}"})
                break
    return findings


def finalize_summary(
    summary: dict[str, Any],
    started_at: str,
    admin_key: str | None,
) -> bool:
    values = sensitive_values(admin_key)
    findings = scan_evidence(values)
    if findings:
        summary["status"] = "fail"
        summary["error_code"] = "evidence_secret_scan_failed"
        summary["secret_scan_findings"] = findings
    cases = summary.get("cases") if isinstance(summary.get("cases"), dict) else {}
    passed = sum(
        1
        for result in cases.values()
        if isinstance(result, dict) and result.get("status") == "pass"
    )
    recorded_failed = len(cases) - passed
    unrecorded_failed = 1 if summary.get("status") != "pass" and recorded_failed == 0 else 0
    failed = recorded_failed + unrecorded_failed
    summary.update(
        {
            "schema_version": 1,
            "source_commit": summary.get("source_commit")
            or command_stdout(["git", "rev-parse", "HEAD"]),
            "source_commit_pushed": summary.get("source_commit_pushed")
            if isinstance(summary.get("source_commit_pushed"), bool)
            else (
                command_stdout(["git", "rev-parse", "HEAD"])
                == command_stdout(["git", "rev-parse", "origin/main"])
            ),
            "worktree": summary.get("worktree") or worktree_summary(),
            "finalization_worktree": worktree_summary(),
            "platform": platform.system().lower(),
            "arch": platform.machine().lower(),
            "binary": summary.get("binary")
            or {
                "path": repo_path_ref(CLAWD_BIN),
                "sha256": sha256_file(CLAWD_BIN),
            },
            "ui": summary.get("ui")
            or {
                "path": "UI/dist",
                "tree_sha256": sha256_tree(ROOT / "UI" / "dist"),
                "digest_algorithm": "sha256(relative_path_nul_file_sha256_nul)",
            },
            "case_counts": {
                "total": passed + failed,
                "passed": passed,
                "failed": failed,
                "unrecorded_failed": unrecorded_failed,
            },
            "started_at": started_at,
            "finished_at": utc_now(),
            "build_strategy": {
                "auto_build": AUTO_BUILD,
                "cargo_environment": (
                    "scripts/shell_compat.sh:configure_cargo_build_environment"
                    if AUTO_BUILD
                    else "existing_binary"
                ),
            },
            "evidence_root": repo_path_ref(LOG_DIR),
            "evidence_relative_paths": evidence_paths(),
            "redaction": {
                "status": "fail" if findings else "pass",
                "admin_key_recorded": False,
                "provider_credentials_recorded": False,
                "cookies_recorded": False,
                "command_secrets_allowed": False,
                "scanner": "scripts/nl_tests/secret_scan.py",
            },
        }
    )
    if isinstance(summary.get("error"), str):
        summary["error"] = redact_text(summary["error"], values)
    write_json(LOG_DIR / "summary.json", summary)
    return not findings


def write_json(path: Path, value: Any) -> None:
    path.parent.mkdir(parents=True, exist_ok=True)
    path.write_text(json.dumps(value, ensure_ascii=False, indent=2), encoding="utf-8")


def free_port() -> int:
    with socket.socket() as sock:
        sock.bind(("127.0.0.1", 0))
        return int(sock.getsockname()[1])


def replace_once(text: str, pattern: str, replacement: str) -> str:
    updated, count = re.subn(pattern, replacement, text, count=1, flags=re.MULTILINE)
    if count != 1:
        raise RegressionFailure(f"failed to patch config pattern: {pattern}")
    return updated


def prepare_workspace() -> Path:
    workspace = Path(tempfile.mkdtemp(prefix="agent-runtime-long-command-"))
    shutil.copy2(ROOT / "Cargo.toml", workspace / "Cargo.toml")
    if (ROOT / "Cargo.lock").exists():
        shutil.copy2(ROOT / "Cargo.lock", workspace / "Cargo.lock")
    shutil.copytree(ROOT / "configs", workspace / "configs")
    shutil.copytree(ROOT / "prompts", workspace / "prompts")
    for directory in ("data", "document", "logs"):
        (workspace / directory).mkdir()
    for directory in ("crates", "scripts", "target"):
        (workspace / directory).symlink_to(ROOT / directory, target_is_directory=True)

    config_path = workspace / "configs/config.toml"
    text = config_path.read_text(encoding="utf-8")
    text = replace_once(text, r'^sqlite_path\s*=\s*".*"$', f'sqlite_path = "{workspace / "data/tasks.sqlite"}"')
    text = replace_once(text, r'^access_profile\s*=\s*".*"$', 'access_profile = "full"')
    text = replace_once(text, r'^poll_interval_ms\s*=\s*\d+$', "poll_interval_ms = 200")
    text = replace_once(text, r'^task_heartbeat_seconds\s*=\s*\d+$', "task_heartbeat_seconds = 5")
    config_path.write_text(text, encoding="utf-8")
    return workspace


class Server:
    def __init__(self, workspace: Path) -> None:
        self.workspace = workspace
        self.port = free_port()
        self.base_url = f"http://127.0.0.1:{self.port}"
        self.key = self._generate_admin_key()
        self.log_handle = (LOG_DIR / "clawd.log").open("wb")
        env = os.environ.copy()
        env.update(
            {
                "APP_INTERNAL_LISTEN": f"127.0.0.1:{self.port}",
                "WORKSPACE_ROOT": str(workspace),
            }
        )
        self.process = subprocess.Popen(
            [str(CLAWD_BIN)],
            cwd=workspace,
            env=env,
            stdout=self.log_handle,
            stderr=subprocess.STDOUT,
            start_new_session=True,
        )
        self._wait_for_health()

    def _generate_admin_key(self) -> str:
        env = os.environ.copy()
        env["APP_CONFIG_PATH"] = str(self.workspace / "configs/config.toml")
        completed = subprocess.run(
            ["bash", str(ROOT / "scripts/auth-key.sh"), "generate", "admin"],
            cwd=self.workspace,
            env=env,
            check=True,
            capture_output=True,
            text=True,
        )
        key = completed.stdout.split(maxsplit=1)[0].strip()
        if not key:
            raise RegressionFailure("isolated admin key generation returned no key")
        return key

    def close(self) -> None:
        if self.process.poll() is None:
            os.killpg(self.process.pid, signal.SIGTERM)
            try:
                self.process.wait(timeout=10)
            except subprocess.TimeoutExpired:
                os.killpg(self.process.pid, signal.SIGKILL)
                self.process.wait(timeout=10)
        self.log_handle.close()

    def request(self, method: str, path: str, body: Any | None = None) -> dict[str, Any]:
        data = None if body is None else json.dumps(body, ensure_ascii=False).encode()
        request = Request(
            self.base_url + path,
            data=data,
            method=method,
            headers={"X-Agent-Key": self.key, "Content-Type": "application/json"},
        )
        try:
            with urlopen(request, timeout=15) as response:
                return json.loads(response.read().decode())
        except HTTPError as error:
            payload = error.read().decode(errors="replace")
            raise RegressionFailure(f"{method} {path} returned HTTP {error.code}: {payload}") from error

    def request_bytes(self, path: str) -> bytes:
        request = Request(
            self.base_url + path,
            method="GET",
            headers={"X-Agent-Key": self.key},
        )
        try:
            with urlopen(request, timeout=15) as response:
                return response.read()
        except HTTPError as error:
            payload = error.read().decode(errors="replace")
            raise RegressionFailure(f"GET {path} returned HTTP {error.code}: {payload}") from error

    def _wait_for_health(self) -> None:
        deadline = time.monotonic() + 90
        while time.monotonic() < deadline:
            if self.process.poll() is not None:
                raise RegressionFailure(f"clawd exited before health, code={self.process.returncode}")
            try:
                if self.request("GET", "/v1/health").get("ok"):
                    return
            except (OSError, RegressionFailure):
                pass
            time.sleep(0.5)
        raise RegressionFailure("clawd health endpoint did not become ready")


def submit_skill(server: Server, skill_name: str, args: dict[str, Any], case: str) -> str:
    request = {
        "user_id": 2_147_200_001,
        "chat_id": 2_147_200_002,
        "channel": "ui",
        "kind": "run_skill",
        "payload": {"skill_name": skill_name, "args": args},
    }
    write_json(LOG_DIR / case / "submit_request.json", request)
    response = server.request("POST", "/v1/tasks", request)
    write_json(LOG_DIR / case / "submit_response.json", response)
    task_id = str((response.get("data") or {}).get("task_id") or "")
    if not response.get("ok") or not task_id:
        raise RegressionFailure(f"{case}: submit failed: {response}")
    return task_id


def submit_command(server: Server, args: dict[str, Any], case: str) -> str:
    if not SUBMIT_VIA_NL:
        return submit_skill(server, "run_cmd", args, case)
    prompt = (
        "Use the disclosed system.run_command capability exactly once to perform this real local "
        "asynchronous command. Do not dry-run, preview, replace, shorten, or restart it. Pass these "
        "arguments exactly, then let the runtime keep polling the same job/checkpoint to terminal: "
        + json.dumps(args, ensure_ascii=False, sort_keys=True)
    )
    request = {
        "user_id": 2_147_200_001,
        "chat_id": 2_147_200_002,
        "channel": "ui",
        "kind": "ask",
        "payload": {"text": prompt},
    }
    write_json(LOG_DIR / case / "submit_request.json", request)
    response = server.request("POST", "/v1/tasks", request)
    write_json(LOG_DIR / case / "submit_response.json", response)
    task_id = str((response.get("data") or {}).get("task_id") or "")
    if not response.get("ok") or not task_id:
        raise RegressionFailure(f"{case}: NL submit failed: {response}")
    return task_id


def query_task(server: Server, task_id: str) -> dict[str, Any]:
    return server.request("GET", f"/v1/tasks/{task_id}")


def approve_if_needed(server: Server, task: dict[str, Any], case: str) -> bool:
    result = ((task.get("data") or {}).get("result_json") or {})
    approval = ((result.get("resume_context") or {}).get("approval_request") or {})
    if approval.get("status") != "pending":
        return False
    request_id = str(approval.get("request_id") or "")
    if not request_id:
        raise RegressionFailure(f"{case}: approval request is missing request_id")
    response = server.request(
        "POST",
        "/v1/tasks/resume-by-task-id",
        {
            "task_id": (task.get("data") or {}).get("task_id"),
            "approval_request_id": request_id,
            "approval_decision": "approve_once",
        },
    )
    write_json(LOG_DIR / case / "approval_response.json", response)
    if not response.get("ok"):
        raise RegressionFailure(f"{case}: approval failed: {response}")
    return True


def pending_job(task: dict[str, Any]) -> dict[str, Any]:
    result = ((task.get("data") or {}).get("result_json") or {})
    journal = result.get("task_journal") or {}
    candidates = [
        result.get("task_checkpoint"),
        (journal.get("summary") or {}).get("task_checkpoint"),
        (journal.get("trace") or {}).get("task_checkpoint"),
        (result.get("resume_context") or {}).get("task_checkpoint"),
    ]
    for checkpoint in candidates:
        if not isinstance(checkpoint, dict):
            continue
        job = checkpoint.get("pending_async_job")
        if isinstance(job, dict) and job:
            return job
    return {}


def wait_for_checkpoint(server: Server, task_id: str, case: str) -> dict[str, Any]:
    deadline = time.monotonic() + WAIT_SECONDS
    approved = False
    while time.monotonic() < deadline:
        task = query_task(server, task_id)
        approved = approve_if_needed(server, task, case) or approved
        job = pending_job(task)
        if job.get("job_id") and job.get("cancel_ref"):
            write_json(LOG_DIR / case / "pending.json", task)
            return task
        status = str((task.get("data") or {}).get("status") or "")
        if status in TERMINAL:
            write_json(LOG_DIR / case / "terminal_before_checkpoint.json", task)
            raise RegressionFailure(f"{case}: terminal before checkpoint: {status}")
        time.sleep(POLL_SECONDS)
    raise RegressionFailure(f"{case}: no async checkpoint within {WAIT_SECONDS}s; approved={approved}")


def wait_for_terminal(
    server: Server,
    task_id: str,
    case: str,
    *,
    timeout: int = WAIT_SECONDS,
) -> tuple[dict[str, Any], int]:
    deadline = time.monotonic() + timeout
    checkpoint_observations = 0
    while time.monotonic() < deadline:
        task = query_task(server, task_id)
        approve_if_needed(server, task, case)
        if pending_job(task).get("job_id"):
            checkpoint_observations += 1
        status = str((task.get("data") or {}).get("status") or "")
        if status in TERMINAL:
            write_json(LOG_DIR / case / "final.json", task)
            return task, checkpoint_observations
        time.sleep(POLL_SECONDS)
    raise RegressionFailure(f"{case}: task did not become terminal within {timeout}s")


def serialized_result(task: dict[str, Any]) -> str:
    return json.dumps((task.get("data") or {}).get("result_json") or {}, ensure_ascii=False)


def nested_values(value: Any, key: str) -> list[Any]:
    found: list[Any] = []
    if isinstance(value, dict):
        for child_key, child in value.items():
            if child_key == key:
                found.append(child)
            found.extend(nested_values(child, key))
    elif isinstance(value, list):
        for child in value:
            found.extend(nested_values(child, key))
    return found


def maximum_integer_value(value: Any, key: str) -> int:
    candidates = [item for item in nested_values(value, key) if isinstance(item, int)]
    return max(candidates, default=0)


def assert_real_execution(case: str, task: dict[str, Any]) -> None:
    if '"dry_run": true' in serialized_result(task):
        raise RegressionFailure(f"{case}: non-X task returned dry_run=true")


def run_success_case(
    server: Server,
    case: str,
    command: str,
    marker: str,
    extra_args: dict[str, Any],
    *,
    min_checkpoint_observations: int,
) -> dict[str, Any]:
    args = {
        "action": "exec",
        "command": command,
        "async_start": True,
        "poll_after_seconds": 5,
        "expires_in_seconds": 240,
        **extra_args,
    }
    task_id = submit_command(server, args, case)
    wait_for_checkpoint(server, task_id, case)
    final, observations = wait_for_terminal(server, task_id, case)
    assert_real_execution(case, final)
    if (final.get("data") or {}).get("status") != "succeeded":
        raise RegressionFailure(f"{case}: unexpected status {(final.get('data') or {}).get('status')}")
    if marker not in serialized_result(final):
        raise RegressionFailure(f"{case}: command output marker missing: {marker}")
    if observations < min_checkpoint_observations:
        raise RegressionFailure(
            f"{case}: only {observations} checkpoint observations, expected {min_checkpoint_observations}"
        )
    return {"task_id": task_id, "checkpoint_observations": observations, "status": "pass"}


def run_health_concurrency(server: Server, long_task_id: str) -> dict[str, Any]:
    case = "concurrent_health_while_long_command_runs"
    health_started = time.monotonic()
    health_response = server.request("GET", "/v1/health")
    health_elapsed = time.monotonic() - health_started
    write_json(LOG_DIR / case / "health_response.json", health_response)
    if not health_response.get("ok"):
        raise RegressionFailure(f"{case}: health endpoint failed while long command was active")

    # Use fixed host builtins for the concurrent checks. Runner skills
    # intentionally require immutable package receipts, while this isolated
    # workspace contains no release receipts by design. list_dir returns
    # symlink_metadata-backed file properties, so it supplies the explicit
    # stat evidence without bypassing package admission.
    stat_case = f"{case}/stat_paths"
    stat_task_id = submit_skill(server, "list_dir", {"path": "."}, stat_case)
    stat_started = time.monotonic()
    stat_task, _ = wait_for_terminal(server, stat_task_id, stat_case, timeout=20)
    stat_elapsed = time.monotonic() - stat_started

    short_case = f"{case}/unrelated_read_file"
    short_task_id = submit_skill(
        server,
        "read_file",
        {"path": "Cargo.toml", "max_bytes": 256},
        short_case,
    )
    short_started = time.monotonic()
    short_task, _ = wait_for_terminal(server, short_task_id, short_case, timeout=20)
    short_elapsed = time.monotonic() - short_started

    long_during = query_task(server, long_task_id)
    write_json(LOG_DIR / case / "long_task_during_health.json", long_during)
    assert_real_execution(stat_case, stat_task)
    assert_real_execution(short_case, short_task)
    if (stat_task.get("data") or {}).get("status") != "succeeded":
        raise RegressionFailure(f"{case}: concurrent stat task did not succeed")
    if (short_task.get("data") or {}).get("status") != "succeeded":
        raise RegressionFailure(f"{case}: unrelated short task did not succeed")

    def has_cargo_file_stat(value: Any) -> bool:
        if isinstance(value, dict):
            if (
                value.get("name") == "Cargo.toml"
                and value.get("kind") == "file"
                and isinstance(value.get("size_bytes"), int)
                and value["size_bytes"] > 0
            ):
                return True
            return any(has_cargo_file_stat(child) for child in value.values())
        if isinstance(value, list):
            return any(has_cargo_file_stat(child) for child in value)
        return False

    stat_result = (stat_task.get("data") or {}).get("result_json") or {}
    if not has_cargo_file_stat(stat_result):
        raise RegressionFailure(f"{case}: concurrent stat task did not verify Cargo.toml")
    short_result = serialized_result(short_task)
    if '"source":"read_file"' not in short_result.replace(" ", "") or "[workspace]" not in short_result:
        raise RegressionFailure(f"{case}: unrelated read_file evidence missing")
    if (long_during.get("data") or {}).get("status") not in {"queued", "running"}:
        raise RegressionFailure(f"{case}: long command was not active when health completed")
    return {
        "stat_task_id": stat_task_id,
        "short_task_id": short_task_id,
        "health_elapsed_seconds": round(health_elapsed, 3),
        "stat_task_elapsed_seconds": round(stat_elapsed, 3),
        "short_task_elapsed_seconds": round(short_elapsed, 3),
        "status": "pass",
    }


def run_deadline_case(server: Server) -> dict[str, Any]:
    case = "explicit_5s_deadline_stops_90s_command"
    task_id = submit_command(
        server,
        {
            "action": "exec_with_deadline",
            "command": "sleep 90; printf 'UNEXPECTED_DEADLINE_MISS\\n'",
            "timeout_seconds": 5,
            "async_start": True,
            "poll_after_seconds": 1,
            "expires_in_seconds": 120,
        },
        case,
    )
    started = time.monotonic()
    wait_for_checkpoint(server, task_id, case)
    final, _ = wait_for_terminal(server, task_id, case, timeout=30)
    harness_elapsed = time.monotonic() - started
    assert_real_execution(case, final)
    result = (final.get("data") or {}).get("result_json") or {}
    failure = (
        ((result.get("task_lifecycle") or {}).get("resume_executor_result_projection") or {}).get(
            "failure_result_json"
        )
        or {}
    )
    if (final.get("data") or {}).get("status") != "failed":
        raise RegressionFailure(f"{case}: expected failed terminal task")
    if failure.get("terminal_reason") != "runtime_timeout":
        raise RegressionFailure(f"{case}: runtime_timeout reason missing")
    if failure.get("exit_code") not in (124, 125):
        raise RegressionFailure(f"{case}: portable timeout exit code missing")
    observed_output = "\n".join(str(failure.get(key) or "") for key in ("stdout", "stderr", "output"))
    if "UNEXPECTED_DEADLINE_MISS" in observed_output:
        raise RegressionFailure(f"{case}: command continued beyond deadline")
    process_started_at = failure.get("started_at", result.get("started_at"))
    process_finished_at = failure.get("finished_at", result.get("finished_at"))
    if isinstance(process_started_at, (int, float)) and isinstance(process_finished_at, (int, float)):
        process_elapsed = float(process_finished_at) - float(process_started_at)
    else:
        process_elapsed = harness_elapsed
    if process_elapsed < 0 or process_elapsed > 15:
        raise RegressionFailure(
            f"{case}: process deadline termination took {process_elapsed:.2f}s "
            f"(harness elapsed {harness_elapsed:.2f}s)"
        )
    return {
        "task_id": task_id,
        "process_elapsed_seconds": round(process_elapsed, 3),
        "harness_elapsed_seconds": round(harness_elapsed, 3),
        "status": "pass",
    }


def run_large_stream_artifact_case(server: Server) -> dict[str, Any]:
    case = "concurrent_stdout_stderr_large_utf8_artifacts"
    command = (
        "(i=1; while [ \"$i\" -le 2500 ]; do "
        "printf 'OUT_%04d_中文_🙂\\n' \"$i\"; i=$((i+1)); done) & out_pid=$!; "
        "(i=1; while [ \"$i\" -le 2500 ]; do "
        "printf 'ERR_%04d_错误_🚨\\n' \"$i\" >&2; i=$((i+1)); done) & err_pid=$!; "
        "wait \"$out_pid\"; wait \"$err_pid\""
    )
    task_id = submit_command(
        server,
        {
            "action": "exec",
            "command": command,
            "async_start": True,
            "poll_after_seconds": 1,
            "expires_in_seconds": 120,
        },
        case,
    )
    wait_for_checkpoint(server, task_id, case)
    final, _ = wait_for_terminal(server, task_id, case, timeout=30)
    assert_real_execution(case, final)
    if (final.get("data") or {}).get("status") != "succeeded":
        raise RegressionFailure(f"{case}: command did not succeed")
    result = (final.get("data") or {}).get("result_json") or {}
    if result.get("artifact_publish_status") != "published":
        raise RegressionFailure(f"{case}: artifact publication failed")
    if not result.get("output_truncated") or not result.get("truncated"):
        raise RegressionFailure(f"{case}: large output was not marked truncated")

    refs = {
        str((ref.get("metadata") or {}).get("stream") or ""): ref
        for ref in result.get("artifact_refs") or []
        if isinstance(ref, dict)
    }
    descriptors = {
        str(item.get("id") or ""): item
        for item in result.get("artifacts") or []
        if isinstance(item, dict)
    }
    ranges = {
        str(item.get("artifact_ref") or ""): item
        for item in result.get("range_handles") or []
        if isinstance(item, dict)
    }
    verification: dict[str, Any] = {}
    markers = {
        "stdout": ("OUT_0001_中文_🙂", "OUT_2500_中文_🙂"),
        "stderr": ("ERR_0001_错误_🚨", "ERR_2500_错误_🚨"),
    }
    for stream, (first_marker, last_marker) in markers.items():
        ref = refs.get(stream)
        if not ref:
            raise RegressionFailure(f"{case}: {stream} artifact ref missing")
        artifact_id = str(ref.get("id") or "")
        descriptor = descriptors.get(artifact_id) or {}
        range_handle = ranges.get(artifact_id) or {}
        download_url = str(descriptor.get("download_url") or "")
        if not download_url:
            raise RegressionFailure(f"{case}: {stream} immutable download URL missing")
        content = server.request_bytes(download_url)
        try:
            text = content.decode("utf-8")
        except UnicodeDecodeError as error:
            raise RegressionFailure(f"{case}: {stream} artifact is not valid UTF-8") from error
        expected_total = int(result.get(f"{stream}_total_bytes") or 0)
        preview_bytes = int(result.get(f"{stream}_preview_bytes") or 0)
        cursor = int(result.get(f"{stream}_cursor") or 0)
        if len(content) != expected_total or expected_total <= 32 * 1024:
            raise RegressionFailure(f"{case}: {stream} total byte contract mismatch")
        if cursor != preview_bytes or not (32 * 1024 <= preview_bytes <= 32 * 1024 + 3):
            raise RegressionFailure(f"{case}: {stream} preview cursor contract mismatch")
        if result.get(f"{stream}_encoding") != "utf-8" or "\ufffd" in str(
            result.get(stream) or ""
        ):
            raise RegressionFailure(f"{case}: {stream} preview split a UTF-8 scalar")
        if first_marker not in text or last_marker not in text:
            raise RegressionFailure(f"{case}: {stream} artifact markers missing")
        actual_sha256 = hashlib.sha256(content).hexdigest()
        if actual_sha256 != ref.get("sha256") or actual_sha256 != descriptor.get("sha256"):
            raise RegressionFailure(f"{case}: {stream} artifact digest mismatch")
        if range_handle.get("read_capability") != "artifact.read_range":
            raise RegressionFailure(f"{case}: {stream} range capability missing")
        if range_handle.get("start_byte") != 0 or range_handle.get("end_byte") != len(content):
            raise RegressionFailure(f"{case}: {stream} range bounds mismatch")
        verification[stream] = {
            "artifact_id": artifact_id,
            "size_bytes": len(content),
            "sha256": actual_sha256,
            "preview_bytes": preview_bytes,
            "encoding": result.get(f"{stream}_encoding"),
            "range_verified": True,
            "download_verified": True,
        }

    write_json(LOG_DIR / case / "artifact_verification.json", verification)
    return {
        "task_id": task_id,
        "streams": verification,
        "status": "pass",
    }


def collect_pids(value: Any) -> set[int]:
    found: set[int] = set()
    if isinstance(value, dict):
        for key, child in value.items():
            if key == "pid" and isinstance(child, int):
                found.add(child)
            found.update(collect_pids(child))
    elif isinstance(value, list):
        for child in value:
            found.update(collect_pids(child))
    return found


def run_cancel_case(server: Server) -> dict[str, Any]:
    case = "cancel_is_idempotent_and_removes_process_group"
    task_id = submit_command(
        server,
        {
            "action": "exec",
            "command": "sleep 90; printf 'UNEXPECTED_CANCEL_MISS\\n'",
            "async_start": True,
            "poll_after_seconds": 1,
            "expires_in_seconds": 120,
        },
        case,
    )
    wait_for_checkpoint(server, task_id, case)
    first = server.request("POST", "/v1/tasks/cancel-by-task-id", {"task_id": task_id})
    write_json(LOG_DIR / case / "cancel_first.json", first)
    final, _ = wait_for_terminal(server, task_id, case, timeout=30)
    second = server.request("POST", "/v1/tasks/cancel-by-task-id", {"task_id": task_id})
    write_json(LOG_DIR / case / "cancel_second.json", second)
    time.sleep(2)
    stable = query_task(server, task_id)
    write_json(LOG_DIR / case / "stable_terminal.json", stable)
    assert_real_execution(case, final)
    first_status = (first.get("data") or {}).get("status")
    second_status = (second.get("data") or {}).get("status")
    if not first.get("ok") or first_status != "task_cancelled":
        raise RegressionFailure(f"{case}: first cancel failed: {first_status}")
    if not second.get("ok") or second_status != "task_already_cancelled":
        raise RegressionFailure(f"{case}: second cancel was not idempotent: {second_status}")
    if (final.get("data") or {}).get("status") != "canceled":
        raise RegressionFailure(f"{case}: task did not become canceled")
    if (stable.get("data") or {}).get("status") != "canceled":
        raise RegressionFailure(f"{case}: terminal state regressed after cancellation")
    cancel_result = (((final.get("data") or {}).get("result_json") or {}).get("cancel_adapter_result") or {})
    if cancel_result.get("adapter_kind") != "local_process_poll":
        raise RegressionFailure(f"{case}: local cancel adapter result missing")
    alive: list[int] = []
    for pid in collect_pids(cancel_result):
        try:
            os.kill(pid, 0)
        except ProcessLookupError:
            continue
        except PermissionError:
            alive.append(pid)
        else:
            alive.append(pid)
    if alive:
        raise RegressionFailure(f"{case}: processes remain alive: {alive}")
    return {"task_id": task_id, "cancelled_pids": sorted(collect_pids(cancel_result)), "status": "pass"}


def slow_case_args() -> dict[str, Any]:
    return {
        "action": "exec",
        "command": (
            "printf 'APP_DURABLE_3705_START\\n'; "
            f"sleep {SLOW_DURATION_SECONDS}; "
            "printf 'APP_DURABLE_3705_DONE\\n'"
        ),
        "async_start": True,
        "poll_after_seconds": SLOW_POLL_SECONDS,
        "expires_in_seconds": SLOW_RETENTION_SECONDS,
    }


def run_no_implicit_deadline_slow_case(server: Server) -> dict[str, Any]:
    case = SLOW_CASE_ID
    start_marker = "APP_DURABLE_3705_START"
    finish_marker = "APP_DURABLE_3705_DONE"
    task_id = submit_command(server, slow_case_args(), case)
    started_at = utc_now()
    started_monotonic = time.monotonic()
    pending = wait_for_checkpoint(server, task_id, case)
    initial_job = pending_job(pending)
    job_id = str(initial_job.get("job_id") or "")
    if not job_id:
        raise RegressionFailure(f"{case}: checkpoint did not expose a job_id")
    runtime_deadlines = nested_values(pending, "runtime_deadline_at")
    if not runtime_deadlines or any(value is not None for value in runtime_deadlines):
        raise RegressionFailure(
            f"{case}: durable checkpoint has an implicit runtime deadline: {runtime_deadlines}"
        )
    retention_deadline_at = initial_job.get("retention_deadline_at")
    if not isinstance(retention_deadline_at, int):
        raise RegressionFailure(f"{case}: retention deadline is missing")

    observations: list[dict[str, Any]] = []
    checkpoint_observations = 0
    crossed_3600 = False
    next_sample_at = 0.0
    timeout_at = started_monotonic + SLOW_DURATION_SECONDS + 600
    final: dict[str, Any] | None = None
    while time.monotonic() < timeout_at:
        task = query_task(server, task_id)
        approve_if_needed(server, task, case)
        elapsed = time.monotonic() - started_monotonic
        current_job = pending_job(task)
        current_job_id = str(current_job.get("job_id") or "")
        if current_job_id:
            checkpoint_observations += 1
            if current_job_id != job_id:
                raise RegressionFailure(
                    f"{case}: job identity changed from {job_id} to {current_job_id}"
                )
        observed_deadlines = nested_values(task, "runtime_deadline_at")
        if any(value is not None for value in observed_deadlines):
            raise RegressionFailure(
                f"{case}: runtime deadline appeared after handoff: {observed_deadlines}"
            )
        status = str((task.get("data") or {}).get("status") or "")
        if elapsed >= 3_600 and status not in TERMINAL:
            crossed_3600 = True
        if elapsed >= next_sample_at or status in TERMINAL:
            observations.append(
                {
                    "observed_at": utc_now(),
                    "elapsed_seconds": round(elapsed, 3),
                    "task_status": status,
                    "execution_state": (task.get("data") or {}).get("execution_state"),
                    "job_id": current_job_id or job_id,
                    "stdout_cursor": maximum_integer_value(task, "stdout_cursor"),
                    "stderr_cursor": maximum_integer_value(task, "stderr_cursor"),
                    "runtime_deadline_at": None,
                }
            )
            write_json(LOG_DIR / case / "observations.json", observations)
            next_sample_at = elapsed + 300
            print(
                f"[INFO] {case} elapsed={elapsed:.0f}s status={status} "
                f"cursor={observations[-1]['stdout_cursor']}",
                flush=True,
            )
        if status in TERMINAL:
            final = task
            break
        time.sleep(SLOW_POLL_SECONDS)
    if final is None:
        raise RegressionFailure(f"{case}: task did not become terminal after the acceptance window")

    write_json(LOG_DIR / case / "final.json", final)
    assert_real_execution(case, final)
    final_data = final.get("data") or {}
    result = final_data.get("result_json") or {}
    wall_elapsed = time.monotonic() - started_monotonic
    if final_data.get("status") != "succeeded":
        raise RegressionFailure(f"{case}: unexpected terminal status {final_data.get('status')}")
    if wall_elapsed < 3_700 or not crossed_3600:
        raise RegressionFailure(
            f"{case}: fixture did not prove the >3600s boundary, elapsed={wall_elapsed:.3f}"
        )
    if start_marker not in serialized_result(final) or finish_marker not in serialized_result(final):
        raise RegressionFailure(f"{case}: start or finish marker is missing")
    if result.get("job_id") != job_id:
        raise RegressionFailure(f"{case}: terminal result changed the job_id")
    final_runtime_deadlines = nested_values(final, "runtime_deadline_at")
    if any(value is not None for value in final_runtime_deadlines):
        raise RegressionFailure(
            f"{case}: terminal result contains an implicit deadline: {final_runtime_deadlines}"
        )
    runtime_timeout_seconds = [
        value for value in nested_values(final, "runtime_timeout_seconds") if value is not None
    ]
    if runtime_timeout_seconds:
        raise RegressionFailure(
            f"{case}: terminal result contains an implicit runtime timeout: {runtime_timeout_seconds}"
        )
    process_started_at = result.get("started_at")
    process_finished_at = result.get("finished_at")
    process_elapsed = None
    if isinstance(process_started_at, (int, float)) and isinstance(process_finished_at, (int, float)):
        process_elapsed = float(process_finished_at) - float(process_started_at)
        if process_elapsed < 3_700:
            raise RegressionFailure(
                f"{case}: process elapsed time did not cross 3700s: {process_elapsed:.3f}"
            )

    return {
        "task_id": task_id,
        "job_id": job_id,
        "started_at": started_at,
        "finished_at": utc_now(),
        "wall_elapsed_seconds": round(wall_elapsed, 3),
        "process_elapsed_seconds": round(process_elapsed, 3) if process_elapsed is not None else None,
        "checkpoint_observations": checkpoint_observations,
        "observation_samples": len(observations),
        "stdout_cursor": maximum_integer_value(final, "stdout_cursor"),
        "stderr_cursor": maximum_integer_value(final, "stderr_cursor"),
        "runtime_deadline_at": None,
        "retention_deadline_at": retention_deadline_at,
        "crossed_3600_seconds_while_running": crossed_3600,
        "terminal_reason": result.get("terminal_reason"),
        "status": "pass",
    }


def ensure_binary() -> None:
    if AUTO_BUILD:
        subprocess.run(
            ["bash", "-lc", "source scripts/shell_compat.sh; configure_cargo_build_environment; cargo build -p clawd"],
            cwd=ROOT,
            check=True,
        )
    if not CLAWD_BIN.is_file() or not os.access(CLAWD_BIN, os.X_OK):
        raise RegressionFailure(f"clawd binary missing or not executable: {CLAWD_BIN}")


def main(argv: list[str] | None = None) -> int:
    args = parse_args(argv)
    if args.list_cases:
        for case_id in CASE_IDS:
            print(case_id)
        return 0
    configure_from_args(args)
    started_at = utc_now()
    LOG_DIR.mkdir(parents=True, exist_ok=True)
    workspace: Path | None = None
    server: Server | None = None
    exit_code = 1
    summary: dict[str, Any] = {
        "status": "fail",
        "submission_mode": "natural_language" if SUBMIT_VIA_NL else "direct_run_skill",
        "source_commit": command_stdout(["git", "rev-parse", "HEAD"]),
        "source_commit_pushed": (
            command_stdout(["git", "rev-parse", "HEAD"])
            == command_stdout(["git", "rev-parse", "origin/main"])
        ),
        "worktree": worktree_summary(),
        "selected_cases": (
            [SLOW_CASE_ID]
            if args.slow_only
            else [*NORMAL_CASE_IDS, *([SLOW_CASE_ID] if args.include_slow else [])]
        ),
        "cases": {},
    }
    try:
        ensure_binary()
        summary["binary"] = {
            "path": repo_path_ref(CLAWD_BIN),
            "sha256": sha256_file(CLAWD_BIN),
        }
        summary["ui"] = {
            "path": "UI/dist",
            "tree_sha256": sha256_tree(ROOT / "UI" / "dist"),
            "digest_algorithm": "sha256(relative_path_nul_file_sha256_nul)",
        }
        workspace = prepare_workspace()
        server = Server(workspace)
        print(
            f"[INFO] isolated clawd={server.base_url} evidence_root={repo_path_ref(LOG_DIR)}",
            flush=True,
        )

        if not args.slow_only:
            heartbeat_case = "heartbeat_70s_crosses_poll_windows"
            heartbeat_task_id = submit_command(
                server,
                {
                    "action": "exec",
                    "command": (
                        "i=1; while [ \"$i\" -le 7 ]; do "
                        "printf 'APP_LONG_HEARTBEAT_%s\\n' \"$i\"; sleep 10; i=$((i+1)); "
                        "done; printf 'APP_LONG_HEARTBEAT_DONE\\n'"
                    ),
                    "async_start": True,
                    "poll_after_seconds": 5,
                    "expires_in_seconds": 240,
                },
                heartbeat_case,
            )
            wait_for_checkpoint(server, heartbeat_task_id, heartbeat_case)
            summary["cases"]["concurrent_health"] = run_health_concurrency(
                server, heartbeat_task_id
            )
            heartbeat_final, heartbeat_observations = wait_for_terminal(
                server, heartbeat_task_id, heartbeat_case, timeout=150
            )
            assert_real_execution(heartbeat_case, heartbeat_final)
            if (heartbeat_final.get("data") or {}).get("status") != "succeeded":
                raise RegressionFailure(f"{heartbeat_case}: task did not succeed")
            if "APP_LONG_HEARTBEAT_DONE" not in serialized_result(heartbeat_final):
                raise RegressionFailure(f"{heartbeat_case}: final marker missing")
            if heartbeat_observations < 5:
                raise RegressionFailure(
                    f"{heartbeat_case}: only {heartbeat_observations} checkpoint observations"
                )
            summary["cases"]["heartbeat_70s"] = {
                "task_id": heartbeat_task_id,
                "checkpoint_observations": heartbeat_observations,
                "status": "pass",
            }
            print("[PASS] 70s heartbeat and concurrent health", flush=True)

            summary["cases"]["silent_35s"] = run_success_case(
                server,
                "silent_35s_survives_5s_poll_and_idle_hint",
                "sleep 35; printf 'APP_LONG_SILENT_DONE\\n'",
                "APP_LONG_SILENT_DONE",
                {"idle_timeout_seconds": 5},
                min_checkpoint_observations=4,
            )
            print("[PASS] 35s silent command", flush=True)

            summary["cases"]["large_stream_artifacts"] = run_large_stream_artifact_case(server)
            print("[PASS] concurrent large UTF-8 stream artifacts", flush=True)

            summary["cases"]["deadline_5s"] = run_deadline_case(server)
            print("[PASS] explicit 5s runtime deadline", flush=True)

            summary["cases"]["cancel_idempotent"] = run_cancel_case(server)
            print("[PASS] cancel and repeated cancel", flush=True)

        if args.include_slow or args.slow_only:
            summary["cases"][SLOW_CASE_ID] = run_no_implicit_deadline_slow_case(server)
            print("[PASS] 3705s durable command without implicit runtime deadline", flush=True)

        summary["status"] = "pass"
        exit_code = 0
    except Exception as error:
        summary["error"] = str(error)
        print(f"[FAIL] {error}", file=sys.stderr, flush=True)
    finally:
        if server is not None:
            server.close()
        if workspace is not None:
            model_log = workspace / "logs/model_io.log"
            if model_log.is_file():
                shutil.copy2(model_log, LOG_DIR / "model_io.log")
            if os.environ.get("KEEP_WORKSPACE", "0") == "1":
                print(f"[INFO] retained isolated workspace: {workspace}", flush=True)
            else:
                shutil.rmtree(workspace)
        admin_key = server.key if server is not None else None
        if not finalize_summary(summary, started_at, admin_key):
            exit_code = 1
    print(json.dumps(summary, ensure_ascii=False), flush=True)
    return exit_code


if __name__ == "__main__":
    raise SystemExit(main())
