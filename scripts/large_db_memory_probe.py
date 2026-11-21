#!/usr/bin/env python3
"""Read-only pagination and Python heap probe for a runtime SQLite database."""

from __future__ import annotations

import argparse
import json
import hashlib
import struct
import zlib
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


def decode_stored_event(raw: str, version: int, blob: bytes | None, connection=None, cache=None) -> Any:
    if version == 3:
        if connection is None or blob is None or len(blob) != 68:
            raise ValueError("event_chunk_reference_invalid")
        row = connection.execute("SELECT payload_json,storage_version,storage_blob FROM task_event_chunks WHERE chunk_id=?", (blob[:32],)).fetchone()
        if row is None or row[1] not in (1, 2):
            raise ValueError("event_chunk_missing")
        key = hashlib.sha256(blob[:32] + row[0].encode() + (row[2] or b"")).digest()
        entries = cache.get(key) if cache is not None else None
        if entries is None:
            entries = decode_stored_event(*row)
            digest = hashlib.sha256(row[0].encode()).digest() if row[1] == 1 else row[2][13:45]
            if digest != blob[:32] or not isinstance(entries, list) or len(entries) > 128:
                raise ValueError("event_chunk_digest_mismatch")
            if cache is not None:
                if len(cache) >= 2: cache.pop(next(iter(cache)))
                cache[key] = entries
        index = struct.unpack("<I", blob[32:36])[0]
        if index >= len(entries) or not isinstance(entries[index], str):
            raise ValueError("event_chunk_index_invalid")
        decoded = entries[index].encode()
        if hashlib.sha256(decoded).digest() != blob[36:]:
            raise ValueError("event_chunk_event_digest_mismatch")
        event = json.loads(decoded)
        projection = {"payload": {k: event.get("payload", {})[k] for k in ("skill", "requested_action_ref") if k in event.get("payload", {})}}
        if "event_kind" in event: projection["event_kind"] = event["event_kind"]
        if json.loads(raw) != projection: raise ValueError("event_chunk_projection_mismatch")
        return event
    if version in (0, 1):
        return json.loads(raw)
    if version != 2 or blob is None or not 45 <= len(blob) <= 16 * 1024 * 1024 + 45:
        raise ValueError("event_storage_blob_invalid")
    length = struct.unpack("<Q", blob[5:13])[0]
    if blob[:4] != b"EVJ1" or blob[4] > 1 or length > 16 * 1024 * 1024:
        raise ValueError("event_storage_header_invalid")
    decoder = zlib.decompressobj()
    decoded = decoder.decompress(blob[45:], length + 1)
    if len(decoded) != length or not decoder.eof or decoder.unused_data or decoder.unconsumed_tail:
        raise ValueError("event_storage_length_mismatch")
    if hashlib.sha256(decoded).digest() != blob[13:45]:
        raise ValueError("event_storage_digest_mismatch")
    event = json.loads(decoded)
    projection = {}
    if blob[4]:
        projection = {"payload": {k: event.get("payload", {})[k] for k in ("skill", "requested_action_ref") if k in event.get("payload", {})}}
        if "event_kind" in event:
            projection["event_kind"] = event["event_kind"]
    if json.loads(raw) != projection:
        raise ValueError("event_storage_projection_mismatch")
    return event


def event_page(connection: sqlite3.Connection, task_id: str | None) -> list[Any]:
    if task_id is None:
        return []
    columns = {row[1] for row in connection.execute("PRAGMA table_info(task_event_archive)")}
    version = "storage_version" if "storage_version" in columns else "0"
    blob = "storage_blob" if "storage_blob" in columns else "NULL"
    rows = connection.execute(
        f"""SELECT event_json, {version}, {blob}
           FROM task_event_archive
           WHERE task_id = ? AND seq > 0
           ORDER BY seq ASC
           LIMIT 1024""",
        (task_id,),
    ).fetchall()
    cache = {}
    return [decode_stored_event(*row, connection=connection, cache=cache) for row in rows]


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
