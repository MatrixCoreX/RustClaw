#!/usr/bin/env python3
"""Contract tests for the restart-boundary regression harness."""

from __future__ import annotations

import importlib.util
from pathlib import Path
import unittest


SCRIPT_PATH = Path(__file__).with_name("regression_clawd_restart_boundaries.py")


def load_harness():
    spec = importlib.util.spec_from_file_location("restart_boundary_harness", SCRIPT_PATH)
    assert spec and spec.loader
    module = importlib.util.module_from_spec(spec)
    spec.loader.exec_module(module)
    return module


class RestartBoundaryHarnessContractTest(unittest.TestCase):
    @classmethod
    def setUpClass(cls) -> None:
        cls.harness = load_harness()

    def test_case_inventory_covers_all_required_restart_boundaries(self) -> None:
        self.assertEqual(
            self.harness.CASE_IDS,
            ("start_boundary", "poll_boundary", "cancel_boundary"),
        )

    def test_local_process_job_ref_is_resolved_exactly(self) -> None:
        self.assertEqual(
            self.harness.job_dir_from_ref("local_process:/tmp/job-1"),
            Path("/tmp/job-1"),
        )
        with self.assertRaises(self.harness.RestartBoundaryFailure):
            self.harness.job_dir_from_ref("provider:job-1")
        with self.assertRaises(self.harness.RestartBoundaryFailure):
            self.harness.job_dir_from_ref("local_process:")

    def test_wait_floor_rejects_invalid_value_before_runtime_start(self) -> None:
        self.assertEqual(
            self.harness.main(
                [
                    "--no-build",
                    "--binary",
                    str(SCRIPT_PATH),
                    "--wait-seconds",
                    "9",
                ]
            ),
            2,
        )


if __name__ == "__main__":
    unittest.main()
