"""Resolve WeChat Channels shares through a skill-owned Yuanbao session.

The login session lives only in the media_download private browser profile. The
module never imports cookies from the user's normal browser and never exposes
session cookies to the downloader process. Yuanbao is called inside Chromium;
only the short-lived preview token is used for the public Channels feed call.
"""

from __future__ import annotations

import contextlib
import fcntl
import ipaddress
import json
import os
import re
import secrets
import socket
import subprocess
import time
import urllib.error
import urllib.parse
import urllib.request
from dataclasses import dataclass
from pathlib import Path
from typing import Any, Callable, Iterator

import xiaohongshu_access
from browser_devtools import DevToolsConnection, DevToolsError


YUANBAO_HOME = "https://yuanbao.tencent.com/"
YUANBAO_PARSE_ENDPOINT = "https://yuanbao.tencent.com/api/weixin/get_parse_result"
FINDER_FEED_ENDPOINT = (
    "https://channels.weixin.qq.com/finder-preview/api/feed/get_feed_info"
)
FINDER_PAGE = "https://channels.weixin.qq.com/finder-preview/pages/feed"
LOGIN_WAIT_SECONDS = 600.0
LOGIN_POLL_SECONDS = 1.0
LOGIN_READY_POLLS = 2
MAX_API_RESPONSE_BYTES = 4 * 1024 * 1024
LOGIN_REQUIRED = "login_required"
DISPLAY_UNAVAILABLE = "display_unavailable"
INTERACTIVE_TIMEOUT = "interactive_verification_timeout"
INTERACTIVE_CANCELLED = "interactive_verification_cancelled"

_SHARE_URL_RE = re.compile(r"https?://[^\s\"'<>，。；：！？）】》、]+")
_SHARE_CODE_RE = re.compile(r"^[A-Za-z0-9_-]+$")


class WechatChannelsAccessError(RuntimeError):
    """A stable provider-scoped access failure."""


@dataclass(frozen=True)
class ResolvedWechatChannels:
    share_url: str
    export_id: str
    author: str
    description: str
    video_urls: tuple[tuple[str, str], ...]
    image_urls: tuple[str, ...]
    cover_url: str | None
    logs: tuple[str, ...]


def normalize_share_url(raw: str) -> str:
    value = raw.strip().rstrip(".,;:!?)]}>\"'，。；：！？）】》、")
    if len(value) > 8_192 or any(char in value for char in "\r\n\t\x00"):
        raise WechatChannelsAccessError("invalid_wechat_channels_share_url")
    parsed = urllib.parse.urlsplit(value)
    if (
        parsed.scheme != "https"
        or parsed.hostname != "weixin.qq.com"
        or parsed.username is not None
        or parsed.password is not None
        or parsed.port is not None
    ):
        raise WechatChannelsAccessError("invalid_wechat_channels_share_url")
    prefix = "/sph/"
    if not parsed.path.startswith(prefix):
        raise WechatChannelsAccessError("invalid_wechat_channels_share_url")
    code = parsed.path[len(prefix) :]
    if not code or "/" in code or not _SHARE_CODE_RE.fullmatch(code):
        raise WechatChannelsAccessError("invalid_wechat_channels_share_url")
    return urllib.parse.urlunsplit(
        (parsed.scheme, parsed.netloc, parsed.path, parsed.query, "")
    )


def extract_share_url(text: str) -> str | None:
    candidates = [match.group(0) for match in _SHARE_URL_RE.finditer(text)]
    if not candidates and text.strip().startswith(("http://", "https://")):
        candidates = [text.strip()]
    for candidate in candidates:
        try:
            return normalize_share_url(candidate)
        except WechatChannelsAccessError:
            continue
    return None


def persistent_profile_dir(browser_profile_root: str | Path | None) -> Path | None:
    if not browser_profile_root:
        return None
    profile = Path(browser_profile_root).expanduser() / "wechat_channels"
    profile.mkdir(parents=True, exist_ok=True, mode=0o700)
    try:
        profile.chmod(0o700)
    except OSError:
        pass
    return profile


@contextlib.contextmanager
def profile_lock(
    profile_dir: Path,
    cancellation_check: Callable[[], None] | None = None,
) -> Iterator[None]:
    lock_path = profile_dir / ".profile.lock"
    descriptor = os.open(lock_path, os.O_CREAT | os.O_RDWR, 0o600)
    try:
        while True:
            if cancellation_check is not None:
                cancellation_check()
            try:
                fcntl.flock(descriptor, fcntl.LOCK_EX | fcntl.LOCK_NB)
                break
            except BlockingIOError:
                time.sleep(0.2)
        yield
    finally:
        try:
            fcntl.flock(descriptor, fcntl.LOCK_UN)
        finally:
            os.close(descriptor)


def parse_preview_url(playable_url: str) -> tuple[str, str]:
    parsed = urllib.parse.urlsplit(playable_url)
    if (
        parsed.scheme != "https"
        or parsed.hostname != "channels.weixin.qq.com"
        or parsed.path != "/finder-preview/pages/feed"
        or parsed.username is not None
        or parsed.password is not None
        or parsed.port is not None
    ):
        raise WechatChannelsAccessError("wechat_channels_preview_schema_changed")
    pairs = urllib.parse.parse_qsl(parsed.query, keep_blank_values=True)
    tokens = [value for key, value in pairs if key == "token"]
    export_ids = [value for key, value in pairs if key == "eid"]
    if len(tokens) != 1 or len(export_ids) != 1 or not tokens[0] or not export_ids[0]:
        raise WechatChannelsAccessError("wechat_channels_preview_schema_changed")
    return tokens[0], export_ids[0]


def _devtools_endpoint() -> tuple[int, list[str]]:
    with socket.socket(socket.AF_INET, socket.SOCK_STREAM) as listener:
        listener.bind(("127.0.0.1", 0))
        port = int(listener.getsockname()[1])
    return port, [
        f"--remote-debugging-port={port}",
        "--remote-debugging-address=127.0.0.1",
    ]


def _wait_for_page(
    port: int,
    process: subprocess.Popen[Any],
    timeout: float,
    cancellation_check: Callable[[], None] | None,
) -> str:
    deadline = time.monotonic() + max(1.0, min(timeout, 20.0))
    last_error: Exception | None = None
    while time.monotonic() < deadline:
        if cancellation_check is not None:
            cancellation_check()
        if process.poll() is not None:
            raise WechatChannelsAccessError(INTERACTIVE_CANCELLED)
        try:
            request = urllib.request.Request(f"http://127.0.0.1:{port}/json/list")
            with urllib.request.urlopen(request, timeout=2.0) as response:
                targets = json.load(response)
            for target in targets:
                if not isinstance(target, dict) or target.get("type") != "page":
                    continue
                websocket_url = target.get("webSocketDebuggerUrl")
                if isinstance(websocket_url, str):
                    return websocket_url
        except (OSError, urllib.error.URLError, json.JSONDecodeError) as exc:
            last_error = exc
        time.sleep(0.1)
    detail = f":{last_error}" if last_error else ""
    raise WechatChannelsAccessError(f"wechat_channels_browser_start_timeout{detail}")


def _wait_result(
    client: DevToolsConnection,
    command_id: int,
    timeout: float,
    cancellation_check: Callable[[], None] | None,
) -> dict[str, Any]:
    deadline = time.monotonic() + max(1.0, timeout)
    while time.monotonic() < deadline:
        if cancellation_check is not None:
            cancellation_check()
        try:
            message = client.recv(timeout=min(1.0, max(0.1, deadline - time.monotonic())))
        except TimeoutError:
            continue
        if message is None:
            raise WechatChannelsAccessError(INTERACTIVE_CANCELLED)
        if message.get("id") == command_id:
            return message
    raise WechatChannelsAccessError("wechat_channels_browser_command_timeout")


def _evaluation_value(message: dict[str, Any]) -> dict[str, Any]:
    result = message.get("result")
    inner = result.get("result") if isinstance(result, dict) else None
    value = inner.get("value") if isinstance(inner, dict) else None
    if isinstance(value, dict):
        return value
    exception = result.get("exceptionDetails") if isinstance(result, dict) else None
    if exception:
        raise WechatChannelsAccessError("wechat_channels_browser_evaluation_failed")
    return {}


def _parse_share_in_page(
    client: DevToolsConnection,
    share_url: str,
    timeout: float,
    cancellation_check: Callable[[], None] | None,
) -> dict[str, Any]:
    endpoint = json.dumps(YUANBAO_PARSE_ENDPOINT)
    payload = json.dumps(
        {"type": "video_channel_url", "url": share_url, "scene": 1},
        ensure_ascii=False,
        separators=(",", ":"),
    )
    expression = f"""
(async () => {{
  try {{
    const response = await fetch({endpoint}, {{
      method: "POST",
      credentials: "include",
      headers: {{"accept":"application/json, text/plain, */*","content-type":"application/json"}},
      body: {json.dumps(payload)}
    }});
    const raw = await response.text();
    let body = null;
    try {{ body = JSON.parse(raw); }} catch (_) {{}}
    return {{http_status: response.status, body, parse_error: body === null}};
  }} catch (_) {{
    return {{network_error: true}};
  }}
}})()
"""
    command_id = client.send(
        "Runtime.evaluate",
        {"expression": expression, "awaitPromise": True, "returnByValue": True},
    )
    return _evaluation_value(
        _wait_result(client, command_id, timeout, cancellation_check)
    )


def _classify_parse_result(value: dict[str, Any]) -> tuple[str, dict[str, Any] | None]:
    status = value.get("http_status")
    body = value.get("body")
    if status in {401, 403}:
        return LOGIN_REQUIRED, None
    if value.get("network_error"):
        return "retry", None
    if not isinstance(body, dict):
        return "retry", None
    code = body.get("code")
    if code == 0:
        data = body.get("data")
        if isinstance(data, dict) and isinstance(data.get("playable_url"), str):
            return "ok", data
        raise WechatChannelsAccessError("wechat_channels_parse_schema_changed")
    if code in {1008, 401, 403}:
        return LOGIN_REQUIRED, None
    raise WechatChannelsAccessError(f"wechat_channels_parse_error:{code}")


def _stop_browser(process: subprocess.Popen[Any]) -> None:
    if process.poll() is not None:
        return
    process.terminate()
    try:
        process.wait(timeout=5.0)
    except subprocess.TimeoutExpired:
        process.kill()
        process.wait(timeout=5.0)


def parse_share_with_browser(
    *,
    chrome: str,
    profile_dir: Path,
    share_url: str,
    visible: bool,
    request_timeout: float,
    login_timeout: float = LOGIN_WAIT_SECONDS,
    cancellation_check: Callable[[], None] | None = None,
) -> dict[str, Any]:
    environment = xiaohongshu_access.desktop_session_environment()
    port, debug_args = _devtools_endpoint()
    command = [
        chrome,
        *([] if visible else ["--headless=new"]),
        *(xiaohongshu_access.visible_chrome_args(environment) if visible else []),
        "--disable-gpu",
        "--no-sandbox",
        "--disable-dev-shm-usage",
        "--no-first-run",
        "--no-default-browser-check",
        *debug_args,
        "--remote-allow-origins=*",
        f"--user-data-dir={profile_dir}",
        "--profile-directory=Default",
        YUANBAO_HOME,
    ]
    process = subprocess.Popen(
        command,
        stdout=subprocess.DEVNULL,
        stderr=subprocess.DEVNULL,
        env=environment,
    )
    client: DevToolsConnection | None = None
    try:
        websocket_url = _wait_for_page(
            port,
            process,
            request_timeout,
            cancellation_check,
        )
        client = DevToolsConnection(
            websocket_url,
            timeout=min(max(request_timeout, 1.0), 10.0),
        )
        client.send("Runtime.enable")
        client.send("Network.enable")
        deadline = time.monotonic() + (login_timeout if visible else max(5.0, request_timeout))
        ready_polls = 0
        while time.monotonic() < deadline:
            if cancellation_check is not None:
                cancellation_check()
            if process.poll() is not None:
                raise WechatChannelsAccessError(INTERACTIVE_CANCELLED)
            try:
                state, data = _classify_parse_result(
                    _parse_share_in_page(
                        client,
                        share_url,
                        min(max(request_timeout, 1.0), 20.0),
                        cancellation_check,
                    )
                )
            except DevToolsError:
                state, data = "retry", None
            except WechatChannelsAccessError as exc:
                if str(exc) not in {
                    "wechat_channels_browser_command_timeout",
                    "wechat_channels_browser_evaluation_failed",
                }:
                    raise
                state, data = "retry", None
            if state == "ok" and data is not None:
                ready_polls += 1
                if not visible or ready_polls >= LOGIN_READY_POLLS:
                    return data
            elif state == LOGIN_REQUIRED:
                ready_polls = 0
                if not visible:
                    raise WechatChannelsAccessError(LOGIN_REQUIRED)
            else:
                ready_polls = 0
            time.sleep(LOGIN_POLL_SECONDS if visible else 0.25)
        if visible:
            raise WechatChannelsAccessError(INTERACTIVE_TIMEOUT)
        raise WechatChannelsAccessError(LOGIN_REQUIRED)
    finally:
        if client is not None:
            client.close()
        _stop_browser(process)


def _read_bounded(response: Any) -> bytes:
    content = response.read(MAX_API_RESPONSE_BYTES + 1)
    if len(content) > MAX_API_RESPONSE_BYTES:
        raise WechatChannelsAccessError("wechat_channels_response_too_large")
    return content


def _post_json(
    url: str,
    payload: dict[str, Any],
    headers: dict[str, str],
    timeout: float,
) -> tuple[int, dict[str, Any]]:
    request = urllib.request.Request(
        url,
        data=json.dumps(payload, ensure_ascii=False, separators=(",", ":")).encode("utf-8"),
        headers=headers,
        method="POST",
    )
    try:
        with urllib.request.urlopen(request, timeout=timeout) as response:
            status = int(getattr(response, "status", response.getcode()))
            raw = _read_bounded(response)
    except urllib.error.HTTPError as exc:
        status = int(exc.code)
        raw = _read_bounded(exc)
    try:
        body = json.loads(raw.decode("utf-8"))
    except (UnicodeDecodeError, json.JSONDecodeError) as exc:
        raise WechatChannelsAccessError("wechat_channels_feed_schema_changed") from exc
    if not isinstance(body, dict):
        raise WechatChannelsAccessError("wechat_channels_feed_schema_changed")
    return status, body


def _valid_media_url(value: Any) -> str | None:
    if not isinstance(value, str) or not value:
        return None
    parsed = urllib.parse.urlsplit(value)
    if (
        parsed.scheme != "https"
        or not parsed.hostname
        or parsed.username is not None
        or parsed.password is not None
        or parsed.port is not None
    ):
        return None
    try:
        ipaddress.ip_address(parsed.hostname.strip("[]"))
    except ValueError:
        return value
    return None


def _nested_video_url(feed: dict[str, Any], key: str) -> Any:
    value = feed.get(key)
    return value.get("videoUrl") if isinstance(value, dict) else None


def fetch_feed_info(
    playable_url: str,
    *,
    timeout: float,
) -> tuple[str, str, tuple[tuple[str, str], ...], tuple[str, ...], str | None]:
    token, export_id = parse_preview_url(playable_url)
    rid = f"{int(time.time()):x}-{secrets.token_hex(4)}"
    query = urllib.parse.urlencode(
        {"_rid": rid, "_pageUrl": FINDER_PAGE}
    )
    referer_query = urllib.parse.urlencode(
        {
            "entry_card_type": "48",
            "comment_scene": "39",
            "appid": "0",
            "token": token,
            "entry_scene": "0",
            "eid": export_id,
        }
    )
    status, body = _post_json(
        f"{FINDER_FEED_ENDPOINT}?{query}",
        {"baseReq": {"generalToken": token}, "exportId": export_id},
        {
            "Accept": "application/json, text/plain, */*",
            "Content-Type": "application/json",
            "Origin": "https://channels.weixin.qq.com",
            "Referer": f"{FINDER_PAGE}?{referer_query}",
            "User-Agent": (
                "Mozilla/5.0 (Windows NT 10.0; Win64; x64) "
                "AppleWebKit/537.36 (KHTML, like Gecko) Chrome/126.0.0.0 Safari/537.36"
            ),
        },
        timeout,
    )
    if not 200 <= status < 300:
        raise WechatChannelsAccessError(f"wechat_channels_feed_http_error:{status}")
    if body.get("errCode") != 0:
        raise WechatChannelsAccessError(
            f"wechat_channels_feed_error:{body.get('errCode')}"
        )
    data = body.get("data")
    feed = data.get("feedInfo") if isinstance(data, dict) else None
    author_info = data.get("authorInfo") if isinstance(data, dict) else None
    if not isinstance(feed, dict):
        raise WechatChannelsAccessError("wechat_channels_feed_schema_changed")

    videos: list[tuple[str, str]] = []
    seen_videos: set[str] = set()
    for source, raw in (
        ("h264VideoInfo.videoUrl", _nested_video_url(feed, "h264VideoInfo")),
        ("h265VideoInfo.videoUrl", _nested_video_url(feed, "h265VideoInfo")),
        ("videoUrl", feed.get("videoUrl")),
    ):
        url = _valid_media_url(raw)
        if url and url not in seen_videos:
            seen_videos.add(url)
            videos.append((source, url))

    images: list[str] = []
    if not videos:
        for item in feed.get("picInfo") or []:
            raw = item.get("url") if isinstance(item, dict) else None
            url = _valid_media_url(raw)
            if url and url not in images:
                images.append(url)
    if not videos and not images:
        raise WechatChannelsAccessError("wechat_channels_media_not_found")

    author = ""
    if isinstance(author_info, dict) and isinstance(author_info.get("nickname"), str):
        author = author_info["nickname"].strip()
    description = str(feed.get("description") or "").strip()
    cover_url = _valid_media_url(feed.get("coverUrl"))
    return author, description, tuple(videos), tuple(images), cover_url


def resolve_share(
    share_text: str,
    *,
    browser_profile_root: str | Path | None,
    chrome: str | None,
    timeout: float,
    browser_fallback: bool,
    cancellation_check: Callable[[], None] | None = None,
) -> ResolvedWechatChannels:
    share_url = extract_share_url(share_text)
    if not share_url:
        raise WechatChannelsAccessError("invalid_wechat_channels_share_url")
    if not chrome:
        raise WechatChannelsAccessError(
            "A Chromium-compatible browser is required for WeChat Channels downloads."
        )
    profile_dir = persistent_profile_dir(browser_profile_root)
    if profile_dir is None:
        raise WechatChannelsAccessError(LOGIN_REQUIRED)

    logs = ["wechat_channels: resolving through skill-owned Yuanbao session"]
    with profile_lock(profile_dir, cancellation_check):
        try:
            parsed = parse_share_with_browser(
                chrome=chrome,
                profile_dir=profile_dir,
                share_url=share_url,
                visible=False,
                request_timeout=timeout,
                cancellation_check=cancellation_check,
            )
            logs.append("wechat_channels: reused saved Yuanbao login")
        except WechatChannelsAccessError as exc:
            if str(exc) != LOGIN_REQUIRED:
                raise
            if not browser_fallback:
                raise
            environment = xiaohongshu_access.desktop_session_environment()
            if not xiaohongshu_access.desktop_display_available(environment):
                raise WechatChannelsAccessError(DISPLAY_UNAVAILABLE) from exc
            logs.append("wechat_channels: opening interactive Yuanbao login")
            parsed = parse_share_with_browser(
                chrome=chrome,
                profile_dir=profile_dir,
                share_url=share_url,
                visible=True,
                request_timeout=timeout,
                cancellation_check=cancellation_check,
            )
            logs.append("wechat_channels: interactive Yuanbao login completed")

    playable_url = parsed.get("playable_url")
    if not isinstance(playable_url, str):
        raise WechatChannelsAccessError("wechat_channels_parse_schema_changed")
    token, export_id = parse_preview_url(playable_url)
    del token
    author, description, videos, images, cover_url = fetch_feed_info(
        playable_url,
        timeout=timeout,
    )
    fallback_author = parsed.get("author")
    fallback_description = parsed.get("desc")
    if not author and isinstance(fallback_author, str):
        author = fallback_author.strip()
    if not description and isinstance(fallback_description, str):
        description = fallback_description.strip()
    logs.append("wechat_channels: resolved media from Channels feed")
    return ResolvedWechatChannels(
        share_url=share_url,
        export_id=export_id,
        author=author,
        description=description,
        video_urls=videos,
        image_urls=images,
        cover_url=cover_url,
        logs=tuple(logs),
    )
