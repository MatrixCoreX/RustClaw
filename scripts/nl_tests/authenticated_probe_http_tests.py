"""Credential-bearing operator probes must not follow HTTP redirects."""
from contextlib import contextmanager
from http.server import BaseHTTPRequestHandler, ThreadingHTTPServer
import json
import io
import os
from pathlib import Path
import shutil
import sqlite3
import tempfile
import threading
import unittest
import urllib.error
import urllib.request
from unittest.mock import patch

from concurrent_health_probe import probe as health_probe
from probe_runtime_read_latency import probe as read_probe
from authenticated_probe_http import authenticated_opener, authenticated_urlopen
from run_live_instruction_steering_suite import ApiClient
from replay_verifier_candidate import receive_response
from configure_isolated_nl_memory import main as configure_memory


FIXTURE_KEY = "fixture-probe-credential"


@contextmanager
def endpoint(handler):
    server = ThreadingHTTPServer(("127.0.0.1", 0), handler)
    thread = threading.Thread(target=server.serve_forever)
    thread.start()
    try:
        yield f"http://127.0.0.1:{server.server_port}"
    finally:
        server.shutdown()
        thread.join()
        server.server_close()


class Handler(BaseHTTPRequestHandler):
    def log_message(self, *_args):
        pass

    def json_response(self):
        body = b'{"ok":true,"data":{"worker_state":"running"}}'
        self.send_response(200)
        self.send_header("Content-Length", str(len(body)))
        self.end_headers()
        self.wfile.write(body)


def setup(root):
    (root / "configs").mkdir()
    (root / "configs/config.toml").write_text('[database]\nsqlite_path="tasks.sqlite"\n')
    with sqlite3.connect(root / "tasks.sqlite") as db:
        db.executescript("""CREATE TABLE auth_keys(user_key TEXT,role TEXT,enabled INTEGER);
            CREATE TABLE tasks(task_id TEXT,status TEXT,result_json TEXT,
              lease_expires_at INTEGER,updated_at TEXT,created_at TEXT);""")
        db.execute("INSERT INTO auth_keys VALUES(?,'admin',1)", (FIXTURE_KEY,))
    task = root / "task.json"
    task.write_text(json.dumps({"data": {"status": "running", "result_json": {
        "task_checkpoint": {"checkpoint_id": "fixture-checkpoint",
                            "pending_async_job": {"job_id": "fixture-job"}}}}}))
    return task, root / "health.json"


class RedirectTests(unittest.TestCase):
    def scenario(self, mode, code, same_origin=False, html_error=False):
        received = []
        origin_requests = []

        class Target(Handler):
            def do_GET(self):
                received.append(self.headers.get("X-Agent-Key") == FIXTURE_KEY
                                or self.headers.get("Authorization") == "Bearer " + FIXTURE_KEY)
                self.json_response()

        with endpoint(Target) as target:
            class Origin(Handler):
                def do_GET(self):
                    if self.path == "/redirect-target":
                        received.append(self.headers.get("X-Agent-Key") == FIXTURE_KEY)
                        self.json_response()
                        return
                    origin_requests.append(self.headers.get("X-Agent-Key") == FIXTURE_KEY
                                           or self.headers.get("Authorization") == "Bearer " + FIXTURE_KEY)
                    self.send_response(code)
                    self.send_header("Location", "/redirect-target" if same_origin else target + "/target")
                    body = b"<html>fixture proxy error</html>" if html_error else b""
                    self.send_header("Content-Length", str(len(body)))
                    self.end_headers()
                    self.wfile.write(body)

                def do_POST(self):
                    self.rfile.read(int(self.headers.get("Content-Length", "0")))
                    self.do_GET()

            with endpoint(Origin) as origin, tempfile.TemporaryDirectory() as directory, \
                    patch.dict(os.environ, {"NO_PROXY": "127.0.0.1,localhost", "no_proxy": "127.0.0.1,localhost"}):
                root = Path(directory)
                task, output = setup(root)
                before = (root / "tasks.sqlite").read_bytes()
                if mode == "read":
                    result = read_probe(root, 1, origin)
                elif mode == "health":
                    result = health_probe(task, output, origin, FIXTURE_KEY)
                elif mode == "steering":
                    result = ApiClient(origin, FIXTURE_KEY, 3).request("GET", "/health")
                    self.assertEqual(result[0], code)
                    self.assertFalse(result[1]["ok"])
                    if html_error:
                        self.assertEqual(result[1]["error"]["code"], "http_error_response_invalid")
                        self.assertEqual(result[1]["error"]["status_code"], code)
                elif mode == "events":
                    with self.assertRaises(urllib.error.HTTPError) as raised:
                        list(ApiClient(origin, FIXTURE_KEY, 3).task_events("fixture-task", 3))
                    self.assertEqual(raised.exception.code, code)
                    raised.exception.close()
                elif mode == "replay":
                    log = io.StringIO()
                    request = urllib.request.Request(origin + "/v1/chat/completions", data=b"{}",
                        headers={"Authorization": "Bearer " + FIXTURE_KEY})
                    with self.assertRaises(urllib.error.HTTPError) as raised:
                        receive_response(request, log, {"model": "fixture"})
                    raised.exception.close()
                    self.assertEqual(json.loads(log.getvalue())["http_status"], code)
                    self.assertNotIn(FIXTURE_KEY, log.getvalue())
                else:
                    isolated = root / "tmp/agent-runtime-nl-isolated-fixture"
                    isolated.mkdir(parents=True)
                    shutil.copyfile(root / "tasks.sqlite", isolated / "tasks.sqlite")
                    with patch("sys.argv", ["configure-memory", "--isolation-root", str(isolated),
                                            "--base-url", origin]):
                        with self.assertRaises(urllib.error.HTTPError) as raised:
                            configure_memory()
                    self.assertEqual(raised.exception.code, code)
                    raised.exception.close()
                self.assertEqual(received, [], "redirected endpoint received an authenticated probe")
                self.assertEqual(origin_requests, [True])
                self.assertEqual((root / "tasks.sqlite").read_bytes(), before)
                if mode == "read":
                    self.assertEqual(result["health"]["failures"], 1)
                    self.assertNotIn(FIXTURE_KEY, json.dumps(result))
                elif mode == "health":
                    self.assertEqual(result, 1)
                    evidence = output.read_text()
                    self.assertNotIn(FIXTURE_KEY, evidence)
                    self.assertEqual(json.loads(evidence)["error_type"], "HTTPError")

    def test_read_probe_rejects_cross_origin_redirect_before_forwarding_key(self):
        self.scenario("read", 302)

    def test_health_probe_rejects_cross_origin_redirect_before_forwarding_key(self):
        self.scenario("health", 302)

    def test_every_redirect_code_is_rejected_including_same_origin(self):
        for code in (301, 302, 303, 307, 308):
            for mode in ("read", "health"):
                with self.subTest(code=code, mode=mode):
                    self.scenario(mode, code, same_origin=True)

    def test_other_authenticated_api_clients_do_not_forward_credentials(self):
        for mode in ("steering", "events", "replay", "memory"):
            with self.subTest(client=mode):
                self.scenario(mode, 302)

    def test_post_redirect_codes_do_not_replay_the_provider_request(self):
        for code in (301, 302, 303, 307, 308):
            with self.subTest(code=code):
                self.scenario("replay", code)

    def test_http_status_survives_non_json_proxy_error_pages(self):
        for code in (301, 503):
            with self.subTest(code=code):
                self.scenario("steering", code, html_error=True)

    def test_direct_authenticated_responses_still_work(self):
        received = []

        class Direct(Handler):
            def do_GET(self):
                received.append(self.headers.get("X-Agent-Key") == FIXTURE_KEY)
                self.json_response()

        with endpoint(Direct) as url, patch.dict(os.environ, {"NO_PROXY": "127.0.0.1", "no_proxy": "127.0.0.1"}):
            request = urllib.request.Request(url, headers={"X-Agent-Key": FIXTURE_KEY})
            with authenticated_urlopen(request, timeout=3) as response:
                self.assertTrue(json.load(response)["ok"])
        self.assertEqual(received, [True])

    def test_proxy_configuration_is_preserved_except_explicit_local_opt_out(self):
        with patch("urllib.request.build_opener") as build:
            authenticated_opener()
            self.assertFalse(any(isinstance(handler, urllib.request.ProxyHandler)
                                 for handler in build.call_args.args))
            authenticated_opener(disable_proxy=True)
            proxies = [handler.proxies for handler in build.call_args.args
                       if isinstance(handler, urllib.request.ProxyHandler)]
            self.assertEqual(proxies, [{}])


if __name__ == "__main__":
    unittest.main()
