import http.client
import json
import threading
import urllib.parse

from small_screen_config import load_enabled_admin_user_key

API_BASE = "http://127.0.0.1:8787"
_api_http_conn = None
_api_http_lock = threading.Lock()


def _api_host_port():
    url = urllib.parse.urlparse(API_BASE)
    host = url.hostname or "127.0.0.1"
    port = url.port
    if port is None:
        port = 443 if (url.scheme or "http").lower() == "https" else 80
    return host, port


def _api_drop_connection_unlocked():
    global _api_http_conn
    if _api_http_conn is not None:
        try:
            _api_http_conn.close()
        except Exception:
            pass
        _api_http_conn = None


def localhost_api_request(method, path, user_key="", body=None):
    global _api_http_conn
    if not path.startswith("/"):
        path = "/" + path
    headers = {}
    stripped_key = (user_key or "").strip()
    if stripped_key:
        headers["X-RustClaw-Key"] = stripped_key
    if body is not None:
        headers["Content-Type"] = "application/json"
    host, port = _api_host_port()
    last_err = None
    with _api_http_lock:
        for attempt in range(2):
            try:
                if _api_http_conn is None:
                    _api_http_conn = http.client.HTTPConnection(host, port, timeout=8)
                _api_http_conn.request(method, path, body=body, headers=headers)
                resp = _api_http_conn.getresponse()
                data = resp.read()
                if 200 <= resp.status < 300:
                    return data
                last_err = RuntimeError(f"HTTP {resp.status}: {data[:200]!r}")
            except Exception as exc:
                last_err = exc
            _api_drop_connection_unlocked()
            if attempt == 0:
                continue
        raise last_err or RuntimeError("request failed")


def fetch_health(user_key=""):
    try:
        raw = localhost_api_request("GET", "/v1/health", user_key)
        body = json.loads(raw.decode())
        data = (body.get("data") or body) if isinstance(body, dict) else {}
        return data, None
    except Exception as exc:
        return None, str(exc)


def fetch_skills_config(user_key=""):
    try:
        raw = localhost_api_request("GET", "/v1/skills/config", user_key)
        body = json.loads(raw.decode())
        data = (body.get("data") or body) or {}
        all_list = data.get("managed_skills") or data.get("skills_list") or []
        switches = data.get("skill_switches") or {}
        all_names = sorted(set(all_list) | set(switches.keys()))
        enabled_list = data.get("runtime_enabled_skills") or data.get("effective_enabled_skills_preview") or []
        enabled_set = set(enabled_list)
        return all_names, enabled_set
    except Exception:
        return None, None


def post_admin_webd_account(user_key, username, password):
    payload = {
        "username": (username or "").strip(),
        "password": password or "",
        "user_key": (user_key or "").strip(),
    }
    if not payload["username"] or not payload["password"] or not payload["user_key"]:
        return False, "missing username/password/user_key"
    body = json.dumps(payload).encode("utf-8")
    try:
        raw = localhost_api_request("POST", "/v1/admin/webd-accounts", user_key, body=body).decode()
        parsed = json.loads(raw) if raw else {}
        if not isinstance(parsed, dict):
            return False, "invalid response"
        if parsed.get("ok"):
            return True, ""
        return False, str(parsed.get("error") or "request failed")
    except Exception as exc:
        return False, str(exc)


def reset_admin_login_account(username="rustclaw", password="rustclaw123456"):
    admin_key = load_enabled_admin_user_key()
    if not admin_key:
        return False, "enabled admin key not found"
    return post_admin_webd_account(admin_key, username, password)
