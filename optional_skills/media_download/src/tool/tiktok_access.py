"""TikTok page state and skill-owned QR login support.

The module keeps TikTok cookies inside media_download private storage.  It
distinguishes an unavailable post from an authentication barrier before an
interactive browser is opened, so deleted/restricted posts never cause a
misleading login prompt.
"""

from __future__ import annotations

import json
import os
import re
import socket
import subprocess
import sys
import time
import urllib.parse
import urllib.request
from pathlib import Path
from typing import Any, Callable

from browser_devtools import DevToolsConnection, DevToolsError
import login_challenge


LOGIN_WAIT_SECONDS = 600.0
LOGIN_POLL_SECONDS = 1.0
LOGIN_READY_POLLS = 2
LOGIN_REQUIRED = "login_required"
CONTENT_AVAILABLE = "content_available"
CONTENT_UNAVAILABLE = "content_unavailable"
ACCESS_UNKNOWN = "access_unknown"
DISPLAY_UNAVAILABLE = "display_unavailable"
INTERACTIVE_TIMEOUT = "interactive_verification_timeout"
INTERACTIVE_CANCELLED = "interactive_verification_cancelled"

_AUTH_COOKIE_NAMES = {
    "sessionid",
    "sessionid_ss",
    "sid_guard",
    "sid_tt",
    "sid_ucp_v1",
}
_UNIVERSAL_DATA_RE = re.compile(
    r'<script[^>]+id=["\']__UNIVERSAL_DATA_FOR_REHYDRATION__["\'][^>]*>(.*?)</script>',
    re.IGNORECASE | re.DOTALL,
)
_LOGIN_ROUTE_RE = re.compile(r"^/(?:login|signup)(?:/|$)", re.IGNORECASE)
_LOGIN_BARRIER_RE = re.compile(
    r'(?:data-e2e|id|class)=["\'][^"\']*(?:login-modal|login-container|login-dialog)[^"\']*["\']',
    re.IGNORECASE,
)


def persistent_profile_dir(browser_profile_root: str | Path | None) -> Path | None:
    if not browser_profile_root:
        return None
    profile = Path(browser_profile_root).expanduser() / "tiktok"
    profile.mkdir(parents=True, exist_ok=True)
    return profile


def _universal_scope(page_text: str) -> dict[str, Any]:
    match = _UNIVERSAL_DATA_RE.search(page_text or "")
    if match is None:
        return {}
    try:
        payload = json.loads(match.group(1))
    except (TypeError, ValueError):
        return {}
    scope = payload.get("__DEFAULT_SCOPE__") if isinstance(payload, dict) else None
    return scope if isinstance(scope, dict) else {}


def classify_page(
    page_text: str,
    url: str,
    expected_item_id: str | None = None,
) -> str:
    """Classify only structural TikTok evidence, never generic page wording."""
    path = urllib.parse.urlsplit(url or "").path
    if _LOGIN_ROUTE_RE.search(path):
        return LOGIN_REQUIRED

    detail = _universal_scope(page_text).get("webapp.video-detail")
    if isinstance(detail, dict):
        item_info = detail.get("itemInfo")
        item = item_info.get("itemStruct") if isinstance(item_info, dict) else None
        item_id = str(item.get("id") or "") if isinstance(item, dict) else ""
        if item_id and (not expected_item_id or item_id == str(expected_item_id)):
            return CONTENT_AVAILABLE
        status_code = detail.get("statusCode")
        # 10204 is TikTok's structural "post unavailable/restricted" result.
        # Other non-zero statuses are intentionally left unknown because they
        # may change meaning across regions and deployments.
        if status_code == 10204:
            return CONTENT_UNAVAILABLE

    # TikTok's normal post shell contains a header login button.  Require a
    # modal/container structure so that button alone cannot trigger login.
    if _LOGIN_BARRIER_RE.search(page_text or ""):
        return LOGIN_REQUIRED
    return ACCESS_UNKNOWN


def needs_skill_owned_login(
    *,
    page_text: str,
    url: str,
    expected_item_id: str | None,
    has_media: bool,
    http_login_barrier: bool = False,
    content_unavailable: bool = False,
) -> bool:
    if has_media or content_unavailable:
        return False
    state = classify_page(page_text, url, expected_item_id)
    return http_login_barrier or state == LOGIN_REQUIRED


def access_outcome_is_terminal(logs: list[str]) -> bool:
    joined = "\n".join(logs)
    return any(
        marker in joined
        for marker in (
            "tiktok: content_unavailable",
            f"tiktok: interactive_login={DISPLAY_UNAVAILABLE}",
            f"tiktok: interactive_login={INTERACTIVE_TIMEOUT}",
            f"tiktok: interactive_login={INTERACTIVE_CANCELLED}",
        )
    )


def desktop_display_available(
    environment: dict[str, str] | None = None,
    host_platform: str | None = None,
) -> bool:
    platform_name = host_platform or sys.platform
    env = os.environ if environment is None else environment
    if platform_name == "darwin":
        return True
    if platform_name.startswith("linux"):
        return bool(env.get("DISPLAY") or env.get("WAYLAND_DISPLAY"))
    return False


def _visible_chrome_args() -> list[str]:
    if not sys.platform.startswith("linux"):
        return []
    args = ["--password-store=basic"]
    if os.environ.get("WAYLAND_DISPLAY"):
        args.append("--ozone-platform=wayland")
    return args


def _devtools_port() -> tuple[int, list[str]]:
    with socket.socket(socket.AF_INET, socket.SOCK_STREAM) as listener:
        listener.bind(("127.0.0.1", 0))
        port = int(listener.getsockname()[1])
    return port, [
        f"--remote-debugging-port={port}",
        "--remote-debugging-address=127.0.0.1",
    ]


def _wait_for_devtools(
    port: int,
    process: subprocess.Popen[Any],
    timeout: float,
    on_tick: Callable[[], None] | None,
) -> str:
    deadline = time.monotonic() + max(1.0, min(timeout, 20.0))
    while time.monotonic() < deadline:
        if on_tick is not None:
            on_tick()
        if process.poll() is not None:
            raise RuntimeError(INTERACTIVE_CANCELLED)
        try:
            with urllib.request.urlopen(f"http://127.0.0.1:{port}/json/list", timeout=2.0) as response:
                targets = json.load(response)
            for target in targets:
                websocket = target.get("webSocketDebuggerUrl") if isinstance(target, dict) else None
                if isinstance(websocket, str) and target.get("type") == "page":
                    return websocket
        except (OSError, ValueError):
            pass
        time.sleep(0.2)
    raise RuntimeError(INTERACTIVE_TIMEOUT)


def _wait_result(
    client: DevToolsConnection,
    command_id: int,
    timeout: float,
) -> dict[str, Any]:
    deadline = time.monotonic() + timeout
    while time.monotonic() < deadline:
        try:
            message = client.recv(timeout=max(0.1, deadline - time.monotonic()))
        except TimeoutError:
            continue
        if message is None:
            raise DevToolsError("Chrome closed the DevTools WebSocket.")
        if message.get("id") == command_id:
            return message
    raise DevToolsError("timed out waiting for DevTools response")


def _cookie_names(message: dict[str, Any]) -> set[str]:
    result = message.get("result")
    cookies = result.get("cookies") if isinstance(result, dict) else None
    if not isinstance(cookies, list):
        return set()
    return {
        str(cookie.get("name"))
        for cookie in cookies
        if isinstance(cookie, dict) and isinstance(cookie.get("name"), str)
    }


def _login_url(page_url: str) -> str:
    query = urllib.parse.urlencode({"redirect_url": page_url})
    return f"https://www.tiktok.com/login/qrcode?{query}"


def wait_for_skill_owned_login(
    *,
    chrome: str,
    profile_dir: Path,
    page_url: str,
    timeout: float = LOGIN_WAIT_SECONDS,
    on_tick: Callable[[], None] | None = None,
    on_opened: Callable[[], None] | None = None,
) -> str:
    """Show TikTok QR login and retain only its private browser profile."""
    if not desktop_display_available():
        return DISPLAY_UNAVAILABLE
    port, debug_args = _devtools_port()
    command = [
        chrome,
        *_visible_chrome_args(),
        *login_challenge.companion_browser_args(),
        "--disable-gpu",
        "--no-sandbox",
        "--disable-dev-shm-usage",
        "--no-first-run",
        "--no-default-browser-check",
        *debug_args,
        "--remote-allow-origins=*",
        f"--user-data-dir={profile_dir}",
        "--profile-directory=Default",
        _login_url(page_url),
    ]
    process = subprocess.Popen(
        command,
        stdout=subprocess.DEVNULL,
        stderr=subprocess.DEVNULL,
        start_new_session=os.name == "posix",
    )
    client: DevToolsConnection | None = None
    ready_polls = 0
    last_challenge_at = 0.0
    scan_timeout = login_challenge.effective_scan_timeout(timeout)
    try:
        websocket = _wait_for_devtools(port, process, timeout, on_tick)
        client = DevToolsConnection(websocket, timeout=min(max(timeout, 1.0), 10.0))
        client.send("Runtime.enable")
        client.send("Network.enable")
        client.send("Page.enable")
        if on_opened is not None:
            on_opened()
        deadline = time.monotonic() + scan_timeout
        while time.monotonic() < deadline:
            if on_tick is not None:
                on_tick()
            if process.poll() is not None:
                return INTERACTIVE_CANCELLED
            now = time.monotonic()
            if login_challenge.challenge_has_expired(
                client,
                lambda command_id, wait: _wait_result(client, command_id, wait),
            ):
                return INTERACTIVE_TIMEOUT
            if now - last_challenge_at >= login_challenge.REFRESH_SECONDS:
                login_challenge.publish_login_challenge(
                    client,
                    lambda command_id, wait: _wait_result(client, command_id, wait),
                    profile_dir,
                    "tiktok",
                )
                last_challenge_at = now
            cookie_id = client.send("Network.getAllCookies")
            authenticated = bool(
                _AUTH_COOKIE_NAMES.intersection(
                    _cookie_names(_wait_result(client, cookie_id, timeout=5.0))
                )
            )
            ready_polls = ready_polls + 1 if authenticated else 0
            if ready_polls >= LOGIN_READY_POLLS:
                return "ok"
            time.sleep(LOGIN_POLL_SECONDS)
        return INTERACTIVE_TIMEOUT
    except (DevToolsError, RuntimeError) as exc:
        return INTERACTIVE_TIMEOUT if str(exc).startswith(INTERACTIVE_TIMEOUT) else INTERACTIVE_CANCELLED
    finally:
        login_challenge.clear_login_challenge(profile_dir, "tiktok")
        if client is not None:
            client.close()
        if process.poll() is None:
            process.terminate()
            try:
                process.wait(timeout=5)
            except subprocess.TimeoutExpired:
                process.kill()
                process.wait(timeout=5)
