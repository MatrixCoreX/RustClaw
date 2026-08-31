#!/usr/bin/env python3
"""Exercise start, poll, and cancel continuity across real clawd restarts."""

from __future__ import annotations

import argparse
from datetime import datetime, timezone
import json
import os
from pathlib import Path
import shutil
import signal
import sqlite3
import subprocess
import sys
import time
from typing import Any
from urllib.error import HTTPError
from urllib.request import Request, urlopen

import regression_long_running_command_lifecycle as lifecycle


ROOT = Path(__file__).resolve().parents[1]
CASE_IDS = ("start_boundary", "poll_boundary", "cancel_boundary")
TERMINAL = {"succeeded", "failed", "timeout", "canceled"}


class RestartBoundaryFailure(RuntimeError):
    pass


def parse_args(argv: list[str] | None = None) -> argparse.Namespace:
    parser = argparse.ArgumentParser(
        description="Run isolated clawd restart continuity at start, poll, and cancel boundaries.",
    )
    parser.add_argument(
        "--list-cases",
        action="store_true",
        help="List restart-boundary case identifiers without building or starting clawd.",
    )
    parser.add_argument(
        "--no-build",
        action="store_true",
        help="Use the selected existing clawd binary without building it first.",
    )
    parser.add_argument(
        "--binary",
        type=Path,
        default=Path(os.environ.get("CLAWD_BIN", ROOT / "target/debug/clawd")),
        help="clawd binary to execute.",
    )
    parser.add_argument(
        "--log-dir",
        type=Path,
        default=ROOT
        / "target"
        / f"clawd_restart_boundaries_{time.strftime('%Y%m%d_%H%M%S')}",
        help="evidence directory.",
    )
    parser.add_argument(
        "--wait-seconds",
        type=int,
        default=90,
        help="maximum wait per lifecycle boundary.",
    )
    return parser.parse_args(argv)


def utc_now() -> str:
    return datetime.now(timezone.utc).isoformat().replace("+00:00", "Z")


def write_json(path: Path, value: Any) -> None:
    path.parent.mkdir(parents=True, exist_ok=True)
    path.write_text(json.dumps(value, ensure_ascii=False, indent=2), encoding="utf-8")


def job_dir_from_ref(job_ref: str) -> Path:
    prefix = "local_process:"
    if not job_ref.startswith(prefix):
        raise RestartBoundaryFailure("restart checkpoint is not a local process job")
    raw = job_ref[len(prefix) :].strip()
    if not raw:
        raise RestartBoundaryFailure("restart checkpoint local process path is empty")
    return Path(raw)


def process_group_alive(process_group_id: int) -> bool:
    try:
        os.killpg(process_group_id, 0)
    except ProcessLookupError:
        return False
    except PermissionError:
        return True
    return True


class IsolatedRuntime:
    def __init__(self, binary: Path, log_root: Path, wait_seconds: int) -> None:
        self.binary = binary
        self.log_root = log_root
        self.wait_seconds = wait_seconds
        self.workspace = lifecycle.prepare_workspace()
        self.port = lifecycle.free_port()
        self.base_url = f"http://127.0.0.1:{self.port}"
        self.key = self._generate_admin_key()
        self.process: subprocess.Popen[bytes] | None = None
        self.log_handle: Any | None = None

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
            raise RestartBoundaryFailure("isolated admin key generation returned no key")
        return key

    def start(self, label: str) -> None:
        if self.process is not None:
            raise RestartBoundaryFailure("isolated clawd is already running")
        log_path = self.log_root / f"clawd_{label}.log"
        log_path.parent.mkdir(parents=True, exist_ok=True)
        self.log_handle = log_path.open("wb")
        env = os.environ.copy()
        env.update(
            {
                "APP_INTERNAL_LISTEN": f"127.0.0.1:{self.port}",
                "WORKSPACE_ROOT": str(self.workspace),
            }
        )
        self.process = subprocess.Popen(
            [str(self.binary)],
            cwd=self.workspace,
            env=env,
            stdout=self.log_handle,
            stderr=subprocess.STDOUT,
            start_new_session=True,
        )
        self.wait_for_health()

    def stop(self) -> None:
        process = self.process
        if process is not None and process.poll() is None:
            os.killpg(process.pid, signal.SIGTERM)
            try:
                process.wait(timeout=10)
            except subprocess.TimeoutExpired:
                os.killpg(process.pid, signal.SIGKILL)
                process.wait(timeout=10)
        self.process = None
        if self.log_handle is not None:
            self.log_handle.close()
            self.log_handle = None

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
            raise RestartBoundaryFailure(
                f"{method} {path} returned HTTP {error.code}: {payload}"
            ) from error

    def wait_for_health(self) -> None:
        deadline = time.monotonic() + self.wait_seconds
        while time.monotonic() < deadline:
            if self.process is None or self.process.poll() is not None:
                code = None if self.process is None else self.process.returncode
                raise RestartBoundaryFailure(
                    f"clawd exited before becoming healthy, code={code}"
                )
            try:
                if self.request("GET", "/v1/health").get("ok"):
                    return
            except (OSError, RestartBoundaryFailure):
                pass
            time.sleep(0.25)
        raise RestartBoundaryFailure("clawd health endpoint did not become ready")

    def cleanup(self) -> None:
        self.stop()
        async_jobs = self.workspace / ".agent-runtime" / "async_jobs"
        if async_jobs.is_dir():
            for job_dir in async_jobs.iterdir():
                if not job_dir.is_dir():
                    continue
                try:
                    pid = int((job_dir / "pid").read_text(encoding="utf-8").strip())
                except (OSError, ValueError):
                    continue
                if process_group_alive(pid):
                    try:
                        os.killpg(pid, signal.SIGKILL)
                    except (ProcessLookupError, PermissionError):
                        pass
        shutil.rmtree(self.workspace)


def submit_command(
    runtime: IsolatedRuntime,
    case_dir: Path,
    command: str,
    *,
    sleep_seconds: int,
) -> str:
    request = {
        "user_id": 2_147_310_001,
        "chat_id": 2_147_310_002,
        "channel": "ui",
        "kind": "run_skill",
        "payload": {
            "skill_name": "run_cmd",
            "args": {
                "action": "exec",
                "command": command,
                "async_start": True,
                "poll_after_seconds": 1,
                "expires_in_seconds": max(120, sleep_seconds + 60),
            },
        },
    }
    write_json(case_dir / "submit_request.json", request)
    response = runtime.request("POST", "/v1/tasks", request)
    write_json(case_dir / "submit_response.json", response)
    task_id = str((response.get("data") or {}).get("task_id") or "")
    if not response.get("ok") or not task_id:
        raise RestartBoundaryFailure(f"task submission failed: {response}")
    return task_id


def query_task(runtime: IsolatedRuntime, task_id: str) -> dict[str, Any]:
    return runtime.request("GET", f"/v1/tasks/{task_id}")


def approve_if_needed(
    runtime: IsolatedRuntime,
    task: dict[str, Any],
    case_dir: Path,
) -> bool:
    result = ((task.get("data") or {}).get("result_json") or {})
    approval = ((result.get("resume_context") or {}).get("approval_request") or {})
    if approval.get("status") != "pending":
        return False
    request_id = str(approval.get("request_id") or "")
    if not request_id:
        raise RestartBoundaryFailure("approval request is missing request_id")
    response = runtime.request(
        "POST",
        "/v1/tasks/resume-by-task-id",
        {
            "task_id": (task.get("data") or {}).get("task_id"),
            "approval_request_id": request_id,
            "approval_decision": "approve_once",
            "idempotency_key": f"restart-boundary-approval-{request_id}",
        },
    )
    write_json(case_dir / "approval_response.json", response)
    if not response.get("ok"):
        raise RestartBoundaryFailure(f"approval failed: {response}")
    return True


def checkpoint_from_task(task: dict[str, Any]) -> dict[str, Any]:
    result = ((task.get("data") or {}).get("result_json") or {})
    journal = result.get("task_journal") or {}
    candidates = (
        result.get("task_checkpoint"),
        (journal.get("summary") or {}).get("task_checkpoint"),
        (journal.get("trace") or {}).get("task_checkpoint"),
        (result.get("resume_context") or {}).get("task_checkpoint"),
    )
    for checkpoint in candidates:
        if not isinstance(checkpoint, dict):
            continue
        job = checkpoint.get("pending_async_job")
        if isinstance(job, dict) and job.get("job_id"):
            return checkpoint
    return {}


def wait_for_checkpoint(
    runtime: IsolatedRuntime,
    task_id: str,
    case_dir: Path,
) -> tuple[dict[str, Any], dict[str, Any]]:
    deadline = time.monotonic() + runtime.wait_seconds
    while time.monotonic() < deadline:
        task = query_task(runtime, task_id)
        approve_if_needed(runtime, task, case_dir)
        checkpoint = checkpoint_from_task(task)
        if checkpoint:
            write_json(case_dir / "pre_restart_task.json", task)
            return task, checkpoint
        status = str((task.get("data") or {}).get("status") or "")
        if status in TERMINAL:
            raise RestartBoundaryFailure(
                f"task became {status} before publishing a checkpoint"
            )
        time.sleep(0.25)
    raise RestartBoundaryFailure("task did not publish an async checkpoint")


def wait_for_job_start(
    runtime: IsolatedRuntime,
    task_id: str,
    case_dir: Path,
) -> Path:
    async_jobs = runtime.workspace / ".agent-runtime" / "async_jobs"
    deadline = time.monotonic() + runtime.wait_seconds
    while time.monotonic() < deadline:
        task = query_task(runtime, task_id)
        approve_if_needed(runtime, task, case_dir)
        status = str((task.get("data") or {}).get("status") or "")
        if status in TERMINAL:
            raise RestartBoundaryFailure(
                f"task became {status} before durable job start was observed"
            )
        if async_jobs.is_dir():
            candidates = [
                path
                for path in async_jobs.iterdir()
                if path.is_dir()
                and (path / "pid").is_file()
                and (path / "job_id").is_file()
            ]
            if len(candidates) == 1:
                return candidates[0]
        time.sleep(0.02)
    raise RestartBoundaryFailure("local process job did not reach durable start")


def wait_for_terminal(
    runtime: IsolatedRuntime,
    task_id: str,
    case_dir: Path,
) -> dict[str, Any]:
    deadline = time.monotonic() + runtime.wait_seconds
    while time.monotonic() < deadline:
        task = query_task(runtime, task_id)
        approve_if_needed(runtime, task, case_dir)
        status = str((task.get("data") or {}).get("status") or "")
        if status in TERMINAL:
            write_json(case_dir / "post_restart_task.json", task)
            return task
        time.sleep(0.25)
    raise RestartBoundaryFailure("task did not become terminal after restart")


def mutation_count(runtime: IsolatedRuntime, task_id: str) -> int:
    database = runtime.workspace / "data" / "tasks.sqlite"
    with sqlite3.connect(database) as connection:
        row = connection.execute(
            "SELECT COUNT(*) FROM task_mutation_ledger WHERE task_id = ?1",
            (task_id,),
        ).fetchone()
    return int(row[0] if row else 0)


def assert_success_result(
    runtime: IsolatedRuntime,
    task_id: str,
    final: dict[str, Any],
    marker: str,
    counter_name: str,
) -> int:
    data = final.get("data") or {}
    serialized = json.dumps(data.get("result_json") or {}, ensure_ascii=False)
    if data.get("status") != "succeeded":
        raise RestartBoundaryFailure(
            f"restart task reached unexpected status {data.get('status')}"
        )
    if marker not in serialized:
        raise RestartBoundaryFailure(f"restart result is missing marker {marker}")
    counter = runtime.workspace / "document" / counter_name
    lines = counter.read_text(encoding="utf-8").splitlines()
    if lines != ["mutation-once"]:
        raise RestartBoundaryFailure(f"non-idempotent command replayed: {lines}")
    count = mutation_count(runtime, task_id)
    if count != 1:
        raise RestartBoundaryFailure(f"mutation ledger count is {count}, expected 1")
    return count


def run_start_boundary(binary: Path, log_root: Path, wait_seconds: int) -> tuple[dict[str, Any], str]:
    case_dir = log_root / "start_boundary"
    runtime = IsolatedRuntime(binary, case_dir, wait_seconds)
    try:
        runtime.start("before_restart")
        task_id = submit_command(
            runtime,
            case_dir,
            (
                "printf 'mutation-once\\n' >> document/start-boundary-counter.txt; "
                "sleep 12; printf 'APP_RESTART_START_DONE\\n'"
            ),
            sleep_seconds=12,
        )
        job_dir = wait_for_job_start(runtime, task_id, case_dir)
        job_id = (job_dir / "job_id").read_text(encoding="utf-8").strip()
        pid = int((job_dir / "pid").read_text(encoding="utf-8").strip())
        write_json(case_dir / "pre_restart_task.json", query_task(runtime, task_id))
        runtime.stop()
        if not process_group_alive(pid):
            raise RestartBoundaryFailure("started job did not survive clawd stop")
        runtime.start("after_restart")
        final = wait_for_terminal(runtime, task_id, case_dir)
        count = assert_success_result(
            runtime,
            task_id,
            final,
            "APP_RESTART_START_DONE",
            "start-boundary-counter.txt",
        )
        return {
            "status": "pass",
            "task_id": task_id,
            "async_job_id": job_id,
            "mutation_count": count,
            "terminal_state": "succeeded",
        }, runtime.key
    finally:
        runtime.cleanup()


def run_poll_boundary(binary: Path, log_root: Path, wait_seconds: int) -> tuple[dict[str, Any], str]:
    case_dir = log_root / "poll_boundary"
    runtime = IsolatedRuntime(binary, case_dir, wait_seconds)
    try:
        runtime.start("before_restart")
        task_id = submit_command(
            runtime,
            case_dir,
            (
                "printf 'mutation-once\\n' >> document/poll-boundary-counter.txt; "
                "sleep 12; printf 'APP_RESTART_POLL_DONE\\n'"
            ),
            sleep_seconds=12,
        )
        _, checkpoint = wait_for_checkpoint(runtime, task_id, case_dir)
        job = checkpoint.get("pending_async_job") or {}
        job_id = str(job.get("job_id") or "")
        job_dir = job_dir_from_ref(str(job.get("cancel_ref") or ""))
        pid = int((job_dir / "pid").read_text(encoding="utf-8").strip())
        runtime.stop()
        if not process_group_alive(pid):
            raise RestartBoundaryFailure("polled job did not survive clawd stop")
        runtime.start("after_restart")
        final = wait_for_terminal(runtime, task_id, case_dir)
        count = assert_success_result(
            runtime,
            task_id,
            final,
            "APP_RESTART_POLL_DONE",
            "poll-boundary-counter.txt",
        )
        return {
            "status": "pass",
            "task_id": task_id,
            "checkpoint_id": checkpoint.get("checkpoint_id"),
            "async_job_id": job_id,
            "mutation_count": count,
            "terminal_state": "succeeded",
        }, runtime.key
    finally:
        runtime.cleanup()


def run_cancel_boundary(
    binary: Path,
    log_root: Path,
    wait_seconds: int,
) -> tuple[dict[str, Any], str]:
    case_dir = log_root / "cancel_boundary"
    runtime = IsolatedRuntime(binary, case_dir, wait_seconds)
    try:
        runtime.start("before_restart")
        task_id = submit_command(
            runtime,
            case_dir,
            (
                "printf 'mutation-once\\n' >> document/cancel-boundary-counter.txt; "
                "trap '' TERM; sleep 60; printf 'UNEXPECTED_CANCEL_RESTART_MISS\\n'"
            ),
            sleep_seconds=60,
        )
        _, checkpoint = wait_for_checkpoint(runtime, task_id, case_dir)
        job = checkpoint.get("pending_async_job") or {}
        job_id = str(job.get("job_id") or "")
        job_dir = job_dir_from_ref(str(job.get("cancel_ref") or ""))
        pid = int((job_dir / "pid").read_text(encoding="utf-8").strip())
        cancel_started = time.monotonic()
        idempotency_key = f"restart-boundary-cancel-{task_id}"
        first = runtime.request(
            "POST", "/v1/tasks/cancel-by-task-id", {"task_id": task_id, "idempotency_key": idempotency_key}
        )
        write_json(case_dir / "cancel_before_restart.json", first)
        if (first.get("data") or {}).get("status") != "task_cancelled":
            raise RestartBoundaryFailure(f"first cancel failed: {first}")
        if not (job_dir / "cancel_requested_at").is_file():
            raise RestartBoundaryFailure("cancel marker was not persisted before restart")
        runtime.stop()
        stopped_after_seconds = time.monotonic() - cancel_started
        if stopped_after_seconds >= 5:
            raise RestartBoundaryFailure("clawd did not stop within the cancel grace window")
        if not process_group_alive(pid):
            raise RestartBoundaryFailure(
                "TERM-ignoring process exited before restart recovery could be tested"
            )
        runtime.start("after_restart")
        deadline = time.monotonic() + wait_seconds
        while time.monotonic() < deadline:
            if (job_dir / "cancel_escalated_signal").is_file() and not process_group_alive(pid):
                break
            time.sleep(0.1)
        else:
            raise RestartBoundaryFailure(
                "pending cancellation was not escalated after clawd restart"
            )
        stable = query_task(runtime, task_id)
        write_json(case_dir / "post_restart_task.json", stable)
        if (stable.get("data") or {}).get("status") != "canceled":
            raise RestartBoundaryFailure("canceled task terminal state regressed after restart")
        second = runtime.request(
            "POST", "/v1/tasks/cancel-by-task-id", {"task_id": task_id, "idempotency_key": idempotency_key}
        )
        write_json(case_dir / "cancel_after_restart.json", second)
        if (second.get("data") or {}).get("status") != "task_cancelled":
            raise RestartBoundaryFailure("repeated cancel was not idempotent after restart")
        counter = runtime.workspace / "document" / "cancel-boundary-counter.txt"
        if counter.read_text(encoding="utf-8").splitlines() != ["mutation-once"]:
            raise RestartBoundaryFailure("cancel-boundary command replayed")
        count = mutation_count(runtime, task_id)
        if count != 1:
            raise RestartBoundaryFailure(
                f"cancel-boundary mutation ledger count is {count}, expected 1"
            )
        return {
            "status": "pass",
            "task_id": task_id,
            "checkpoint_id": checkpoint.get("checkpoint_id"),
            "async_job_id": job_id,
            "mutation_count": count,
            "restart_within_grace_seconds": round(stopped_after_seconds, 3),
            "cancel_escalated_signal": (
                job_dir / "cancel_escalated_signal"
            ).read_text(encoding="utf-8").strip(),
            "terminal_state": "canceled",
            "process_alive": False,
        }, runtime.key
    finally:
        runtime.cleanup()


def evidence_paths(log_root: Path) -> list[str]:
    paths = [
        path.relative_to(log_root).as_posix()
        for path in sorted(log_root.rglob("*"))
        if path.is_file()
    ]
    if "summary.json" not in paths:
        paths.append("summary.json")
    return sorted(paths)


def finalize_summary(
    summary: dict[str, Any],
    log_root: Path,
    binary: Path,
    auto_build: bool,
    started_at: str,
    admin_keys: list[str],
) -> bool:
    lifecycle.LOG_DIR = log_root
    values = lifecycle.sensitive_values(None) + admin_keys
    findings = lifecycle.scan_evidence(sorted(set(values), key=len, reverse=True))
    cases = summary.get("cases") if isinstance(summary.get("cases"), dict) else {}
    passed = sum(
        1
        for result in cases.values()
        if isinstance(result, dict) and result.get("status") == "pass"
    )
    failed = len(CASE_IDS) - passed
    if findings:
        summary["status"] = "fail"
        summary["error_code"] = "evidence_secret_scan_failed"
        summary["secret_scan_findings"] = findings
    head = lifecycle.command_stdout(["git", "rev-parse", "HEAD"])
    summary.update(
        {
            "schema_version": 1,
            "source_commit": head,
            "source_commit_pushed": head
            == lifecycle.command_stdout(["git", "rev-parse", "origin/main"]),
            "worktree": lifecycle.worktree_summary(),
            "platform": lifecycle.platform.system().lower(),
            "arch": lifecycle.platform.machine().lower(),
            "binary": {
                "path": lifecycle.repo_path_ref(binary),
                "sha256": lifecycle.sha256_file(binary),
            },
            "ui": {
                "path": "UI/dist",
                "tree_sha256": lifecycle.sha256_tree(ROOT / "UI" / "dist"),
                "digest_algorithm": "sha256(relative_path_nul_file_sha256_nul)",
            },
            "case_counts": {
                "total": len(CASE_IDS),
                "passed": passed,
                "failed": failed,
            },
            "started_at": started_at,
            "finished_at": utc_now(),
            "build_strategy": {
                "auto_build": auto_build,
                "cargo_environment": (
                    "scripts/shell_compat.sh:configure_cargo_build_environment"
                    if auto_build
                    else "existing_binary"
                ),
            },
            "evidence_root": lifecycle.repo_path_ref(log_root),
            "evidence_relative_paths": evidence_paths(log_root),
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
        summary["error"] = lifecycle.redact_text(summary["error"], values)
    write_json(log_root / "summary.json", summary)
    return not findings and failed == 0


def ensure_binary(binary: Path, auto_build: bool) -> None:
    if auto_build:
        subprocess.run(
            [
                "bash",
                "-lc",
                "source scripts/shell_compat.sh; "
                "configure_cargo_build_environment; cargo build -p clawd",
            ],
            cwd=ROOT,
            check=True,
        )
    if not binary.is_file() or not os.access(binary, os.X_OK):
        raise RestartBoundaryFailure(f"clawd binary missing or not executable: {binary}")


def main(argv: list[str] | None = None) -> int:
    args = parse_args(argv)
    if args.list_cases:
        for case_id in CASE_IDS:
            print(case_id)
        return 0
    binary = args.binary.expanduser().resolve()
    log_root = args.log_dir.expanduser().resolve()
    auto_build = not args.no_build
    if args.wait_seconds < 10:
        print("--wait-seconds must be at least 10", file=sys.stderr)
        return 2
    started_at = utc_now()
    log_root.mkdir(parents=True, exist_ok=True)
    summary: dict[str, Any] = {"status": "fail", "cases": {}}
    admin_keys: list[str] = []
    exit_code = 1
    try:
        ensure_binary(binary, auto_build)
        runners = (
            ("start_boundary", run_start_boundary),
            ("poll_boundary", run_poll_boundary),
            ("cancel_boundary", run_cancel_boundary),
        )
        for case_id, runner in runners:
            print(f"[CASE] {case_id}", flush=True)
            result, admin_key = runner(binary, log_root, args.wait_seconds)
            admin_keys.append(admin_key)
            summary["cases"][case_id] = result
            print(f"[PASS] {case_id}", flush=True)
        summary["status"] = "pass"
        exit_code = 0
    except Exception as error:
        summary["error"] = str(error)
        print(f"[FAIL] {error}", file=sys.stderr, flush=True)
    if not finalize_summary(
        summary,
        log_root,
        binary,
        auto_build,
        started_at,
        admin_keys,
    ):
        exit_code = 1
    print(json.dumps(summary, ensure_ascii=False), flush=True)
    return exit_code


if __name__ == "__main__":
    raise SystemExit(main())
