import json
from pathlib import Path
import tempfile
import unittest
from unittest.mock import patch

from manual_trace_evidence import restore_execution_streams, trace_hash


class TraceEvidenceTests(unittest.TestCase):
    def fixture(self, root):
        path = root / "run/cases/date/case_sample/final.json"
        path.parent.mkdir(parents=True)
        streams = {"step_results": [{"step": n, "status": "ok"} for n in range(12)],
                   "capability_results": [{"step": n} for n in range(12)]}
        trace = {name: items[:8] for name, items in streams.items()}
        trace["trace_storage"] = {"truncated": True,
            "evidence_streams": {name: trace_hash(items) for name, items in streams.items()}}
        final = {"data": {"result_json": {"task_journal": {"trace": trace}}}}
        log = root / "run/server.log"
        return path, streams, final, log

    def line(self, streams, task="task", prefix="task_call:", phase="finalize"):
        return f"2026-09-15T08:00:00Z  INFO {prefix} task_journal_summary task_id={task} kind=ask phase={phase} " + json.dumps({
            "task_id": task, "trace": {**streams, "ask_state_transitions": [{"to":"completed"}]}}) + "\n"

    def test_restores_later_steps_without_mutating_final_or_trusting_lifecycle(self):
        with tempfile.TemporaryDirectory() as directory:
            path, streams, final, log = self.fixture(Path(directory))
            log.write_text(self.line(streams))
            restored, detail = restore_execution_streams(final, path, "task")
            self.assertTrue(detail["ok"])
            self.assertEqual(detail["counts"]["step_results"], 12)
            self.assertEqual(restored["data"]["result_json"]["task_journal"]["trace"]["step_results"], streams["step_results"])
            self.assertEqual(len(final["data"]["result_json"]["task_journal"]["trace"]["step_results"]), 8)
            self.assertNotIn("ask_state_transitions", restored["data"]["result_json"]["task_journal"]["trace"])

    def test_wrong_task_altered_stream_model_log_partial_and_early_snapshot_rejected(self):
        for variant in range(6):
            with self.subTest(variant=variant), tempfile.TemporaryDirectory() as directory:
                path, streams, final, log = self.fixture(Path(directory))
                if variant == 0:
                    streams["step_results"][-1]["status"] = "error"
                if variant == 1:
                    streams["capability_results"].pop()
                line = self.line(streams, task="other" if variant == 2 else "task",
                                 prefix="model_io:" if variant == 3 else "task_call:",
                                 phase="step_execute" if variant == 4 else "finalize")
                log.write_text(line.rstrip("\n") if variant == 5 else line)
                restored, detail = restore_execution_streams(final, path, "task")
                self.assertFalse(detail["ok"])
                self.assertEqual(restored, final)

    def test_missing_log_cannot_pass_and_legacy_projection_is_not_rewritten(self):
        with tempfile.TemporaryDirectory() as directory:
            path, _, final, _ = self.fixture(Path(directory))
            self.assertFalse(restore_execution_streams(final, path, "task")[1]["ok"])
            del final["data"]["result_json"]["task_journal"]["trace"]["trace_storage"]["evidence_streams"]
            self.assertEqual(restore_execution_streams(final, path, "task"), (final, None))

    def test_wrapper_timestamped_log_is_verified(self):
        with tempfile.TemporaryDirectory() as directory:
            path, streams, final, log = self.fixture(Path(directory))
            log = log.with_name("clawd_full_nl_20260915_120000.log")
            log.write_text(self.line(streams))
            self.assertTrue(restore_execution_streams(final, path, "task")[1]["ok"])
            streams["step_results"].pop()
            log.write_text(self.line(streams))
            self.assertFalse(restore_execution_streams(final, path, "task")[1]["ok"])

    def test_background_terminal_log_without_foreground_span_requires_same_hashes(self):
        for phase in ("finalize", "failure"):
            with self.subTest(phase=phase), tempfile.TemporaryDirectory() as directory:
                path, streams, final, log = self.fixture(Path(directory))
                log.write_text(self.line(streams, prefix="", phase=phase))
                self.assertTrue(restore_execution_streams(final, path, "task")[1]["ok"])
                streams["capability_results"].pop()
                log.write_text(self.line(streams, prefix="", phase=phase))
                self.assertFalse(restore_execution_streams(final, path, "task")[1]["ok"])

    def test_log_symlink_is_not_trusted(self):
        with tempfile.TemporaryDirectory() as directory:
            root = Path(directory)
            path, streams, final, log = self.fixture(root)
            outside = root / "outside.log"
            outside.write_text(self.line(streams))
            log.symlink_to(outside)
            self.assertFalse(restore_execution_streams(final, path, "task")[1]["ok"])

    def test_operator_log_requires_same_task_and_matching_stream_digests(self):
        with tempfile.TemporaryDirectory() as directory:
            root = Path(directory)
            path, streams, final, _ = self.fixture(root)
            log = root / "deployed.log"
            with patch.dict("os.environ", {"NL_RUNTIME_TRACE_LOG": str(log)}):
                log.write_text(self.line(streams))
                self.assertTrue(restore_execution_streams(final, path, "task")[1]["ok"])
                log.write_text(self.line(streams, task="different"))
                self.assertFalse(restore_execution_streams(final, path, "task")[1]["ok"])
                streams["step_results"].pop()
                log.write_text(self.line(streams))
                self.assertFalse(restore_execution_streams(final, path, "task")[1]["ok"])

    def test_operator_log_symlink_is_rejected(self):
        with tempfile.TemporaryDirectory() as directory:
            root = Path(directory)
            path, streams, final, log = self.fixture(root)
            target = root / "deployed.log"
            target.write_text(self.line(streams))
            log = root / "link.log"
            log.symlink_to(target)
            with patch.dict("os.environ", {"NL_RUNTIME_TRACE_LOG": str(log)}):
                self.assertFalse(restore_execution_streams(final, path, "task")[1]["ok"])


if __name__ == "__main__":
    unittest.main()
