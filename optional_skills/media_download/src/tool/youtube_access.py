"""Skill-owned YouTube login state and yt-dlp cookie integration.

The helper never reads the user's normal browser profile. Authentication state
is created only inside the media_download skill's private storage after the
user completes an interactive YouTube sign-in or verification flow.
"""

from __future__ import annotations

import json
import os
import socket
import subprocess
import sys
import time
import urllib.error
import urllib.request
from pathlib import Path
from typing import Any, Callable

from browser_devtools import DevToolsConnection, DevToolsError


LOGIN_WAIT_SECONDS = 600.0
LOGIN_POLL_SECONDS = 1.0
LOGIN_READY_POLLS = 2
LOGIN_REQUIRED = "login_required"
DISPLAY_UNAVAILABLE = "display_unavailable"
INTERACTIVE_TIMEOUT = "interactive_verification_timeout"
INTERACTIVE_CANCELLED = "interactive_verification_cancelled"

_AUTH_COOKIE_NAMES = {
    "SID",
    "HSID",
    "SSID",
    "APISID",
    "SAPISID",
    "__Secure-1PSID",
    "__Secure-3PSID",
}
_LOGIN_DIAGNOSTIC_MARKERS = (
    "sign in to confirm you're not a bot",
    "sign in to confirm you’re not a bot",
    "use --cookies-from-browser or --cookies",
    "login required",
)


def persistent_profile_dir(browser_profile_root: str | Path | None) -> Path | None:
    if not browser_profile_root:
        return None
    profile = Path(browser_profile_root).expanduser() / "youtube"
    profile.mkdir(parents=True, exist_ok=True)
    return profile


def profile_cookie_database(profile_dir: Path) -> Path | None:
    for candidate in (
        profile_dir / "Default" / "Network" / "Cookies",
        profile_dir / "Default" / "Cookies",
    ):
        try:
            if candidate.is_file() and candidate.stat().st_size > 0:
                return candidate
        except OSError:
            continue
    return None


def profile_has_cookies(profile_dir: Path) -> bool:
    return profile_cookie_database(profile_dir) is not None


def browser_name_for_executable(chrome: str) -> str:
    name = Path(chrome).name.lower()
    if "brave" in name:
        return "brave"
    if "edge" in name:
        return "edge"
    if "chromium" in name:
        return "chromium"
    return "chrome"


def ytdlp_browser_spec(chrome: str, profile_dir: Path, host_platform: str | None = None) -> str:
    browser = browser_name_for_executable(chrome)
    profile = profile_dir / "Default"
    platform_name = host_platform or sys.platform
    keyring = "+basictext" if platform_name.startswith("linux") else ""
    return f"{browser}{keyring}:{profile}"


def requires_login_diagnostic(text: str) -> bool:
    lowered = str(text or "").lower()
    return any(marker in lowered for marker in _LOGIN_DIAGNOSTIC_MARKERS)


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


def visible_chrome_args(
    environment: dict[str, str] | None = None,
    host_platform: str | None = None,
) -> list[str]:
    platform_name = host_platform or sys.platform
    env = os.environ if environment is None else environment
    args: list[str] = []
    if platform_name.startswith("linux"):
        args.append("--password-store=basic")
        if env.get("WAYLAND_DISPLAY"):
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
    on_tick: Callable[[], None] | None = None,
) -> str:
    deadline = time.monotonic() + max(1.0, min(timeout, 20.0))
    last_error: Exception | None = None
    while time.monotonic() < deadline:
        if on_tick is not None:
            on_tick()
        if process.poll() is not None:
            raise RuntimeError(INTERACTIVE_CANCELLED)
        try:
            request = urllib.request.Request(f"http://127.0.0.1:{port}/json/list")
            with urllib.request.urlopen(request, timeout=2.0) as response:
                targets = json.load(response)
            for target in targets:
                if isinstance(target, dict) and target.get("type") == "page":
                    websocket_url = target.get("webSocketDebuggerUrl")
                    if isinstance(websocket_url, str):
                        return websocket_url
        except (OSError, json.JSONDecodeError, urllib.error.URLError) as exc:
            last_error = exc
        time.sleep(0.1)
    raise RuntimeError(f"{INTERACTIVE_TIMEOUT}:{last_error}" if last_error else INTERACTIVE_TIMEOUT)


def _wait_devtools_result(
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
        if not isinstance(message, dict):
            if message is None:
                raise DevToolsError("Chrome closed the DevTools WebSocket.")
            continue
        if message.get("id") == command_id:
            return message
    raise DevToolsError("timed out waiting for DevTools response")


def _evaluation_value(message: dict[str, Any]) -> dict[str, Any]:
    result = message.get("result")
    if not isinstance(result, dict):
        return {}
    inner = result.get("result")
    if not isinstance(inner, dict):
        return {}
    value = inner.get("value")
    return value if isinstance(value, dict) else {}


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


def _read_login_observation(
    client: DevToolsConnection,
    *,
    timeout: float,
) -> tuple[dict[str, Any], set[str]]:
    # Read each command before sending the next one. _wait_devtools_result ignores
    # unrelated CDP messages, so multiple in-flight commands could otherwise lose
    # an out-of-order response and incorrectly look like a cancelled login.
    state_id = client.send(
        "Runtime.evaluate",
        {"expression": _PAGE_STATE_SCRIPT, "returnByValue": True},
    )
    state = _evaluation_value(_wait_devtools_result(client, state_id, timeout=timeout))
    cookie_id = client.send("Network.getAllCookies")
    cookies = _cookie_names(_wait_devtools_result(client, cookie_id, timeout=timeout))
    return state, cookies


_PAGE_STATE_SCRIPT = """
(() => ({
  host: location.hostname || "",
  has_player: Boolean(document.querySelector("video, ytd-player")),
  ready_state: document.readyState || ""
}))()
"""


def wait_for_skill_owned_login(
    *,
    chrome: str,
    profile_dir: Path,
    page_url: str,
    timeout: float = LOGIN_WAIT_SECONDS,
    on_tick: Callable[[], None] | None = None,
) -> str:
    """Open a visible private profile and wait for authenticated YouTube access."""
    if not desktop_display_available():
        return DISPLAY_UNAVAILABLE
    port, debug_args = _devtools_port()
    command = [
        chrome,
        *visible_chrome_args(),
        "--disable-gpu",
        "--no-sandbox",
        "--disable-dev-shm-usage",
        "--no-first-run",
        "--no-default-browser-check",
        *debug_args,
        "--remote-allow-origins=*",
        f"--user-data-dir={profile_dir}",
        "--profile-directory=Default",
        page_url,
    ]
    process = subprocess.Popen(
        command,
        stdout=subprocess.DEVNULL,
        stderr=subprocess.DEVNULL,
        start_new_session=os.name == "posix",
    )
    client: DevToolsConnection | None = None
    ready_polls = 0
    try:
        websocket_url = _wait_for_devtools(
            port,
            process,
            timeout=min(timeout, 20.0),
            on_tick=on_tick,
        )
        client = DevToolsConnection(websocket_url, timeout=min(max(timeout, 1.0), 10.0))
        client.send("Runtime.enable")
        client.send("Network.enable")
        deadline = time.monotonic() + max(5.0, timeout)
        while time.monotonic() < deadline:
            if on_tick is not None:
                on_tick()
            if process.poll() is not None:
                return INTERACTIVE_CANCELLED
            state, cookies = _read_login_observation(client, timeout=5.0)
            youtube_page_ready = str(state.get("host", "")).endswith("youtube.com") and bool(
                state.get("has_player")
            )
            authenticated = bool(cookies & _AUTH_COOKIE_NAMES)
            if youtube_page_ready and authenticated:
                ready_polls += 1
                if ready_polls >= LOGIN_READY_POLLS:
                    return "ok"
            else:
                ready_polls = 0
            time.sleep(LOGIN_POLL_SECONDS)
        return INTERACTIVE_TIMEOUT
    except (DevToolsError, RuntimeError) as exc:
        value = str(exc)
        if value.startswith(INTERACTIVE_TIMEOUT):
            return INTERACTIVE_TIMEOUT
        return INTERACTIVE_CANCELLED
    finally:
        if client is not None:
            client.close()
        if process.poll() is None:
            process.terminate()
            try:
                process.wait(timeout=5)
            except subprocess.TimeoutExpired:
                process.kill()
                process.wait(timeout=5)
