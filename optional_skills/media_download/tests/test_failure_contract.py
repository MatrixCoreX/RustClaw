import io
import json
from pathlib import Path
import sys
import unittest
import urllib.error
from contextlib import redirect_stderr

TOOL_DIR = Path(__file__).parents[1] / "src" / "tool"
sys.path.insert(0, str(TOOL_DIR))
from download_failure import ERROR_PREFIX, emit_error, error_metadata, read_error
sys.path.remove(str(TOOL_DIR))


class FailureContractTest(unittest.TestCase):
    def test_http_rate_limit_survives_wrapping(self):
        cause = urllib.error.HTTPError("https://example.test", 429, "limited", {"Retry-After": "42"}, None)
        error = RuntimeError("all candidates failed")
        error.__cause__ = cause
        output = io.StringIO()
        with redirect_stderr(output):
            emit_error(error)
        result = read_error(output.getvalue())
        self.assertEqual(result["error_code"], "rate_limited")
        self.assertTrue(result["retryable"])
        self.assertEqual(result["status_code"], 429)
        self.assertEqual(result["retry_after_seconds"], 42)
        cause.close()

    def test_prose_cannot_change_error_class_or_retryability(self):
        for text in ("this action is required", "no media was requested", "timeout", "HTTP 429"):
            self.assertEqual(read_error(text)["error_code"], "execution_failed")
            self.assertFalse(read_error(text)["retryable"])

    def test_exception_types_control_dependency_and_timeout_errors(self):
        self.assertEqual(error_metadata(ModuleNotFoundError("fixture"))["error_code"], "dependency_unavailable")
        self.assertEqual(error_metadata(FileNotFoundError("fixture"))["error_code"], "not_found")
        self.assertEqual(error_metadata(TimeoutError())["error_code"], "request_timeout")

    def test_malformed_frames_are_ignored(self):
        for value in ([], {"schema_version": 1, "error_code": {}, "retryable": True},
                      {"schema_version": 2, "error_code": "rate_limited", "retryable": True}):
            self.assertEqual(read_error(ERROR_PREFIX + json.dumps(value))["error_code"], "execution_failed")
