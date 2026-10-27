#!/usr/bin/env python3
"""Reject empty or wrongly selected trace directories, even in mixed runs."""
from contextlib import redirect_stdout
import io
from pathlib import Path
import sys
import tempfile
import unittest
from unittest.mock import Mock, patch

import agent_loop_trace_release_gate_impl as gate


class TraceInputTests(unittest.TestCase):
    def setUp(self):
        self.directory = tempfile.TemporaryDirectory()
        self.addCleanup(self.directory.cleanup)
        self.root = Path(self.directory.name)
        self.summarizer = gate.load_summarizer()
        self.summarizer.summarize_run_dirs = Mock(return_value={
            "turn_files": 1,
            "round_decision_envelope_source_counts": {"planner_round_action": 1},
        })

    def turn(self, directory, name="case_001/final.json"):
        path = directory / name
        path.parent.mkdir(parents=True, exist_ok=True)
        path.write_text("{}", encoding="utf-8")
        return directory

    def invoke(self, directories):
        output = io.StringIO()
        with patch.object(sys, "argv", ["trace-gate", *map(str, directories)]), \
                patch.object(gate, "load_summarizer", return_value=self.summarizer), \
                redirect_stdout(output):
            result = gate.main()
        return result, output.getvalue()

    def test_empty_directory_cannot_hide_behind_a_valid_directory(self):
        valid = self.turn(self.root / "valid")
        empty = self.root / "empty"
        empty.mkdir()
        result, output = self.invoke([valid, empty])
        self.assertEqual(result, 1)
        self.assertIn(f"missing_turn_files={empty}", output)
        self.summarizer.summarize_run_dirs.assert_not_called()

    def test_wrong_parent_does_not_silently_skip_nested_runs(self):
        parent = self.root / "results"
        self.turn(parent / "dated-run")
        result, output = self.invoke([parent])
        self.assertEqual(result, 1)
        self.assertIn(f"missing_turn_files={parent}", output)

    def test_supported_artifact_layouts_keep_all_runs_without_deduplication(self):
        for name in ["case_001/final.json", "turn_001_case_002.json", "turn_001.json"]:
            with self.subTest(name=name):
                directory = self.turn(self.root / name.replace("/", "_"), name)
                result, output = self.invoke([directory])
                self.assertEqual(result, 0)
                self.assertIn("AGENT_LOOP_TRACE_RELEASE_GATE ok", output)
                self.summarizer.summarize_run_dirs.assert_called_with(
                    [directory], 3, dedupe_latest_case=False,
                )

    def test_missing_directory_still_fails_explicitly(self):
        with self.assertRaises(SystemExit):
            self.invoke([self.root / "missing"])


if __name__ == "__main__":
    result = unittest.main(exit=False).result
    if result.wasSuccessful():
        print("AGENT_LOOP_TRACE_INPUT_SELF_TEST ok")
    raise SystemExit(0 if result.wasSuccessful() else 1)
