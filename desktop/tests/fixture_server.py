"""Loopback-only protocol fixture. Never connects to or changes a real device."""
import hashlib
import http.server
import json
import ssl
import subprocess
import threading
import time
import uuid
from pathlib import Path


class Fixture:
    def __init__(self, directory):
        self.directory = Path(directory)
        self.directory.mkdir(parents=True, exist_ok=True, mode=0o700)
        def openssl(*args):
            subprocess.run(["openssl", *args], cwd=self.directory, check=True, stdout=subprocess.DEVNULL, stderr=subprocess.DEVNULL)
        openssl("req", "-x509", "-newkey", "rsa:2048", "-nodes", "-keyout", "ca.key", "-out", "ca.pem", "-subj", "/CN=Desktop protocol test CA", "-days", "1", "-addext", "basicConstraints=critical,CA:TRUE")
        openssl("req", "-new", "-newkey", "rsa:2048", "-nodes", "-keyout", "leaf.key", "-out", "leaf.csr", "-subj", "/CN=localhost")
        (self.directory / "extensions.cnf").write_text("subjectAltName=DNS:localhost\nbasicConstraints=critical,CA:FALSE\nkeyUsage=digitalSignature,keyEncipherment\nextendedKeyUsage=serverAuth\n")
        openssl("x509", "-req", "-in", "leaf.csr", "-CA", "ca.pem", "-CAkey", "ca.key", "-CAcreateserial", "-out", "leaf.pem", "-days", "1", "-extfile", "extensions.cnf")
        der = ssl.PEM_cert_to_DER_cert((self.directory / "ca.pem").read_text())
        self.fingerprint = hashlib.sha256(der).hexdigest()
        self.pem = (self.directory / "ca.pem").read_text()
        subprocess.run(["ffmpeg", "-v", "error", "-y", "-f", "lavfi", "-i", "testsrc2=size=720x480:rate=30", "-t", "12", "-c:v", "libx264", "-preset", "ultrafast", "-crf", "18", "-pix_fmt", "yuv420p", "-movflags", "+faststart", str(self.directory / "preview.mp4")], check=True)
        self.media = (self.directory / "preview.mp4").read_bytes()
        self.requests = []
        self.ranges = []
        self.tasks = {}
        self.session_cookie = "fixture-session"
        self.csrf_token = uuid.uuid4().hex
        fixture = self

        class Handler(http.server.BaseHTTPRequestHandler):
            protocol_version = "HTTP/1.1"
            def handle(self):
                try:
                    super().handle()
                except (BrokenPipeError, ConnectionResetError, ssl.SSLError):
                    pass  # Expected when a client aborts an in-flight request.
            def log_message(self, *_args):
                pass
            def send_data(self, data, status=200, headers=None):
                headers = dict(headers or {})
                if isinstance(data, (dict, list)):
                    data = json.dumps(data).encode()
                    content_type = "application/json"
                else:
                    content_type = "application/octet-stream"
                self.send_response(status)
                self.send_header("Content-Type", headers.pop("Content-Type", content_type))
                self.send_header("Content-Length", str(len(data)))
                for key, value in headers.items():
                    self.send_header(key, value)
                self.end_headers()
                if self.command != "HEAD":
                    self.wfile.write(data)
            def request_body(self):
                if self.headers.get("Transfer-Encoding") == "chunked":
                    chunks = []
                    while True:
                        size = int(self.rfile.readline().split(b";")[0].strip(), 16)
                        if size == 0:
                            self.rfile.readline()
                            break
                        chunks.append(self.rfile.read(size))
                        self.rfile.read(2)
                    return b"".join(chunks)
                return self.rfile.read(int(self.headers.get("Content-Length", 0)))
            def do_HEAD(self):
                self.do_GET()
            def do_POST(self):
                self.do_GET()
            def do_DELETE(self):
                self.do_GET()
            def do_GET(self):
                path = self.path.split("?", 1)[0]
                body = self.request_body() if self.command in ("POST", "PUT", "PATCH") else b""
                fixture.requests.append((self.command, path, len(body)))
                try:
                    payload = json.loads(body or b"{}")
                except ValueError:
                    payload = {}
                key_ok = self.headers.get("X-Agent-Key") == "fixture-key"
                cookie_ok = "session=" + fixture.session_cookie in self.headers.get("Cookie", "")
                identity = {"user_id": 7, "chat_id": 9, "role": "admin", "user_key": "fixture-key"}
                if path == "/webd/session":
                    if gate := getattr(fixture, "bootstrap_gate", None):
                        gate.wait(timeout=10)
                    return self.send_data({"ok": True, "data": {"logged_in": cookie_ok, "csrf_token": None, "username": None, "role": None}})
                if path == "/webd/login":
                    if payload != {"username": "tester", "password": "fixture-password"} or self.headers.get("Origin") != f"{'https' if self.server is fixture.server else 'http'}://{self.headers.get('Host')}":
                        return self.send_data({"ok": False}, 403)
                    return self.send_data({"ok": True, "data": {"csrf_token": fixture.csrf_token}}, headers={"Set-Cookie": "session=" + fixture.session_cookie + "; Path=/; HttpOnly; SameSite=Lax" + ("; Secure" if self.server is fixture.server else "")})
                if path == "/v1/auth/ui-key/verify":
                    return self.send_data({"ok": payload.get("user_key") == "fixture-key", "data": identity}, 200 if payload.get("user_key") == "fixture-key" else 401)
                if not (key_ok or cookie_ok):
                    return self.send_data({"ok": False, "error": "auth_required"}, 401)
                if self.command not in ("GET", "HEAD") and not key_ok and self.headers.get("X-Agent-Csrf-Token") != fixture.csrf_token:
                    return self.send_data({"ok": False, "error": "csrf_required"}, 403)
                if path.startswith("/v1/nni/assets/owner/") and hasattr(fixture, "owner_api"):
                    return fixture.owner_api.handle(self, payload)
                if hasattr(fixture, "owner_api") and path in ("/v1/nni/bancor/market", "/v1/nni/assets/market", "/v1/nni/bancor/candles", "/v1/nni/bancor/trades"):
                    return self.send_data({"ok": True, "data": fixture.owner_api.market_data(self.path)})
                if path == "/v1/auth/me" or path == "/v1/local/interaction-context":
                    return self.send_data({"ok": True, "data": identity})
                if path == "/v1/aipps":
                    return self.send_data({"ok": True, "data": {"schema_version": 1, "apps": [{"skill_name": "protocol_fixture", "package_version": "1", "renderer": "sandbox_bundle_v1", "data_contract": "capability_bridge_v1", "installed": True, "entrypoint": "index.html", "bridge_capabilities": ["fixture.read"], "titles": {"zh": "协议测试应用"}, "descriptions": {"zh": "隔离边界测试"}, "default_locale": "zh", "icon": "app", "task_channel_scope": None}]}})
                if path.startswith("/v1/aipps/protocol_fixture/assets/"):
                    return self.send_data(b'<!doctype html><html><body><h1 id="fixture-app">Isolated skill fixture</h1><script>window.parent.postMessage({schema_version:1,type:"aipp.ready"},"*")</script></body></html>', headers={"Content-Type": "text/html"})
                if path == "/v1/events":
                    self.send_response(200)
                    self.send_header("Content-Type", "text/event-stream")
                    self.send_header("Transfer-Encoding", "chunked")
                    self.end_headers()
                    try:
                        for chunk in (b"data: first\n\n", b"data: second\n\n"):
                            self.wfile.write(f"{len(chunk):x}\r\n".encode() + chunk + b"\r\n")
                            self.wfile.flush()
                            time.sleep(0.8)
                        self.wfile.write(b"0\r\n\r\n")
                    except (BrokenPipeError, ConnectionResetError, ssl.SSLError):
                        pass  # The cancellation test intentionally closes the stream.
                    return
                if path == "/v1/echo":
                    return self.send_data({"ok": True, "data": {"bytes": len(body)}})
                if path.endswith("/artifacts/file/content"):
                    fixture.ranges.append(self.headers.get("Range"))
                    data = fixture.media
                    bounds = self.headers.get("Range", "bytes=0-").removeprefix("bytes=").split("-")
                    start = int(bounds[0] or 0)
                    end = min(int(bounds[1]) if bounds[1] else len(data) - 1, len(data) - 1)
                    return self.send_data(data[start:end + 1], 206, {"Content-Type":"video/mp4", "Content-Range": f"bytes {start}-{end}/{len(data)}", "Accept-Ranges": "bytes"})
                if path == "/v1/tasks" and self.command == "POST":
                    task_id = "00000000-0000-4000-8000-000000000001"
                    fixture.tasks[task_id] = {"task_id": task_id, "status": "succeeded", "result_json": {"text": "Fixture completed"}, "error_text": None}
                    return self.send_data({"ok": True, "data": {"task_id": task_id}})
                if path in ["/v1/tasks/" + t for t in fixture.tasks]:
                    return self.send_data({"ok": True, "data": fixture.tasks[path.rsplit("/", 1)[1]]})
                if path == "/v1/health":
                    return self.send_data({"ok": True, "data": {"status": "ok", "uptime_seconds": 30, "queue_length": 0, "running_tasks": 0}})
                if path == "/v1/nni/config":
                    return self.send_data({"ok": True, "data": {"joined": False, "remote_nodes": [], "asset_owner_pubkey": "11" * 32}})
                if path == "/v1/config" or path.startswith("/v1/config/"):
                    return self.send_data({"ok": True, "data": {}})
                if path == "/v1/tasks" or path.startswith("/v1/logs"):
                    return self.send_data({"ok": True, "data": {"items": [], "tasks": [], "files": []}})
                if path == "/v1/auth/keys":
                    return self.send_data({"ok": True, "data": {"keys": []}})
                if path.startswith("/v1/tasks/conversation-history"):
                    return self.send_data({"ok": True, "data": {"items": [], "threads": [], "has_more": False}})
                return self.send_data({"ok": False, "error": "fixture_endpoint_unavailable"}, 404)

        self.server = http.server.ThreadingHTTPServer(("127.0.0.1", 0), Handler)
        context = ssl.SSLContext(ssl.PROTOCOL_TLS_SERVER)
        context.load_cert_chain(self.directory / "leaf.pem", self.directory / "leaf.key")
        self.server.socket = context.wrap_socket(self.server.socket, server_side=True)
        self.origin = f"https://localhost:{self.server.server_port}"
        self.thread = threading.Thread(target=self.server.serve_forever, daemon=True)
        self.thread.start()
        self.local_server = http.server.ThreadingHTTPServer(("127.0.0.1", 0), Handler)
        self.local_origin = f"http://127.0.0.1:{self.local_server.server_port}"
        self.local_thread = threading.Thread(target=self.local_server.serve_forever, daemon=True)
        self.local_thread.start()

    def close(self):
        self.server.shutdown()
        self.server.server_close()
        self.local_server.shutdown()
        self.local_server.server_close()
