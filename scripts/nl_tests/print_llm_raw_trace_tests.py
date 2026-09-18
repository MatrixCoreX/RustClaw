#!/usr/bin/env python3
"""Raw model metadata tests, including streamed provider responses."""
import json
import unittest

from print_llm_raw_trace import raw_response_metadata


class RawResponseMetadataTests(unittest.TestCase):
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


if __name__ == "__main__":
    unittest.main()
