"""Exercise resume argument admission with health-only HTTP and zero task calls."""
from http.server import BaseHTTPRequestHandler, ThreadingHTTPServer
import json
import os
from pathlib import Path
import subprocess
import tempfile
import threading
import unittest

ROOT = Path(__file__).resolve().parents[2]


class ManualRunnerResumeTests(unittest.TestCase):
    def test_explicit_zero_is_valid_but_omitted_and_negative_resume_are_rejected(self):
        requests = []

        class Handler(BaseHTTPRequestHandler):
            def do_GET(self):
                requests.append(("GET", self.path))
                self.send_response(200 if self.path == "/v1/health" else 500)
                self.send_header("Content-Type", "application/json")
                self.end_headers()
                self.wfile.write(json.dumps({"ok": True, "data": {"worker_state": "running"}}).encode())

            def do_POST(self):
                requests.append(("POST", self.path))
                self.send_error(500)

            def log_message(self, *_args):
                pass

        server = ThreadingHTTPServer(("127.0.0.1", 0), Handler)
        thread = threading.Thread(target=server.serve_forever, daemon=True)
        thread.start()
        try:
            with tempfile.TemporaryDirectory() as directory:
                root = Path(directory)
                cases = root / "cases.txt"
                cases.write_text("fixture|unused|requires_tool_call=false|Fixture input\n")
                command = ["bash", str(ROOT / "scripts/nl_tests/run_manual_test.sh"),
                    "--case-file", str(cases), "--case-name", "not-present",
                    "--resume-dir", str(root / "run"), "--user-key", "fixture-test-key",
                    "--base-url", f"http://127.0.0.1:{server.server_port}", "--wait-seconds", "1"]
                env = {**os.environ, "NL_CLAWD_BIN": str(root / "absent-binary")}
                for flags in ([], ["--resume-line", "-1"]):
                    result = subprocess.run(command + flags, env=env, capture_output=True, text=True, timeout=30)
                    self.assertEqual(result.returncode, 2, result.stdout + result.stderr)
                    self.assertEqual(requests, [])
                result = subprocess.run(command + ["--resume-line", "0"], env=env,
                    capture_output=True, text=True, timeout=30)
                self.assertEqual(result.returncode, 0, result.stdout + result.stderr)
                self.assertEqual(requests, [("GET", "/v1/health")])
                self.assertEqual((root / "run/summary.jsonl").read_text(), "")
        finally:
            server.shutdown()
            thread.join()
            server.server_close()


if __name__ == "__main__":
    unittest.main()
