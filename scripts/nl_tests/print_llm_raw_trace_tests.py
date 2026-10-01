#!/usr/bin/env python3
"""Raw model metadata tests, including streamed provider responses."""
import json
import io
import unittest
from contextlib import redirect_stdout
from pathlib import Path

from print_llm_raw_trace import (
    main,
    print_row,
    read_state,
    raw_response_metadata,
    row_fingerprint,
    write_state,
)


class RawResponseMetadataTests(unittest.TestCase):
    def test_numbered_trace_preserves_parent_child_attribution(self):
        output = io.StringIO()
        with redirect_stdout(output):
            print_row({"task_id":"child-a", "parent_task_id":"parent-a",
                       "child_task_id":"child-a", "logical_call_index":3},
                      2, Path("model_io.log"), 10, 1000, "")
        text = output.getvalue()
        self.assertIn("[LLM#2]", text)
        self.assertIn("parent_task_id=parent-a", text)
        self.assertIn("child_task_id=child-a", text)
        self.assertIn("logical_call_index=3", text)

    def test_public_stream_capture_keeps_terminal_metadata_before_truncated_prefix(self):
        header = {"record_type":"public_stream_evidence", "schema_version":1,
                  "terminal":{"choices":[{"finish_reason":"tool_calls"}],
                              "usage":{"total_tokens":42}}}
        raw = json.dumps(header) + '\n{"choices":[{"delta":{"content":"incomplete...(truncated)'
        self.assertEqual(raw_response_metadata(raw), ("tool_calls", {"total_tokens":42}))

    def test_plain_response(self):
        self.assertEqual(raw_response_metadata(json.dumps({
            "choices": [{"finish_reason": "stop"}], "usage": {"total_tokens": 9},
        })), ("stop", {"total_tokens": 9}))

    def test_jsonl_trailing_usage_and_null(self):
        rows = [
            {"choices": [{"finish_reason": None}], "usage": None},
            {"choices": [{"finish_reason": "tool_calls"}]},
            {"choices": [], "usage": {"total_tokens": 17}},
            {"choices": [], "usage": None},
        ]
        self.assertEqual(raw_response_metadata("\n".join(map(json.dumps, rows))),
                         ("tool_calls", {"total_tokens": 17}))

    def test_sse_comments_done_and_invalid_lines(self):
        raw = ': heartbeat\n\ndata: {"choices":[{"finish_reason":"length"}]}\n\ndata: [DONE]\nmalformed'
        self.assertEqual(raw_response_metadata(raw), ("length", None))

    def test_no_inference_from_prose(self):
        for raw in [None, {}, "stop: tool_calls", '{"text":"finish_reason=stop"}']:
            with self.subTest(raw=raw):
                self.assertEqual(raw_response_metadata(raw), (None, None))

    def test_state_retains_bounded_row_fingerprints_across_log_rotation(self):
        import tempfile

        raw_line = json.dumps({"task_id": "task-a", "logical_call_index": 1})
        fingerprint = row_fingerprint(raw_line)
        with tempfile.TemporaryDirectory() as raw_tmp:
            state_path = Path(raw_tmp) / "trace-state.json"
            write_state(state_path, 120, 2, "task-a", [fingerprint])
            state = read_state(state_path)

        self.assertEqual(state["offset"], 120)
        self.assertEqual(state["next_index"], 2)
        self.assertEqual(state["active_task_id"], "task-a")
        self.assertEqual(state["seen_row_hashes"], [fingerprint])

    def test_rotated_log_skips_copied_row_and_prints_new_row_once(self):
        import tempfile

        first = json.dumps({
            "task_id": "task-a",
            "logical_call_index": 1,
            "clean_response": "first",
        })
        second = json.dumps({
            "task_id": "task-a",
            "logical_call_index": 2,
            "clean_response": "second",
        })
        unrelated_padding = json.dumps({"task_id": "other", "data": "x" * 4096})
        with tempfile.TemporaryDirectory() as raw_tmp:
            root = Path(raw_tmp)
            log_path = root / "model_io.log"
            state_path = root / "trace-state.json"
            log_path.write_text(first + "\n" + unrelated_padding + "\n", encoding="utf-8")

            first_output = io.StringIO()
            with redirect_stdout(first_output):
                self.assertEqual(main([
                    "--log", str(log_path),
                    "--task-id", "task-a",
                    "--state-file", str(state_path),
                ]), 0)

            log_path.write_text(first + "\n" + second + "\n", encoding="utf-8")
            rotated_output = io.StringIO()
            with redirect_stdout(rotated_output):
                self.assertEqual(main([
                    "--log", str(log_path),
                    "--task-id", "task-a",
                    "--state-file", str(state_path),
                ]), 0)

        self.assertIn("[LLM#1]", first_output.getvalue())
        self.assertNotIn("[LLM#1]", rotated_output.getvalue())
        self.assertEqual(rotated_output.getvalue().count("[LLM#2]"), 1)
        self.assertIn("response_text=second", rotated_output.getvalue())


if __name__ == "__main__":
    unittest.main()
