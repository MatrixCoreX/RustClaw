"""Retained-task acceptance must never submit or overwrite the original case."""
from contextlib import contextmanager
from http.server import BaseHTTPRequestHandler, ThreadingHTTPServer
import hashlib
import json
import os
from pathlib import Path
import subprocess
import tempfile
import threading
import unittest


TASK_ID = "00000000-0000-4000-8000-000000000001"
PROMPT = "fixture request"
SCRIPT = Path(__file__).with_name("run_manual_test.sh")


def write_source(root):
    source = root / "original"
    case = source / "case_001_line_001_first"
    case.mkdir(parents=True)
    (case / "submit.json").write_text(json.dumps({"ok": True, "data": {"task_id": TASK_ID}}))
    (case / "final.json").write_text(json.dumps({"data": {
        "task_id": TASK_ID, "status": "running", "result_json": {},
        "lifecycle": {"state": "waiting"}}}))
    row = {"task_id": TASK_ID, "case_name": "first", "source_line": 1,
           "prompt": PROMPT, "tags": "requires_tool_call=false", "mode": "ask",
           "expect_substr": "json_eq:/data/status=succeeded", "status": "provider_unavailable"}
    (source / "summary.jsonl").write_text(json.dumps(row) + "\n")
    (root / "cases.txt").write_text(
        f"fixture|first|requires_tool_call=false|{PROMPT}|expect=json_eq:/data/status=succeeded\n")
    return case


def digests(root):
    return {str(path.relative_to(root)): hashlib.sha256(path.read_bytes()).hexdigest()
            for path in root.rglob("*") if path.is_file()}


@contextmanager
def server(replies):
    events = {"posts": [], "queries": []}

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
            events["posts"].append(self.path)
            self.send_json({"task_id": TASK_ID})

        def do_GET(self):
            if "/tasks/" not in self.path:
                self.send_json({})
                return
            events["queries"].append(self.path)
            self.send_json(replies[min(len(events["queries"]) - 1, len(replies) - 1)])

    http = ThreadingHTTPServer(("127.0.0.1", 0), Handler)
    thread = threading.Thread(target=http.serve_forever)
    thread.start()
    try:
        yield f"http://127.0.0.1:{http.server_port}", events
    finally:
        http.shutdown()
        thread.join()
        http.server_close()


def terminal(status="succeeded"):
    return {"task_id": TASK_ID, "status": status,
            "result_json": {"text": "fixture output"}, "error_text": None}


def invoke(root, source, base, *extra):
    return subprocess.run(["bash", str(SCRIPT), "--case-file", str(root / "cases.txt"),
        "--case-name", "first", "--observe-case", str(source),
        "--log-root", str(root / "observations"), "--base-url", base,
        "--user-key", "fixture-key", "--wait-seconds", "3", "--poll-seconds", "1",
        "--provider-retries", "2", "--no-llm-trace", *extra],
        capture_output=True, text=True, timeout=20,
        env={**os.environ, "NO_PROXY": "127.0.0.1,localhost", "no_proxy": "127.0.0.1,localhost"})


class ObservationTests(unittest.TestCase):
    def test_observes_original_task_until_success_without_any_post(self):
        with tempfile.TemporaryDirectory(prefix="nl-observation-") as directory:
            root = Path(directory)
            case = write_source(root)
            before = digests(case.parent)
            with server([{"task_id": TASK_ID, "status": "running"}, terminal()]) as (base, events):
                result = invoke(root, case, base)
            self.assertEqual(result.returncode, 0, result.stdout[-2000:] + result.stderr)
            self.assertEqual(events["posts"], [])
            self.assertEqual(events["queries"], [f"/v1/tasks/{TASK_ID}"] * 2)
            self.assertEqual(digests(case.parent), before)
            summary = next((root / "observations").glob("*/summary.jsonl"))
            row = json.loads(summary.read_text())
            self.assertEqual(row["task_id"], TASK_ID)
            self.assertEqual(row["assertion"], "pass")
            evidence = json.loads(next(summary.parent.glob("case_*/observation.json")).read_text())
            self.assertEqual(evidence["task_id"], TASK_ID)
            self.assertEqual(evidence["mode"], "existing_task_readonly")
            self.assertEqual(evidence["source_sha256"]["summary.jsonl"], before["summary.jsonl"])

    def test_failed_task_and_provider_wait_never_become_success_or_resubmit(self):
        for reply, expected in [(terminal("failed"), "failed"), ({
                "task_id": TASK_ID, "status": "running", "result_json": {},
                "lifecycle": {"state": "waiting", "provider_blocker_active": True,
                              "provider_status": {"external_provider_blocked": True}}}, "provider_unavailable")]:
            with self.subTest(expected=expected), tempfile.TemporaryDirectory() as directory:
                root = Path(directory)
                case = write_source(root)
                with server([reply]) as (base, events):
                    result = invoke(root, case, base)
                self.assertEqual(result.returncode, 1, result.stdout[-2000:] + result.stderr)
                self.assertEqual(events["posts"], [])
                row = json.loads(next((root / "observations").glob("*/summary.jsonl")).read_text())
                self.assertEqual(row["status"], expected)
                self.assertEqual(row["assertion"], "fail")

    def test_wrong_original_binding_is_rejected_before_task_queries(self):
        for changed in ("prompt", "task_id", "tags", "expect_substr", "case_name", "source_line"):
            with self.subTest(field=changed), tempfile.TemporaryDirectory() as directory:
                root = Path(directory)
                case = write_source(root)
                file = case.parent / "summary.jsonl"
                row = json.loads(file.read_text())
                row[changed] = "changed"
                file.write_text(json.dumps(row) + "\n")
                with server([terminal()]) as (base, events):
                    result = invoke(root, case, base)
                self.assertNotEqual(result.returncode, 0)
                self.assertIn("observation_source_binding_invalid", result.stdout + result.stderr)
                self.assertEqual(events, {"posts": [], "queries": []})

    def test_wrong_live_task_identity_cannot_pass(self):
        with tempfile.TemporaryDirectory() as directory:
            root = Path(directory)
            case = write_source(root)
            reply = terminal()
            reply["task_id"] = "00000000-0000-4000-8000-000000000002"
            with server([reply]) as (base, events):
                result = invoke(root, case, base)
            self.assertNotEqual(result.returncode, 0)
            self.assertIn("observation_reply_identity_invalid", result.stdout + result.stderr)
            self.assertEqual(events["posts"], [])

    def test_observation_cannot_append_over_the_source_run(self):
        with tempfile.TemporaryDirectory() as directory:
            root = Path(directory)
            case = write_source(root)
            before = digests(case.parent)
            with server([terminal()]) as (base, events):
                result = invoke(root, case, base, "--resume-dir", str(case.parent), "--resume-line", "0")
            self.assertNotEqual(result.returncode, 0)
            self.assertEqual(events, {"posts": [], "queries": []})
            self.assertEqual(digests(case.parent), before)

    def test_ambiguous_rows_mismatched_ids_symlinks_and_confirmation_are_rejected(self):
        for defect in ("duplicate_summary", "final_identity", "submit_identity", "source_symlink", "confirmation"):
            with self.subTest(defect=defect), tempfile.TemporaryDirectory() as directory:
                root = Path(directory)
                case = write_source(root)
                if defect == "duplicate_summary":
                    file = case.parent / "summary.jsonl"
                    file.write_text(file.read_text() * 2)
                elif defect in ("final_identity", "submit_identity"):
                    file = case / ("final.json" if defect == "final_identity" else "submit.json")
                    data = json.loads(file.read_text())
                    data["data"]["task_id"] = "../another-task"
                    file.write_text(json.dumps(data))
                elif defect == "source_symlink":
                    file = case / "submit.json"
                    moved = root / "original-submit.json"
                    file.rename(moved)
                    file.symlink_to(moved)
                else:
                    file = root / "cases.txt"
                    file.write_text(file.read_text().rstrip() + ";confirm:fixture confirmation\n")
                with server([terminal()]) as (base, events):
                    result = invoke(root, case, base)
                self.assertNotEqual(result.returncode, 0)
                self.assertEqual(events, {"posts": [], "queries": []})

    def test_duplicate_selected_case_names_are_rejected(self):
        with tempfile.TemporaryDirectory() as directory:
            root = Path(directory)
            case = write_source(root)
            file = root / "cases.txt"
            file.write_text(file.read_text() * 2)
            with server([terminal()]) as (base, events):
                result = invoke(root, case, base)
            self.assertNotEqual(result.returncode, 0)
            self.assertEqual(events, {"posts": [], "queries": []})

    def test_each_observation_has_new_evidence_and_preserves_the_original(self):
        with tempfile.TemporaryDirectory() as directory:
            root = Path(directory)
            case = write_source(root)
            before = digests(case.parent)
            with server([terminal()]) as (base, events):
                for _ in range(2):
                    result = invoke(root, case, base)
                    self.assertEqual(result.returncode, 0, result.stdout[-2000:] + result.stderr)
            self.assertEqual(events["posts"], [])
            self.assertEqual(len(list((root / "observations").glob("*/summary.jsonl"))), 2)
            self.assertEqual(digests(case.parent), before)


if __name__ == "__main__":
    unittest.main()
