#!/usr/bin/env python3
"""Unit tests for the active-checkpoint health probe."""
from __future__ import annotations

import tempfile
import unittest
from pathlib import Path
from unittest.mock import patch

from concurrent_health_probe import NOT_READY_EXIT, active_async_context, probe


class FakeResponse:
    status = 200

    def __enter__(self) -> "FakeResponse":
        return self

    def __exit__(self, *_args: object) -> None:
        return None

    def read(self) -> bytes:
        return b'{"ok":true,"data":{"worker_state":"running","queue_length":1,"running_length":1}}'


class ConcurrentHealthProbeTest(unittest.TestCase):
    def test_finds_nested_active_async_checkpoint(self) -> None:
        context = active_async_context(
            {
                "task_checkpoint": {
                    "checkpoint_id": "checkpoint-1",
                    "pending_async_job": {"job_id": "local_process:job-1"},
                }
            }
        )
        self.assertEqual(
            context,
            {
                "checkpoint_id": "checkpoint-1",
                "async_job_id": "local_process:job-1",
            },
        )

    def test_probe_waits_until_task_has_active_checkpoint(self) -> None:
        with tempfile.TemporaryDirectory() as tmp:
            task_path = Path(tmp) / "task.json"
            output_path = Path(tmp) / "evidence.json"
            task_path.write_text(
                '{"data":{"status":"running","result_json":{}}}',
                encoding="utf-8",
            )
            self.assertEqual(
                probe(task_path, output_path, "http://127.0.0.1:1", "secret"),
                NOT_READY_EXIT,
            )
            self.assertFalse(output_path.exists())

    @patch("concurrent_health_probe.urlopen", return_value=FakeResponse())
    def test_probe_records_health_without_recording_key(self, _urlopen: object) -> None:
        with tempfile.TemporaryDirectory() as tmp:
            task_path = Path(tmp) / "task.json"
            output_path = Path(tmp) / "evidence.json"
            task_path.write_text(
                """
                {"data":{"status":"running","result_json":{"task_checkpoint":{
                  "checkpoint_id":"checkpoint-1",
                  "pending_async_job":{"job_id":"local_process:job-1"}
                }}}}
                """,
                encoding="utf-8",
            )
            self.assertEqual(
                probe(task_path, output_path, "http://127.0.0.1:8787", "secret"),
                0,
            )
            evidence = output_path.read_text(encoding="utf-8")
            self.assertIn('"health_ok": true', evidence)
            self.assertIn('"task_status": "running"', evidence)
            self.assertNotIn("secret", evidence)


if __name__ == "__main__":
    unittest.main()
