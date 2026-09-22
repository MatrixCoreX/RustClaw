"""Xiaohongshu share targeting, stream URL recognition, and skill-owned login.

This module never reads the system browser, another skill's storage, or raw
cookie values from the planner. A login session may live only in this skill's
private browser-profile directory after the user signs in locally.
"""

from __future__ import annotations

import json
import os
import re
import socket
import subprocess
import sys
import time
import urllib.error
import urllib.parse
import urllib.request
from dataclasses import dataclass
from pathlib import Path
from typing import Any, Callable, Iterable

from browser_devtools import DevToolsConnection, DevToolsError


NOTE_PATH_RE = re.compile(r"/(?:explore|discovery/item)/([0-9a-zA-Z]+)")
LOGIN_PATH_RE = re.compile(r"/login(?:/|$)", re.IGNORECASE)
XSEC_QUERY_KEYS = ("xsec_token", "xsecToken")
VIDEO_CDN_HOST_MARKERS = ("sns-video", "sns-bak", "redcdn")
VIDEO_CDN_PATH_MARKERS = (".mp4", "stream", "video")
LOGIN_WAIT_SECONDS = 600.0
LOGIN_POLL_SECONDS = 1.0
LOGIN_READY_POLLS = 2
LOGIN_REQUIRED = "login_required"
DISPLAY_UNAVAILABLE = "display_unavailable"
INTERACTIVE_TIMEOUT = "interactive_verification_timeout"
INTERACTIVE_CANCELLED = "interactive_verification_cancelled"


@dataclass(frozen=True)
class XiaohongshuShareTarget:
    original_url: str
    page_url: str
    note_id: str | None
    xsec_token: str | None
    login_barrier: bool = False


def extract_xiaohongshu_note_id(*parts: str) -> str | None:
    for part in parts:
        if not part:
            continue
        decoded = urllib.parse.unquote(html_unescape(part))
        match = NOTE_PATH_RE.search(decoded)
        if match:
            return match.group(1)
        query = urllib.parse.parse_qs(urllib.parse.urlsplit(decoded).query)
        for key in ("note_id", "noteId", "item_id"):
            values = query.get(key) or []
            if values and str(values[0]).strip():
                return str(values[0]).strip()
    return None


def html_unescape(value: str) -> str:
    return (
        value.replace("&amp;", "&")
        .replace("&quot;", '"')
        .replace("&#39;", "'")
        .replace("&lt;", "<")
        .replace("&gt;", ">")
    )


def extract_xsec_token(*parts: str) -> str | None:
    for part in parts:
        if not part:
            continue
        decoded = urllib.parse.unquote(html_unescape(part))
        query = urllib.parse.parse_qs(urllib.parse.urlsplit(decoded).query)
        for key in XSEC_QUERY_KEYS:
            values = query.get(key) or []
            token = str(values[0]).strip() if values else ""
            if token:
                return token
        match = re.search(r"[?&](?:xsec_token|xsecToken)=([^&#]+)", decoded)
        if match:
            token = urllib.parse.unquote(match.group(1)).strip()
            if token:
                return token
    return None


def is_login_url(url: str) -> bool:
    parsed = urllib.parse.urlsplit(url)
    if LOGIN_PATH_RE.search(parsed.path):
        return True
    query = urllib.parse.parse_qs(parsed.query)
    redirect = str((query.get("redirectPath") or [""])[0])
    return bool(redirect) and LOGIN_PATH_RE.search(urllib.parse.urlsplit(redirect).path or "")


def item_url(note_id: str, xsec_token: str | None = None, *, source: str = "app_share") -> str:
    query = {"xsec_source": source}
    if xsec_token:
        query["xsec_token"] = xsec_token
    return f"https://www.xiaohongshu.com/discovery/item/{note_id}?{urllib.parse.urlencode(query)}"


def target_from_url(url: str) -> XiaohongshuShareTarget:
    if is_login_url(url):
        recovered = recover_from_login_url(url)
        if recovered is not None:
            return recovered
    note_id = extract_xiaohongshu_note_id(url)
    token = extract_xsec_token(url)
    page_url = item_url(note_id, token) if note_id else url
    return XiaohongshuShareTarget(
        original_url=url,
        page_url=page_url,
        note_id=note_id,
        xsec_token=token,
        login_barrier=is_login_url(url) and note_id is None,
    )


def recover_from_login_url(url: str) -> XiaohongshuShareTarget | None:
    parsed = urllib.parse.urlsplit(url)
    query = urllib.parse.parse_qs(parsed.query)
    redirect = str((query.get("redirectPath") or query.get("redirect") or [""])[0]).strip()
    if not redirect:
        return None
    redirect = urllib.parse.unquote(redirect)
    if not redirect.startswith(("http://", "https://")):
        redirect = urllib.parse.urljoin("https://www.xiaohongshu.com/", redirect)
    note_id = extract_xiaohongshu_note_id(redirect, url)
    token = extract_xsec_token(redirect, url)
    if not note_id:
        return XiaohongshuShareTarget(
            original_url=url,
            page_url=url,
            note_id=None,
            xsec_token=token,
            login_barrier=True,
        )
    return XiaohongshuShareTarget(
        original_url=url,
        page_url=item_url(note_id, token),
        note_id=note_id,
        xsec_token=token,
        login_barrier=True,
    )


def select_share_target(original_url: str, chain: Iterable[str]) -> XiaohongshuShareTarget:
    urls = [original_url, *[item for item in chain if item]]
    for candidate in reversed(urls):
        if is_login_url(candidate):
            recovered = recover_from_login_url(candidate)
            if recovered is not None and recovered.note_id:
                return recovered
            continue
        target = target_from_url(candidate)
        if target.note_id:
            return target
    last = urls[-1] if urls else original_url
    if is_login_url(last):
        recovered = recover_from_login_url(last)
        if recovered is not None:
            return recovered
    return target_from_url(last)


class _RedirectRecorder(urllib.request.HTTPRedirectHandler):
    def __init__(self) -> None:
        super().__init__()
        self.urls: list[str] = []

    def redirect_request(self, req, fp, code, msg, headers, newurl):  # type: ignore[override]
        self.urls.append(newurl)
        return super().redirect_request(req, fp, code, msg, headers, newurl)


def resolve_xiaohongshu_share_url(
    url: str,
    *,
    cookie: str | None = None,
    timeout: float = 20.0,
    headers: dict[str, str] | None = None,
) -> XiaohongshuShareTarget:
    recorder = _RedirectRecorder()
    opener = urllib.request.build_opener(recorder)
    request_headers = dict(headers or {})
    if cookie:
        request_headers["Cookie"] = cookie
    request = urllib.request.Request(url, headers=request_headers)
    final = url
    try:
        with opener.open(request, timeout=timeout) as response:
            final = response.geturl()
    except (urllib.error.HTTPError, urllib.error.URLError, TimeoutError, OSError):
        if recorder.urls:
            final = recorder.urls[-1]
    return select_share_target(url, [*recorder.urls, final])


def preferred_browser_urls(share_text: str, urls: Iterable[str]) -> list[str]:
    ordered: list[str] = []
    seen: set[str] = set()

    def add(url: str) -> None:
        if not url or url in seen or is_login_url(url):
            return
        seen.add(url)
        ordered.append(url)

    seed = [share_text, *urls]
    target = select_share_target(next((item for item in seed if item), ""), seed)
    if target.note_id:
        add(target.page_url)
    for url in urls:
        add(url)
    return ordered or [url for url in urls if url]


def xiaohongshu_html_is_login(page_text: str, url: str = "") -> bool:
    if url and is_login_url(url):
        return True
    if LOGIN_PATH_RE.search(url):
        return True
    lowered = page_text[:8000]
    if NOTE_PATH_RE.search(page_text) and "noteDetailMap" in page_text:
        return False
    return "noteDetailMap" not in page_text and (
        "/login" in lowered
        or "redirectPath=" in lowered
        or '"login"' in lowered
    )


def xiaohongshu_needs_skill_owned_login(
    *,
    page_text: str = "",
    url: str = "",
    http_login_barrier: bool = False,
    dump_timed_out: bool = False,
    has_media: bool = False,
) -> bool:
    """Open the skill-owned profile when the note is a login wall, including empty dump-dom."""
    if has_media:
        return False
    if xiaohongshu_html_is_login(page_text, url):
        return True
    if not http_login_barrier:
        return False
    text = str(page_text or "")
    if dump_timed_out or not text.strip():
        return True
    return "noteDetailMap" not in text


def xiaohongshu_login_outcome_is_terminal(logs: Iterable[str]) -> bool:
    joined = "\n".join(logs)
    return any(
        token in joined
        for token in (
            "xiaohongshu: display_unavailable",
            f"interactive_login={INTERACTIVE_TIMEOUT}",
            f"interactive_login={INTERACTIVE_CANCELLED}",
            "xiaohongshu: login_required",
        )
    )


def looks_like_xiaohongshu_video_url(url: str) -> bool:
    raw = url.strip()
    if raw.startswith("//"):
        raw = f"https:{raw}"
    if not raw.startswith(("http://", "https://")):
        return False
    parsed = urllib.parse.urlsplit(raw)
    host = parsed.netloc.lower()
    path = parsed.path
    if not path or path == "/":
        return False
    if path.endswith((".ico", ".json", ".pdf", ".js", ".css", ".png", ".jpg", ".jpeg", ".webp", ".svg")):
        return False
    if any(token in host for token in VIDEO_CDN_HOST_MARKERS):
        return True
    return any(token in path.lower() for token in VIDEO_CDN_PATH_MARKERS) and any(
        token in host for token in ("xiaohongshu", "xhscdn", "xhs")
    )


def iter_xiaohongshu_stream_groups(stream: Any) -> Iterable[tuple[str, int, Any]]:
    if not isinstance(stream, dict):
        return
    for codec_index, (codec, items) in enumerate(stream.items()):
        if not isinstance(codec, str) or not isinstance(items, list):
            continue
        for stream_index, stream_item in enumerate(items):
            yield f"{codec}", codec_index * 10 + stream_index, stream_item


def persistent_profile_dir(browser_profile_root: str | Path | None) -> Path | None:
    if not browser_profile_root:
        return None
    root = Path(browser_profile_root).expanduser()
    profile = root / "xiaohongshu"
    profile.mkdir(parents=True, exist_ok=True)
    return profile


def desktop_display_available(
    environment: dict[str, str] | None = None,
    platform: str | None = None,
) -> bool:
    host = platform or sys.platform
    env = os.environ if environment is None else environment
    if host == "darwin":
        return True
    if host.startswith("linux"):
        return bool(env.get("DISPLAY") or env.get("WAYLAND_DISPLAY"))
    return False


def visible_chrome_args(
    environment: dict[str, str] | None = None,
    platform: str | None = None,
) -> list[str]:
    host = platform or sys.platform
    env = os.environ if environment is None else environment
    if host.startswith("linux") and env.get("WAYLAND_DISPLAY"):
        return ["--ozone-platform=wayland"]
    return []


def note_access_ready(snapshot: dict[str, Any], note_id: str | None) -> bool:
    if snapshot.get("login"):
        return False
    ids = snapshot.get("note_ids")
    if not isinstance(ids, list):
        ids = []
    if note_id:
        return note_id in ids
    return bool(ids) or bool(snapshot.get("has_video"))


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
    deadline = time.monotonic() + max(1.0, min(timeout, 15.0))
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


NOTE_ACCESS_SCRIPT = """
(() => {
  const path = location.pathname || "";
  const login = /\\/login/i.test(path);
  const map = (window.__INITIAL_STATE__ && window.__INITIAL_STATE__.note
    && window.__INITIAL_STATE__.note.noteDetailMap) || {};
  const ids = Object.keys(map);
  const hasVideo = Object.values(map).some((entry) => entry && entry.note && entry.note.video);
  return { login, note_ids: ids, has_video: hasVideo };
})()
"""


def wait_for_skill_owned_login(
    *,
    chrome: str,
    profile_dir: Path,
    page_url: str,
    note_id: str | None,
    timeout: float = LOGIN_WAIT_SECONDS,
    on_tick: Callable[[], None] | None = None,
) -> str:
    """Open a visible skill-owned profile and wait until the note is readable."""
    if not desktop_display_available():
        return DISPLAY_UNAVAILABLE
    port, debug_args = _devtools_port()
    command = [
        chrome,
        *visible_chrome_args(),
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
        stdout=subprocess.DEVNULL,
        stderr=subprocess.DEVNULL,
        start_new_session=os.name == "posix",
    )
    ready_polls = 0
    client: DevToolsConnection | None = None
    try:
        websocket_url = _wait_for_devtools(
            port,
            process,
            timeout=min(timeout, 20.0),
            on_tick=on_tick,
        )
        client = DevToolsConnection(websocket_url, timeout=min(max(timeout, 1.0), 10.0))
        client.send("Runtime.enable")
        deadline = time.monotonic() + max(5.0, timeout)
        while time.monotonic() < deadline:
            if on_tick is not None:
                on_tick()
            if process.poll() is not None:
                return INTERACTIVE_CANCELLED
            command_id = client.send(
                "Runtime.evaluate",
                {"expression": NOTE_ACCESS_SCRIPT, "returnByValue": True},
            )
            event = _wait_devtools_result(client, command_id, timeout=5.0)
            snapshot = _evaluate_result(event)
            if note_access_ready(snapshot, note_id):
                ready_polls += 1
                if ready_polls >= LOGIN_READY_POLLS:
                    return "ok"
            else:
                ready_polls = 0
            time.sleep(LOGIN_POLL_SECONDS)
        return INTERACTIVE_TIMEOUT
    except DevToolsError:
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


def _wait_devtools_result(client: DevToolsConnection, command_id: int, timeout: float) -> dict[str, Any]:
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
    raise DevToolsError("timed out waiting for Runtime.evaluate")


def _evaluate_result(message: dict[str, Any]) -> dict[str, Any]:
    result = message.get("result")
    if not isinstance(result, dict):
        return {}
    inner = result.get("result")
    if not isinstance(inner, dict):
        return {}
    value = inner.get("value")
    return value if isinstance(value, dict) else {}
