"""Exercise the real auth CLI using isolated SQLite and fixture credentials."""
from contextlib import closing, contextmanager
from http.server import BaseHTTPRequestHandler, ThreadingHTTPServer
import json
import os
from pathlib import Path
import sqlite3
import subprocess
import tempfile
import threading
import unittest


ROOT = Path(__file__).resolve().parents[2]
KEY = "fixture-auth-admin-credential"
PASSWORD = "fixture-auth-password"


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

    def success(self):
        body = b'{"ok":true,"data":{}}'
        self.send_response(200)
        self.send_header("Content-Length", str(len(body)))
        self.end_headers()
        self.wfile.write(body)


def run_cli(directory, base):
    root = Path(directory)
    config = root / "config.toml"
    database = root / "auth.sqlite"
    config.write_text('[database]\nsqlite_path=' + json.dumps(str(database)) + '\n')
    with closing(sqlite3.connect(database)) as db:
        db.execute("CREATE TABLE auth_keys(user_key TEXT PRIMARY KEY, role TEXT, enabled INTEGER, created_at TEXT, last_used_at TEXT)")
        db.execute("INSERT INTO auth_keys VALUES(?,'admin',1,'1',NULL)", (KEY,))
        db.commit()
    env = {**os.environ, "APP_CONFIG_PATH": str(config), "APP_ADMIN_KEY": KEY,
           "WEBD_PASSWORD": PASSWORD, "BASE_URL": base,
           "NO_PROXY": "127.0.0.1,localhost", "no_proxy": "127.0.0.1,localhost"}
    return subprocess.run(["bash", str(ROOT / "scripts/auth-key.sh"), "webd-set", KEY, "fixture-user"],
                          env=env, text=True, capture_output=True, timeout=10)


class AuthKeyHttpTests(unittest.TestCase):
    def scenario(self, code, same_origin=False):
        forwarded = []
        original = []
        class Target(Handler):
            def do_GET(self):
                forwarded.append(self.headers.get("X-Agent-Key") == KEY)
                self.success()
            def do_POST(self):
                self.rfile.read(int(self.headers.get("Content-Length", "0")))
                self.do_GET()
        with endpoint(Target) as target:
            class Origin(Handler):
                def do_POST(self):
                    original.append((self.path, self.headers.get("X-Agent-Key") == KEY,
                                     json.loads(self.rfile.read(int(self.headers["Content-Length"])))))
                    self.send_response(code)
                    self.send_header("Location", "/target" if same_origin else target + "/target")
                    self.send_header("Content-Length", "0")
                    self.end_headers()
                def do_GET(self):
                    forwarded.append(self.headers.get("X-Agent-Key") == KEY)
                    self.success()
            with endpoint(Origin) as origin, tempfile.TemporaryDirectory() as directory:
                result = run_cli(directory, origin)
        return result, original, forwarded

    def test_cross_origin_302_never_forwards_admin_credentials(self):
        result, original, forwarded = self.scenario(302)
        self.assertEqual(forwarded, [])
        self.assertNotEqual(result.returncode, 0)
        self.assertEqual(len(original), 1)

    def test_all_redirect_codes_same_and_cross_origin_fail_without_replay(self):
        for code in (301, 302, 303, 307, 308):
            for same_origin in (False, True):
                with self.subTest(code=code, same_origin=same_origin):
                    result, original, forwarded = self.scenario(code, same_origin)
                    self.assertEqual(forwarded, [])
                    self.assertEqual(len(original), 1)
                    self.assertNotEqual(result.returncode, 0)
                    self.assertIn(f"HTTP {code}", result.stderr)
                    self.assertNotIn(PASSWORD, result.stdout + result.stderr)

    def test_direct_authenticated_post_still_sets_the_account(self):
        received = []
        class Direct(Handler):
            def do_POST(self):
                received.append((self.path, self.headers.get("X-Agent-Key"),
                                 json.loads(self.rfile.read(int(self.headers["Content-Length"])))))
                self.success()
        with endpoint(Direct) as base, tempfile.TemporaryDirectory() as directory:
            result = run_cli(directory, base)
        self.assertEqual(result.returncode, 0, result.stderr)
        self.assertEqual(received, [("/v1/admin/webd-accounts", KEY,
                                    {"username": "fixture-user", "password": PASSWORD, "user_key": KEY})])
        self.assertNotIn(PASSWORD, result.stdout + result.stderr)


if __name__ == "__main__":
    unittest.main()
