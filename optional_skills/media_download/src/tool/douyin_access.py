"""Skill-owned Douyin login state used to resolve complete image-post media.

Public Douyin payloads can expose a live photo as a single still image.  This
module keeps the optional authenticated session inside media_download's own
private storage so the downloader can distinguish an ordinary image from a
live photo without reading the user's normal browser profile.
"""

from __future__ import annotations

import json
import os
import socket
import sqlite3
import subprocess
import time
import urllib.error
import urllib.request
from pathlib import Path
from typing import Any, Callable

from browser_devtools import DevToolsConnection, DevToolsError
import login_challenge
import xiaohongshu_access


LOGIN_WAIT_SECONDS = 600.0
LOGIN_POLL_SECONDS = 1.0
LOGIN_READY_POLLS = 2
DISPLAY_UNAVAILABLE = "display_unavailable"
INTERACTIVE_TIMEOUT = "interactive_verification_timeout"
INTERACTIVE_CANCELLED = "interactive_verification_cancelled"

_AUTH_COOKIE_NAMES = {
    "sessionid",
    "sessionid_ss",
    "sid_guard",
    "sid_tt",
    "uid_tt",
    "uid_tt_ss",
}


def persistent_profile_dir(browser_profile_root: str | Path | None) -> Path | None:
    if not browser_profile_root:
        return None
    profile = Path(browser_profile_root).expanduser() / "douyin"
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


def profile_has_authenticated_session(profile_dir: Path) -> bool:
    database = profile_cookie_database(profile_dir)
    if database is None:
        return False
    connection: sqlite3.Connection | None = None
    try:
        connection = sqlite3.connect(f"file:{database}?mode=ro", uri=True, timeout=1.0)
        rows = connection.execute(
            "select name, expires_utc, length(value), length(encrypted_value) "
            "from cookies where host_key like '%douyin.com'"
        ).fetchall()
    except sqlite3.Error:
        return False
    finally:
        if connection is not None:
            connection.close()
    chrome_now = int((time.time() + 11_644_473_600) * 1_000_000)
    return any(
        row
        and str(row[0]) in _AUTH_COOKIE_NAMES
        and (int(row[2] or 0) > 0 or int(row[3] or 0) > 0)
        and (int(row[1] or 0) == 0 or int(row[1]) > chrome_now)
        for row in rows
    )


def remove_stale_chrome_singleton_artifacts(profile_dir: Path) -> bool:
    """Remove Chrome's stale single-instance links for this private profile.

    Chrome records a PID from its own process namespace in ``SingletonLock``.
    A sandbox PID can later collide with an unrelated host PID, so checking that
    number alone can incorrectly make a reusable private profile look busy.  The
    Unix socket is the authoritative liveness signal: preserve every marker when
    it accepts a connection, otherwise remove only the three Chrome-owned
    singleton symlinks.
    """
    socket_marker = profile_dir / "SingletonSocket"
    if socket_marker.is_symlink():
        try:
            target = Path(os.readlink(socket_marker))
            if not target.is_absolute():
                target = profile_dir / target
            with socket.socket(socket.AF_UNIX, socket.SOCK_STREAM) as probe:
                probe.settimeout(0.2)
                probe.connect(str(target))
            return False
        except OSError:
            pass

    removed = False
    for name in ("SingletonCookie", "SingletonLock", "SingletonSocket"):
        marker = profile_dir / name
        if not marker.is_symlink():
            continue
        try:
            marker.unlink()
            removed = True
        except FileNotFoundError:
            pass
    return removed


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
            with urllib.request.urlopen(
                f"http://127.0.0.1:{port}/json/list",
                timeout=2.0,
            ) as response:
                targets = json.load(response)
            for target in targets:
                if isinstance(target, dict) and target.get("type") == "page":
                    websocket_url = target.get("webSocketDebuggerUrl")
                    if isinstance(websocket_url, str):
                        return websocket_url
        except (OSError, json.JSONDecodeError, urllib.error.URLError):
            pass
        time.sleep(0.1)
    raise RuntimeError(INTERACTIVE_TIMEOUT)


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
        if message is None:
            raise DevToolsError("Chrome closed the DevTools WebSocket.")
        if isinstance(message, dict) and message.get("id") == command_id:
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


def wait_for_skill_owned_login(
    *,
    chrome: str,
    profile_dir: Path,
    page_url: str,
    timeout: float = LOGIN_WAIT_SECONDS,
    on_tick: Callable[[], None] | None = None,
    on_opened: Callable[[], None] | None = None,
) -> str:
    """Open Douyin visibly and retain only the private profile after login."""
    desktop_environment = xiaohongshu_access.desktop_session_environment()
    if not xiaohongshu_access.desktop_display_available(desktop_environment):
        return DISPLAY_UNAVAILABLE

    remove_stale_chrome_singleton_artifacts(profile_dir)
    port, debug_args = _devtools_port()
    command = [
        chrome,
        *xiaohongshu_access.visible_chrome_args(desktop_environment),
        "--disable-gpu",
        "--no-sandbox",
        "--disable-dev-shm-usage",
        "--disable-blink-features=AutomationControlled",
        "--mute-audio",
        *debug_args,
        "--remote-allow-origins=*",
        f"--user-data-dir={profile_dir}",
        "--profile-directory=Default",
        page_url,
    ]
    process = subprocess.Popen(
        command,
        env=desktop_environment,
        stdout=subprocess.DEVNULL,
        stderr=subprocess.DEVNULL,
        start_new_session=os.name == "posix",
    )
    client: DevToolsConnection | None = None
    ready_polls = 0
    last_challenge_at = 0.0
    try:
        websocket_url = _wait_for_devtools(
            port,
            process,
            timeout,
            on_tick,
        )
        client = DevToolsConnection(websocket_url, timeout=min(max(timeout, 1.0), 10.0))
        client.send("Network.enable")
        if on_opened is not None:
            on_opened()
        deadline = time.monotonic() + max(5.0, timeout)
        while time.monotonic() < deadline:
            if on_tick is not None:
                on_tick()
            if process.poll() is not None:
                return INTERACTIVE_CANCELLED
            now = time.monotonic()
            if now - last_challenge_at >= login_challenge.REFRESH_SECONDS:
                login_challenge.publish_login_challenge(
                    client,
                    lambda command_id, wait: _wait_devtools_result(client, command_id, wait),
                    profile_dir,
                    "douyin",
                )
                last_challenge_at = now
            command_id = client.send("Network.getAllCookies")
            names = _cookie_names(_wait_devtools_result(client, command_id, timeout=5.0))
            if _AUTH_COOKIE_NAMES.intersection(names):
                ready_polls += 1
                if ready_polls >= LOGIN_READY_POLLS:
                    return "ok"
            else:
                ready_polls = 0
            time.sleep(LOGIN_POLL_SECONDS)
        return INTERACTIVE_TIMEOUT
    except (DevToolsError, RuntimeError):
        return INTERACTIVE_CANCELLED
    finally:
        login_challenge.clear_login_challenge(profile_dir, "douyin")
        if client is not None:
            client.close()
        if process.poll() is None:
            process.terminate()
            try:
                process.wait(timeout=5)
            except subprocess.TimeoutExpired:
                process.kill()
                process.wait(timeout=5)
        remove_stale_chrome_singleton_artifacts(profile_dir)
