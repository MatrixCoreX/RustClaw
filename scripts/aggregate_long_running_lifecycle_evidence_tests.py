#!/usr/bin/env python3
"""Tests for long-running lifecycle evidence aggregation."""

from __future__ import annotations

import importlib.util
import json
from pathlib import Path
import tempfile
import unittest


SCRIPT_PATH = Path(__file__).with_name("aggregate_long_running_lifecycle_evidence.py")


def load_aggregator():
    spec = importlib.util.spec_from_file_location("long_running_lifecycle_aggregator", SCRIPT_PATH)
    assert spec and spec.loader
    module = importlib.util.module_from_spec(spec)
    spec.loader.exec_module(module)
    return module


class AggregateContractTest(unittest.TestCase):
    @classmethod
    def setUpClass(cls) -> None:
        cls.aggregator = load_aggregator()

    def fixtures(self, root: Path, *, nl_result: str = "pass") -> dict[str, Path]:
        deterministic = root / "deterministic.json"
        restart = root / "restart.json"
        live_nl = root / "live.jsonl"
        deterministic.write_text(
            json.dumps(
                {
                    "status": "pass",
                    "source_commit": "commit-a",
                    "case_counts": {"total": 5, "passed": 5, "failed": 0},
                }
            ),
            encoding="utf-8",
        )
        restart.write_text(json.dumps({"status": "pass"}), encoding="utf-8")
        live_nl.write_text(
            json.dumps({"case_name": "heartbeat", "result": nl_result}) + "\n",
            encoding="utf-8",
        )
        return {
            "deterministic": deterministic,
            "restart_continuity": restart,
            "live_nl": live_nl,
        }

    def test_aggregates_all_three_evidence_classes(self) -> None:
        with tempfile.TemporaryDirectory() as directory:
            summary = self.aggregator.aggregate(self.fixtures(Path(directory)))

        self.assertEqual(summary["schema_version"], 1)
        self.assertEqual(summary["status"], "pass")
        self.assertEqual(summary["case_counts"], {"total": 7, "passed": 7, "failed": 0})
        self.assertEqual(
            [source["role"] for source in summary["sources"]],
            ["deterministic", "restart_continuity", "live_nl"],
        )
        self.assertTrue(all(not Path(source["path"]).is_absolute() for source in summary["sources"]))
        self.aggregator.validate_output_secrets(summary)

    def test_live_nl_failure_fails_aggregate(self) -> None:
        with tempfile.TemporaryDirectory() as directory:
            summary = self.aggregator.aggregate(
                self.fixtures(Path(directory), nl_result="fail")
            )

        self.assertEqual(summary["status"], "fail")
        self.assertEqual(summary["case_counts"], {"total": 7, "passed": 6, "failed": 1})

    def test_rejects_inconsistent_case_counts(self) -> None:
        with self.assertRaises(self.aggregator.AggregateError):
            self.aggregator.object_counts(
                {
                    "case_counts": {
                        "total": 2,
                        "passed": 2,
                        "failed": 1,
                    }
                }
            )


if __name__ == "__main__":
    unittest.main()
