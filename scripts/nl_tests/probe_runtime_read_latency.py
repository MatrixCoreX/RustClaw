#!/usr/bin/env python3
"""Read-only operator probe; emits aggregate timings, never task bodies or keys."""
from __future__ import annotations

import argparse
import json
import math
from pathlib import Path
import platform
import sqlite3
import time
import tomllib
import urllib.request


def timing_summary(values: list[float]) -> dict[str, float | int]:
    ordered = sorted(values)
    if not ordered:
        return {"samples": 0}
    return {"samples": len(ordered), "p50_ms": round(ordered[(len(ordered) - 1) // 2], 3),
            "p95_ms": round(ordered[math.ceil(len(ordered) * 0.95) - 1], 3),
            "max_ms": round(ordered[-1], 3)}


def probe(root: Path, samples: int) -> dict:
    config = tomllib.loads((root / "configs/config.toml").read_text())
    database = (root / config["database"]["sqlite_path"]).resolve()
    report = {"platform": platform.system(), "architecture": platform.machine(),
              "read_only": True, "samples_requested": samples}
    with sqlite3.connect(database.as_uri() + "?mode=ro", uri=True, timeout=1) as db:
        db.execute("PRAGMA query_only=ON")
        report["database"] = {
            "journal_mode": db.execute("PRAGMA journal_mode").fetchone()[0],
            "page_count": db.execute("PRAGMA page_count").fetchone()[0],
            "page_size": db.execute("PRAGMA page_size").fetchone()[0],
            "free_pages": db.execute("PRAGMA freelist_count").fetchone()[0],
            "file_bytes": database.stat().st_size,
            "wal_bytes": Path(str(database) + "-wal").stat().st_size if Path(str(database) + "-wal").exists() else 0,
            "tasks_by_status": dict(db.execute("SELECT status, count(*) FROM tasks GROUP BY status")),
        }
        key = db.execute("SELECT user_key FROM auth_keys WHERE role='admin' AND enabled=1 LIMIT 1").fetchone()
        query = "SELECT task_id, length(result_json) FROM tasks WHERE status='running' AND result_json IS NOT NULL AND lease_expires_at <= ? ORDER BY CAST(COALESCE(NULLIF(updated_at,''),created_at,'0') AS INTEGER), task_id LIMIT 128 OFFSET ?"
        report["recovery_read_plan"] = [row[3] for row in db.execute("EXPLAIN QUERY PLAN " + query, (int(time.time()), 0))]
        for offset in (0, 1024):
            timings = []
            returned = 0
            for _ in range(samples):
                start = time.perf_counter()
                returned = len(db.execute(query, (int(time.time()), offset)).fetchall())
                timings.append((time.perf_counter() - start) * 1000)
            report[f"recovery_query_offset_{offset}"] = {**timing_summary(timings), "rows": returned}
    if key:
        opener = urllib.request.build_opener(urllib.request.ProxyHandler({}))
        timings = []
        failures = 0
        for _ in range(samples):
            start = time.perf_counter()
            try:
                request = urllib.request.Request("http://127.0.0.1:8787/v1/health", headers={"X-Agent-Key": key[0]})
                with opener.open(request, timeout=30) as response:
                    if not json.loads(response.read(64 * 1024)).get("ok"):
                        failures += 1
            except Exception:
                failures += 1
            timings.append((time.perf_counter() - start) * 1000)
        report["health"] = {**timing_summary(timings), "failures": failures}
    else:
        report["health"] = {"status": "enabled_admin_key_unavailable"}
    return report


if __name__ == "__main__":
    parser = argparse.ArgumentParser()
    parser.add_argument("--root", type=Path, required=True)
    parser.add_argument("--samples", type=int, default=10, choices=range(1, 51))
    args = parser.parse_args()
    print(json.dumps(probe(args.root.resolve(), args.samples), sort_keys=True))
