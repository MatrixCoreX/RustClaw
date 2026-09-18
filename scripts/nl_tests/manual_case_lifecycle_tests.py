"""Provider checkpoints pause acceptance without replaying partially executed work."""
import copy
from http.server import BaseHTTPRequestHandler, ThreadingHTTPServer
import json
import os
from pathlib import Path
import subprocess
import tempfile
import threading
import unittest


def waiting_data():
    return {"status": "running", "result_json": {}, "lifecycle": {
        "state": "waiting", "checkpoint_id": "fixture-checkpoint",
        "provider_blocker_active": True,
        "provider_status": {"external_provider_blocked": True, "status_code": "quota_exhausted"}}}


class LifecycleTests(unittest.TestCase):
    def test_pending_resume_does_not_treat_retained_blocker_as_a_new_failure(self):
        from manual_case_lifecycle import poll_status
        data = waiting_data()
        data["lifecycle"]["control_request"] = {"kind":"resume", "status":"pending"}
        self.assertEqual(poll_status(data), "running")
        for control in ({"kind":"pause","status":"pending"},
                        {"kind":"resume","status":"applied"}, None, "pending"):
            data["lifecycle"]["control_request"] = control
            self.assertEqual(poll_status(data), "provider_wait")

    def test_only_structured_provider_wait_stops_polling(self):
        from manual_case_lifecycle import poll_status
        data = waiting_data()
        self.assertEqual(poll_status(data), "provider_wait")
        for key, value in (("state", "running"), ("provider_blocker_active", False),
                           ("provider_blocker_active", "true"), ("provider_status", {})):
            other = copy.deepcopy(data)
            other["lifecycle"][key] = value
            self.assertEqual(poll_status(other), "running")

    def test_other_states_and_prose_do_not_become_provider_wait(self):
        from manual_case_lifecycle import poll_status
        for status in ("succeeded", "failed", "canceled", "queued"):
            self.assertEqual(poll_status({**waiting_data(), "status": status}), status)
        self.assertEqual(poll_status({"status": "running", "error_text": "quota_exhausted"}), "running")
        self.assertEqual(poll_status({"status": "running", "lifecycle": {"state": "needs_user"}}), "needs_user")
        self.assertEqual(poll_status({"status": "running", "lifecycle": {"state": "waiting"}}), "running")

    def test_runner_preserves_waiting_task_and_never_resubmits(self):
        posts = []

        class Handler(BaseHTTPRequestHandler):
            def log_message(self, *_args):
                pass

            def send_json(self, data):
                raw = json.dumps({"ok": True, "data": data}).encode()
                self.send_response(200)
                self.send_header("Content-Type", "application/json")
                self.send_header("Content-Length", str(len(raw)))
                self.end_headers()
                self.wfile.write(raw)

            def do_POST(self):
                posts.append(json.loads(self.rfile.read(int(self.headers["Content-Length"]))))
                self.send_json({"task_id": "fixture-waiting-task"})

            def do_GET(self):
                self.send_json(waiting_data() if "/tasks/" in self.path else {})

        server = ThreadingHTTPServer(("127.0.0.1", 0), Handler)
        thread = threading.Thread(target=server.serve_forever)
        thread.start()
        try:
            with tempfile.TemporaryDirectory(prefix="nl-lifecycle-test-") as directory:
                root = Path(directory)
                cases = root / "cases.txt"
                cases.write_text("fixture|first|requires_tool_call=true|fixture request\n"
                                 "fixture|second|requires_tool_call=true|another fixture request\n")
                script = Path(__file__).with_name("run_manual_test.sh").resolve()
                result = subprocess.run(["bash", str(script), "--case-file", str(cases),
                    "--log-root", str(root / "logs"), "--base-url", f"http://127.0.0.1:{server.server_port}",
                    "--user-key", "fixture-key", "--wait-seconds", "1", "--provider-retries", "2",
                    "--fail-fast", "0", "--no-llm-trace"], capture_output=True, text=True, timeout=20,
                    env={**os.environ, "NO_PROXY": "127.0.0.1,localhost", "no_proxy": "127.0.0.1,localhost"})
                summary = next((root / "logs").glob("*/summary.jsonl"))
                rows = [json.loads(line) for line in summary.read_text().splitlines()]
                self.assertEqual(rows[0]["status"], "provider_unavailable", result.stdout[-2000:])
                self.assertEqual(result.returncode, 1)
                self.assertEqual(len(posts), 1)
                self.assertEqual(len(rows), 1)
                final = json.loads(next((root / "logs").glob("*/case_*/final.json")).read_text())
                self.assertEqual(final["data"], waiting_data())
        finally:
            server.shutdown()
            thread.join()
            server.server_close()


if __name__ == "__main__":
    unittest.main()
