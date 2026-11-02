"""Optional, public-only account evidence from the skill-owned browser connection."""
import json
import os
from pathlib import Path
import time
import uuid
from urllib.parse import urlsplit

from account_sessions import DOMAINS, public_snapshot, task_use_file, write_snapshot

class AccountObserver:
    def __init__(self, root: Path, request_id: str):
        self.root, self.request_id = root, request_id
        self.requests = {}
        self.pending_credentials = {}
        self.credential_requests = set()
        self.observed_requests = set()
        self.self_responses = []

    @classmethod
    def from_environment(cls):
        root, request_id = os.environ.get("APP_MEDIA_ACCOUNT_ROOT"), os.environ.get("APP_MEDIA_ACCOUNT_REQUEST")
        if not root or not request_id or not Path(root).is_absolute():
            return None
        return cls(Path(root), request_id)

    def observe(self, event: dict):
        params = event.get("params", {})
        if event.get("method") == "Network.requestWillBeSent":
            host = urlsplit(params.get("request", {}).get("url", "")).hostname or ""
            for platform, (domain, _) in DOMAINS.items():
                if host == domain or host.endswith("." + domain):
                    self.requests[params.get("requestId")] = platform
                    pending = self.pending_credentials.pop(params.get("requestId"), None)
                    if pending is not None:
                        self.observe_credentials(platform, pending)
        elif event.get("method") == "Network.requestWillBeSentExtraInfo":
            platform = self.requests.get(params.get("requestId"))
            if not platform:
                self.pending_credentials[params.get("requestId")] = params.get("associatedCookies", [])
                return
            self.observe_credentials(platform, params.get("associatedCookies", []))
        elif event.get("method") == "Network.responseReceived":
            url = params.get("response", {}).get("url", "")
            endpoint = urlsplit(url).path.rstrip("/")
            if endpoint in {"/aweme/v1/web/user/profile/self", "/api/sns/web/v1/user/selfinfo", "/ajax/config", "/passport/web/account/info", "/x/web-interface/nav"}:
                self.self_responses.append((params.get("requestId"), url))
        elif event.get("method") in {"Network.loadingFinished", "Network.loadingFailed"}:
            self.requests.pop(params.get("requestId"), None)
            self.pending_credentials.pop(params.get("requestId"), None)

    def observe_credentials(self, platform, cookies):
        self.observed_requests.add(platform)
        if any(not row.get("blockedReasons") and row.get("cookie", {}).get("name") in DOMAINS[platform][1]
               and bool(row.get("cookie", {}).get("value"))
               for row in cookies if isinstance(row, dict)):
            self.credential_requests.add(platform)

    def flush(self, connection):
        identity = None
        deadline = time.monotonic() + 0.75
        for request_id, url in reversed(self.self_responses):
            if time.monotonic() >= deadline:
                break
            try:
                command = connection.send("Network.getResponseBody", {"requestId": request_id})
                while time.monotonic() < deadline:
                    message = connection.recv(timeout=max(0.01, deadline - time.monotonic()))
                    if message is None:
                        break
                    if message.get("id") == command:
                        body = message.get("result", {}).get("body", "")
                        if len(body) <= 262144 and not message.get("result", {}).get("base64Encoded"):
                            identity = self_account_identity(url, json.loads(body))
                        break
                if identity:
                    break
            except Exception:
                pass
        platforms = set(self.observed_requests)
        if isinstance(identity, dict) and identity.get("platform") in DOMAINS:
            platforms.add(identity["platform"])
        uses = []
        for platform in sorted(platforms):
            name = identity.get("display_name") if isinstance(identity, dict) and identity.get("platform") == platform else None
            authenticated = platform in self.credential_requests
            snapshot = public_snapshot({"status": "verified" if name and authenticated else "saved_session" if authenticated else "unknown",
                                        "display_name": name, "checked_at": int(time.time()),
                                        "session_present": True if authenticated else None}, platform)
            write_snapshot(self.root, snapshot)
            uses.append({"schema_version": 1, "platform": platform,
                         "mode": "authenticated_browser" if authenticated else "unknown",
                         "account": snapshot, "evidence": "request_cookie_attached" if authenticated else "browser_observation_only"})
        if not uses:
            return
        destination = task_use_file(self.root, self.request_id)
        if destination.parent.is_symlink() or destination.is_symlink():
            return
        destination.parent.mkdir(mode=0o700, parents=True, exist_ok=True)
        previous = []
        try:
            previous = json.loads(destination.read_text())
        except (OSError, ValueError):
            pass
        previous = previous if isinstance(previous, list) else []
        merged = {row["platform"]: row for row in previous if isinstance(row, dict) and row.get("platform") in DOMAINS}
        merged.update({row["platform"]: row for row in uses})
        temporary = destination.with_suffix("." + uuid.uuid4().hex + ".tmp")
        with temporary.open("w", encoding="utf-8") as stream:
            json.dump(list(merged.values()), stream, ensure_ascii=False)
        temporary.chmod(0o600)
        temporary.replace(destination)


def self_account_identity(url: str, value: dict):
    if not isinstance(value, dict):
        return None
    data = value.get("data")
    data = data if isinstance(data, dict) else {}
    parsed = urlsplit(url)
    endpoint = parsed.path.rstrip("/")
    for platform, (domain, _) in DOMAINS.items():
        if not (parsed.hostname == domain or (parsed.hostname or "").endswith("." + domain)):
            continue
        user = None
        if platform == "douyin" and endpoint == "/aweme/v1/web/user/profile/self":
            user = value.get("user")
        if platform == "xiaohongshu" and endpoint == "/api/sns/web/v1/user/selfinfo":
            user = data.get("basic_info") or data
        if platform == "weibo" and endpoint == "/ajax/config":
            user = data.get("user")
        if platform == "tiktok" and endpoint == "/passport/web/account/info":
            user = data
        if platform == "bilibili" and endpoint == "/x/web-interface/nav" and data.get("isLogin") is True:
            user = data
        if isinstance(user, dict):
            name = user.get("nickname") or user.get("screen_name") or user.get("uname")
            if isinstance(name, str) and name.strip() and len(name) <= 256 and not any(ord(c) < 32 or ord(c) == 127 for c in name):
                return {"platform": platform, "display_name": name.strip()}
    return None
