#!/usr/bin/env python3
"""Run real clawd command-lifecycle regressions against an isolated workspace."""

from __future__ import annotations

import json
import os
from pathlib import Path
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


class RegressionFailure(RuntimeError):
    pass


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
    text = replace_once(text, r'^task_timeout_seconds\s*=\s*\d+$', "task_timeout_seconds = 300")
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

    # Use a fixed host builtin for the concurrent queue check. Runner skills
    # intentionally require immutable package receipts, while this isolated
    # workspace contains no release receipts by design.
    short_task_id = submit_skill(server, "list_dir", {"path": "."}, case)
    started = time.monotonic()
    short_task, _ = wait_for_terminal(server, short_task_id, case, timeout=20)
    elapsed = time.monotonic() - started
    long_during = query_task(server, long_task_id)
    write_json(LOG_DIR / case / "long_task_during_health.json", long_during)
    assert_real_execution(case, short_task)
    if (short_task.get("data") or {}).get("status") != "succeeded":
        raise RegressionFailure(f"{case}: concurrent short task did not succeed")
    if (long_during.get("data") or {}).get("status") not in {"queued", "running"}:
        raise RegressionFailure(f"{case}: long command was not active when health completed")
    return {
        "task_id": short_task_id,
        "health_elapsed_seconds": round(health_elapsed, 3),
        "short_task_elapsed_seconds": round(elapsed, 3),
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


def ensure_binary() -> None:
    if AUTO_BUILD:
        subprocess.run(
            ["bash", "-lc", "source scripts/shell_compat.sh; configure_cargo_build_environment; cargo build -p clawd"],
            cwd=ROOT,
            check=True,
        )
    if not CLAWD_BIN.is_file() or not os.access(CLAWD_BIN, os.X_OK):
        raise RegressionFailure(f"clawd binary missing or not executable: {CLAWD_BIN}")


def main() -> int:
    LOG_DIR.mkdir(parents=True, exist_ok=True)
    ensure_binary()
    workspace = prepare_workspace()
    server: Server | None = None
    summary: dict[str, Any] = {
        "status": "fail",
        "submission_mode": "natural_language" if SUBMIT_VIA_NL else "direct_run_skill",
        "cases": {},
        "log_dir": str(LOG_DIR),
    }
    try:
        server = Server(workspace)
        print(f"[INFO] isolated clawd={server.base_url} log_dir={LOG_DIR}", flush=True)

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
        summary["cases"]["concurrent_health"] = run_health_concurrency(server, heartbeat_task_id)
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

        summary["cases"]["deadline_5s"] = run_deadline_case(server)
        print("[PASS] explicit 5s runtime deadline", flush=True)

        summary["cases"]["cancel_idempotent"] = run_cancel_case(server)
        print("[PASS] cancel and repeated cancel", flush=True)

        summary["status"] = "pass"
        write_json(LOG_DIR / "summary.json", summary)
        print(json.dumps(summary, ensure_ascii=False), flush=True)
        return 0
    except Exception as error:
        summary["error"] = str(error)
        write_json(LOG_DIR / "summary.json", summary)
        print(f"[FAIL] {error}", file=sys.stderr, flush=True)
        return 1
    finally:
        if server is not None:
            server.close()
        model_log = workspace / "logs/model_io.log"
        if model_log.is_file():
            shutil.copy2(model_log, LOG_DIR / "model_io.log")
        if os.environ.get("KEEP_WORKSPACE", "0") == "1":
            print(f"[INFO] retained isolated workspace: {workspace}", flush=True)
        else:
            shutil.rmtree(workspace)


if __name__ == "__main__":
    raise SystemExit(main())
