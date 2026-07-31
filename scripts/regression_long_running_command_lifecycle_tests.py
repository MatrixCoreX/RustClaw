#!/usr/bin/env python3
"""Unit tests for the long-running lifecycle regression harness."""

from __future__ import annotations

from contextlib import redirect_stdout
import importlib.util
import io
import json
from pathlib import Path
import tempfile
import unittest


SCRIPT_PATH = Path(__file__).with_name("regression_long_running_command_lifecycle.py")


def load_harness():
    spec = importlib.util.spec_from_file_location("long_running_lifecycle_harness", SCRIPT_PATH)
    assert spec and spec.loader
    module = importlib.util.module_from_spec(spec)
    spec.loader.exec_module(module)
    return module


class CliContractTest(unittest.TestCase):
    @classmethod
    def setUpClass(cls) -> None:
        cls.harness = load_harness()

    def test_list_cases_has_no_runtime_side_effect(self) -> None:
        output = io.StringIO()
        with redirect_stdout(output):
            status = self.harness.main(["--list-cases"])

        self.assertEqual(status, 0)
        self.assertEqual(output.getvalue().splitlines(), list(self.harness.CASE_IDS))

    def test_unknown_argument_fails_during_parse(self) -> None:
        with self.assertRaises(SystemExit) as raised:
            self.harness.parse_args(["--unknown-option"])

        self.assertEqual(raised.exception.code, 2)

    def test_explicit_binary_log_dir_and_no_build_are_parsed(self) -> None:
        args = self.harness.parse_args(
            [
                "--no-build",
                "--binary",
                "/tmp/example-clawd",
                "--log-dir",
                "/tmp/example-evidence",
            ]
        )

        self.assertTrue(args.no_build)
        self.assertEqual(args.binary, Path("/tmp/example-clawd"))
        self.assertEqual(args.log_dir, Path("/tmp/example-evidence"))


class SummaryContractTest(unittest.TestCase):
    @classmethod
    def setUpClass(cls) -> None:
        cls.harness = load_harness()

    def test_final_summary_contains_traceability_and_redaction_contract(self) -> None:
        original_log_dir = self.harness.LOG_DIR
        original_binary = self.harness.CLAWD_BIN
        original_auto_build = self.harness.AUTO_BUILD
        try:
            with tempfile.TemporaryDirectory() as directory:
                root = Path(directory)
                binary = root / "clawd"
                binary.write_bytes(b"test-binary")
                binary.chmod(0o755)
                self.harness.LOG_DIR = root / "evidence"
                self.harness.LOG_DIR.mkdir()
                self.harness.CLAWD_BIN = binary
                self.harness.AUTO_BUILD = False
                self.harness.write_json(
                    self.harness.LOG_DIR / "case" / "final.json",
                    {"status": "safe"},
                )
                summary = {
                    "status": "pass",
                    "submission_mode": "direct_run_skill",
                    "cases": {"case": {"status": "pass"}},
                }

                self.assertTrue(
                    self.harness.finalize_summary(
                        summary,
                        "2026-07-31T00:00:00Z",
                        "isolated-admin-key-value",
                    )
                )
                persisted = json.loads(
                    (self.harness.LOG_DIR / "summary.json").read_text(encoding="utf-8")
                )

            self.assertEqual(persisted["schema_version"], 1)
            self.assertTrue(persisted["source_commit"])
            self.assertIn(persisted["worktree"]["status"], {"clean", "dirty"})
            self.assertTrue(persisted["binary"]["sha256"])
            self.assertIn("tree_sha256", persisted["ui"])
            self.assertEqual(
                persisted["case_counts"],
                {"total": 1, "passed": 1, "failed": 0, "unrecorded_failed": 0},
            )
            self.assertEqual(persisted["build_strategy"]["cargo_environment"], "existing_binary")
            self.assertEqual(persisted["redaction"]["status"], "pass")
            self.assertIn("case/final.json", persisted["evidence_relative_paths"])
            self.assertIn("summary.json", persisted["evidence_relative_paths"])
        finally:
            self.harness.LOG_DIR = original_log_dir
            self.harness.CLAWD_BIN = original_binary
            self.harness.AUTO_BUILD = original_auto_build

    def test_secret_scan_reports_known_values_without_echoing_them(self) -> None:
        original_log_dir = self.harness.LOG_DIR
        try:
            with tempfile.TemporaryDirectory() as directory:
                self.harness.LOG_DIR = Path(directory)
                secret = "isolated-admin-key-value"
                (self.harness.LOG_DIR / "artifact.txt").write_text(
                    f"unexpected={secret}",
                    encoding="utf-8",
                )
                findings = self.harness.scan_evidence([secret])

            self.assertEqual(
                findings,
                [{"path": "artifact.txt", "kind": "known_secret_value"}],
            )
            self.assertNotIn(secret, json.dumps(findings))
        finally:
            self.harness.LOG_DIR = original_log_dir


if __name__ == "__main__":
    unittest.main()
