#!/usr/bin/env python3
"""Probe runtime health while an NL task owns an active async checkpoint."""
from __future__ import annotations

import argparse
import json
import os
import time
from datetime import datetime, timezone
from pathlib import Path
from typing import Any
from urllib.error import HTTPError, URLError
from urllib.request import Request, urlopen


NOT_READY_EXIT = 3


def active_async_context(value: Any) -> dict[str, str] | None:
    if isinstance(value, dict):
        pending = value.get("pending_async_job")
        if isinstance(pending, dict) and pending.get("job_id"):
            return {
                "checkpoint_id": str(value.get("checkpoint_id") or ""),
                "async_job_id": str(pending.get("job_id") or ""),
            }
        for child in value.values():
            context = active_async_context(child)
            if context is not None:
                return context
    elif isinstance(value, list):
        for child in value:
            context = active_async_context(child)
            if context is not None:
                return context
    return None


def utc_now() -> str:
    return datetime.now(timezone.utc).isoformat().replace("+00:00", "Z")


def write_json(path: Path, value: dict[str, Any]) -> None:
    path.parent.mkdir(parents=True, exist_ok=True)
    path.write_text(
        json.dumps(value, ensure_ascii=False, indent=2, sort_keys=True) + "\n",
        encoding="utf-8",
    )


def probe(task_path: Path, output_path: Path, base_url: str, user_key: str) -> int:
    task = json.loads(task_path.read_text(encoding="utf-8"))
    data = task.get("data") if isinstance(task, dict) else None
    data = data if isinstance(data, dict) else {}
    task_status = str(data.get("status") or "")
    context = active_async_context(data.get("result_json"))
    if task_status != "running" or context is None:
        return NOT_READY_EXIT

    started = time.monotonic()
    request = Request(
        f"{base_url.rstrip('/')}/v1/health",
        headers={"X-Agent-Key": user_key},
    )
    evidence: dict[str, Any] = {
        "schema_version": 1,
        "observed_at": utc_now(),
        "task_status": task_status,
        **context,
    }
    try:
        with urlopen(request, timeout=5) as response:
            health = json.loads(response.read().decode("utf-8"))
            evidence["health_http_status"] = int(response.status)
    except (HTTPError, URLError, TimeoutError, json.JSONDecodeError) as exc:
        evidence.update(
            {
                "status": "fail",
                "error_type": type(exc).__name__,
                "elapsed_ms": round((time.monotonic() - started) * 1000),
            }
        )
        write_json(output_path, evidence)
        return 1

    health_data = health.get("data") if isinstance(health, dict) else None
    health_data = health_data if isinstance(health_data, dict) else {}
    health_ok = bool(health.get("ok")) and health_data.get("worker_state") == "running"
    evidence.update(
        {
            "status": "pass" if health_ok else "fail",
            "health_ok": health_ok,
            "worker_state": health_data.get("worker_state"),
            "queue_length": health_data.get("queue_length"),
            "running_length": health_data.get("running_length"),
            "elapsed_ms": round((time.monotonic() - started) * 1000),
        }
    )
    write_json(output_path, evidence)
    return 0 if health_ok else 1


def parse_args() -> argparse.Namespace:
    parser = argparse.ArgumentParser(description=__doc__)
    parser.add_argument("--task-json", type=Path, required=True)
    parser.add_argument("--output", type=Path, required=True)
    parser.add_argument("--base-url", required=True)
    parser.add_argument("--user-key", default=os.environ.get("APP_USER_KEY", ""))
    args = parser.parse_args()
    if not args.user_key:
        parser.error("APP_USER_KEY or --user-key is required")
    return args


def main() -> int:
    args = parse_args()
    return probe(args.task_json, args.output, args.base_url, args.user_key)


if __name__ == "__main__":
    raise SystemExit(main())
