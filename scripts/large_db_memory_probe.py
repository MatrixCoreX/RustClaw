#!/usr/bin/env python3
"""Read-only pagination and Python heap probe for a runtime SQLite database."""

from __future__ import annotations

import argparse
import json
import pathlib
import sqlite3
import time
import tracemalloc
from collections.abc import Callable
from typing import Any


TERMINAL_STATUSES = ("succeeded", "failed", "canceled", "timeout")


def open_read_only(path: pathlib.Path) -> sqlite3.Connection:
    uri = f"file:{path.resolve()}?mode=ro"
    connection = sqlite3.connect(uri, uri=True, timeout=30.0)
    connection.execute("PRAGMA query_only=ON")
    connection.execute("PRAGMA busy_timeout=30000")
    return connection


def table_exists(connection: sqlite3.Connection, table: str) -> bool:
    return (
        connection.execute(
            "SELECT 1 FROM sqlite_master WHERE type='table' AND name=?", (table,)
        ).fetchone()
        is not None
    )


def table_count(connection: sqlite3.Connection, table: str) -> int:
    if not table_exists(connection, table):
        return 0
    return int(connection.execute(f'SELECT COUNT(*) FROM "{table}"').fetchone()[0])


def measure(label: str, operation: Callable[[], Any]) -> dict[str, Any]:
    tracemalloc.start()
    started = time.monotonic()
    value = operation()
    current, peak = tracemalloc.get_traced_memory()
    tracemalloc.stop()
    item_count = len(value) if isinstance(value, list) else None
    return {
        "label": label,
        "elapsed_ms": round((time.monotonic() - started) * 1000, 3),
        "python_heap_current_bytes": current,
        "python_heap_peak_bytes": peak,
        "item_count": item_count,
    }


def history_page(connection: sqlite3.Connection, offset: int) -> list[tuple[Any, ...]]:
    placeholders = ",".join("?" for _ in TERMINAL_STATUSES)
    return connection.execute(
        f"""SELECT task_id, kind, payload_json, status, channel, user_id,
                   external_user_id,
                   CAST(COALESCE(NULLIF(created_at, ''), '0') AS INTEGER),
                   CAST(COALESCE(NULLIF(updated_at, ''), created_at, '0') AS INTEGER)
            FROM tasks
            WHERE status IN ({placeholders})
            ORDER BY CAST(COALESCE(NULLIF(created_at, ''), '0') AS INTEGER) DESC,
                     task_id DESC
            LIMIT 50 OFFSET ?""",
        (*TERMINAL_STATUSES, offset),
    ).fetchall()


def busiest_event_task(connection: sqlite3.Connection) -> tuple[str | None, int]:
    if not table_exists(connection, "task_event_archive"):
        return None, 0
    row = connection.execute(
        """SELECT task_id, COUNT(*) AS event_count
           FROM task_event_archive
           GROUP BY task_id
           ORDER BY event_count DESC
           LIMIT 1"""
    ).fetchone()
    return (str(row[0]), int(row[1])) if row else (None, 0)


def event_page(connection: sqlite3.Connection, task_id: str | None) -> list[Any]:
    if task_id is None:
        return []
    rows = connection.execute(
        """SELECT event_json
           FROM task_event_archive
           WHERE task_id = ? AND seq > 0
           ORDER BY seq ASC
           LIMIT 1024""",
        (task_id,),
    ).fetchall()
    return [json.loads(row[0]) for row in rows]


def artifact_page(connection: sqlite3.Connection) -> list[tuple[Any, ...]]:
    if not table_exists(connection, "task_event_artifacts"):
        return []
    return connection.execute(
        """SELECT task_id, artifact_id, payload_bytes, created_at_ms
           FROM task_event_artifacts
           ORDER BY rowid DESC
           LIMIT 100"""
    ).fetchall()


def bounded_result_projection(connection: sqlite3.Connection) -> list[tuple[Any, ...]]:
    return connection.execute(
        """SELECT task_id, length(result_json), substr(result_json, 1, 65536)
           FROM tasks
           WHERE result_json IS NOT NULL
           ORDER BY length(result_json) DESC
           LIMIT 20"""
    ).fetchall()


def run_probe(path: pathlib.Path) -> dict[str, Any]:
    connection = open_read_only(path)
    try:
        counts = {
            table: table_count(connection, table)
            for table in (
                "tasks",
                "task_event_stream",
                "task_event_archive",
                "task_event_artifacts",
            )
        }
        task_id, event_count = busiest_event_task(connection)
        workloads = [
            measure("task_history_first_page", lambda: history_page(connection, 0)),
            measure("task_history_deep_page", lambda: history_page(connection, 100_000)),
            measure("task_event_replay_page", lambda: event_page(connection, task_id)),
            measure("task_artifact_metadata_page", lambda: artifact_page(connection)),
            measure(
                "bounded_large_result_projection",
                lambda: bounded_result_projection(connection),
            ),
        ]
        return {
            "schema_version": 1,
            "database_bytes": path.stat().st_size,
            "database_wal_bytes": pathlib.Path(f"{path}-wal").stat().st_size
            if pathlib.Path(f"{path}-wal").exists()
            else 0,
            "counts": counts,
            "largest_event_task": {
                "task_id_sha256_prefix": __import__("hashlib")
                .sha256((task_id or "").encode())
                .hexdigest()[:16],
                "event_count": event_count,
            },
            "workloads": workloads,
            "integrity_check": connection.execute("PRAGMA quick_check").fetchone()[0],
        }
    finally:
        connection.close()


def validate_plan_scale(report: dict[str, Any]) -> list[str]:
    errors = []
    if report["database_bytes"] < 8 * 1024**3:
        errors.append("database_smaller_than_8_gib")
    if report["counts"]["task_event_archive"] < 10_000:
        errors.append("fewer_than_10000_archived_events")
    if report["counts"]["task_event_artifacts"] < 100:
        errors.append("fewer_than_100_artifacts")
    if report["integrity_check"] != "ok":
        errors.append("sqlite_quick_check_failed")
    return errors


def main() -> int:
    parser = argparse.ArgumentParser(description=__doc__)
    parser.add_argument("--database", type=pathlib.Path, required=True)
    parser.add_argument("--require-plan-scale", action="store_true")
    args = parser.parse_args()
    report = run_probe(args.database)
    report["validation_errors"] = (
        validate_plan_scale(report) if args.require_plan_scale else []
    )
    print(json.dumps(report, indent=2, sort_keys=True))
    return int(bool(report["validation_errors"]))


if __name__ == "__main__":
    raise SystemExit(main())
