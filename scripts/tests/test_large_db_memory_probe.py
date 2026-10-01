#!/usr/bin/env python3
"""Tests for the read-only large database memory probe."""

from __future__ import annotations

import importlib.util
import pathlib
import sqlite3
import tempfile
import unittest


ROOT = pathlib.Path(__file__).resolve().parents[2]
SPEC = importlib.util.spec_from_file_location(
    "large_db_memory_probe", ROOT / "scripts/large_db_memory_probe.py"
)
PROBE = importlib.util.module_from_spec(SPEC)
assert SPEC.loader is not None
SPEC.loader.exec_module(PROBE)


class LargeDatabaseMemoryProbeTests(unittest.TestCase):
    def test_probe_uses_bounded_pages_and_does_not_mutate_the_database(self) -> None:
        with tempfile.TemporaryDirectory(prefix="large-db-probe-") as directory:
            path = pathlib.Path(directory) / "runtime.db"
            connection = sqlite3.connect(path)
            connection.executescript(
                """
                CREATE TABLE tasks(
                    task_id TEXT PRIMARY KEY, user_id INTEGER, chat_id INTEGER,
                    channel TEXT, external_user_id TEXT, kind TEXT,
                    payload_json TEXT, status TEXT, result_json TEXT,
                    created_at TEXT, updated_at TEXT
                );
                CREATE TABLE task_event_archive(
                    task_id TEXT, seq INTEGER, event_json TEXT
                );
                CREATE TABLE task_event_stream(task_id TEXT, seq INTEGER);
                CREATE TABLE task_event_artifacts(
                    task_id TEXT, artifact_id TEXT, payload_json TEXT,
                    payload_bytes INTEGER, created_at_ms INTEGER
                );
                """
            )
            for index in range(120):
                connection.execute(
                    "INSERT INTO tasks VALUES(?, 1, 1, 'ui', NULL, 'ask', ?, 'succeeded', ?, ?, ?)",
                    (
                        f"task-{index}",
                        '{"text":"request"}',
                        '{"text":"result"}',
                        str(index),
                        str(index),
                    ),
                )
                connection.execute(
                    "INSERT INTO task_event_archive VALUES(?, ?, ?)",
                    ("task-0", index + 1, '{"event_kind":"progress"}'),
                )
                connection.execute(
                    "INSERT INTO task_event_artifacts VALUES(?, ?, '{}', 2, ?)",
                    ("task-0", f"artifact-{index}", index),
                )
            connection.commit()
            before_changes = connection.total_changes
            connection.close()

            report = PROBE.run_probe(path)

            self.assertEqual(report["counts"]["tasks"], 120)
            self.assertEqual(report["largest_event_task"]["event_count"], 120)
            self.assertEqual(report["integrity_check"], "ok")
            self.assertEqual(
                [workload["item_count"] for workload in report["workloads"]],
                [50, 0, 120, 100, 20],
            )
            reopened = sqlite3.connect(path)
            self.assertEqual(reopened.total_changes, 0)
            self.assertEqual(before_changes, 360)
            reopened.close()


if __name__ == "__main__":
    unittest.main(verbosity=2)
