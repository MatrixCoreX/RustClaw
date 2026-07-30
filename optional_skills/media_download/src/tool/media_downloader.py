#!/usr/bin/env python3
"""
Download an accessible short-video post from a copied share message.

This tool does not crack DRM, bypass private posts, or remove a watermark from an
already-downloaded file. It extracts public playback candidates from pages/API
responses that the user can access and prefers non-watermark playback URLs.
"""

from __future__ import annotations

import argparse
import base64
import copy
import html
import http.cookiejar
import json
import os
import queue
import re
import shlex
import shutil
import sqlite3
import subprocess
import sys
import tempfile
import threading
import time
import urllib.error
import urllib.parse
import urllib.request
from dataclasses import dataclass, field
from pathlib import Path
from typing import Any, Callable, Iterable

import image_ocr
import video_transcriber
from browser_devtools import DevToolsConnection, DevToolsError
from task_cancellation import CancellationToken, OperationCancelled, terminate_process

try:
    import readline
except ImportError:  # pragma: no cover - depends on platform Python build.
    readline = None  # type: ignore[assignment]


DEFAULT_HEADERS = {
    "User-Agent": (
        "Mozilla/5.0 (Windows NT 10.0; Win64; x64) "
        "AppleWebKit/537.36 (KHTML, like Gecko) "
        "Chrome/126.0.0.0 Safari/537.36"
    ),
    "Accept": (
        "text/html,application/xhtml+xml,application/xml;q=0.9,"
        "application/json;q=0.8,*/*;q=0.7"
    ),
    "Accept-Language": "zh-CN,zh;q=0.9,en;q=0.8",
    "Referer": "https://www.douyin.com/",
}

SHARE_URL_RE = re.compile(r"https?://[^\s\"'<>，。；：！？）】》、]+")
SCRIPT_RENDER_DATA_RE = re.compile(
    r'<script[^>]+id=["\']RENDER_DATA["\'][^>]*>(.*?)</script>',
    re.IGNORECASE | re.DOTALL,
)
JSON_SCRIPT_RE = re.compile(
    r'<script[^>]+type=["\']application/json["\'][^>]*>(.*?)</script>',
    re.IGNORECASE | re.DOTALL,
)
SCRIPT_CONTENT_RE = re.compile(r"<script[^>]*>(.*?)</script>", re.IGNORECASE | re.DOTALL)
AWEME_ID_PATTERNS = (
    re.compile(r"/(?:video|note)/(\d{10,})"),
    re.compile(r"[?&](?:aweme_id|modal_id|item_ids|item_id)=(\d{10,})"),
    re.compile(r'"(?:aweme_id|awemeId|item_id|itemId)"\s*:\s*"?(\d{10,})"?'),
)
DOUYIN_PROFILE_PATH_RE = re.compile(r"/user/([^/?#]+)")
XIAOHONGSHU_PROFILE_PATH_RE = re.compile(r"/user/profile/([^/?#]+)")
KUAISHOU_ID_PATTERNS = (
    re.compile(r"/short-video/([^/?#]+)"),
    re.compile(r"[?&](?:photoId|photo_id)=([^&#]+)"),
    re.compile(r'"(?:photoId|photo_id)"\s*:\s*"([^"]+)"'),
)
XIAOHONGSHU_ID_PATTERNS = (
    re.compile(r"/(?:explore|discovery/item)/([0-9a-zA-Z]+)"),
    re.compile(r"[?&](?:note_id|noteId|item_id)=([^&#]+)"),
    re.compile(r'"(?:noteId|note_id|itemId|item_id)"\s*:\s*"([^"]+)"'),
)
TIKTOK_ID_PATTERNS = (
    re.compile(r"/@[^/?#]+/video/(\d{10,})"),
    re.compile(r"/video/(\d{10,})"),
    re.compile(r"[?&](?:item_id|itemId|video_id|videoId)=(\d{10,})"),
    re.compile(r'"(?:id|itemId|item_id|videoId|video_id)"\s*:\s*"?(\d{10,})"?'),
)
YOUTUBE_ID_PATTERNS = (
    re.compile(r"youtu\.be/([0-9A-Za-z_-]{11})"),
    re.compile(r"/(?:shorts|embed|live)/([0-9A-Za-z_-]{11})"),
    re.compile(r"[?&]v=([0-9A-Za-z_-]{11})"),
    re.compile(r'"(?:videoId|video_id)"\s*:\s*"([0-9A-Za-z_-]{11})"'),
)
JS_STATE_MARKERS = (
    "__INITIAL_STATE__",
    "__APOLLO_STATE__",
    "__NEXT_DATA__",
    "__NUXT__",
    "__INITIAL_DATA__",
    "__INITIAL_DATA_FOR_REHYDRATION__",
    "__UNIVERSAL_DATA_FOR_REHYDRATION__",
    "SIGI_STATE",
)
DOUYIN_DOMAINS = ("douyin.com", "iesdouyin.com")
KUAISHOU_DOMAINS = ("kuaishou.com", "gifshow.com", "ksurl.cn", "kwai.com", "v.kuaishou.com")
XIAOHONGSHU_DOMAINS = ("xiaohongshu.com", "xhslink.com", "xhscdn.com", "xhs.cn")
TIKTOK_DOMAINS = ("tiktok.com", "tiktokv.com", "tiktokcdn.com", "vm.tiktok.com", "vt.tiktok.com")
YOUTUBE_DOMAINS = ("youtube.com", "youtu.be", "youtube-nocookie.com")
PLATFORMS = ("auto", "douyin", "kuaishou", "xiaohongshu", "tiktok", "youtube")
PLATFORM_ALIASES = {"titok": "tiktok", "yt": "youtube"}
PLATFORM_CHOICES = PLATFORMS + tuple(PLATFORM_ALIASES)
DEFAULT_YOUTUBE_FORMAT = "bv*[ext=mp4]+ba[ext=m4a]/b[ext=mp4]/bv*+ba/b"
OUTPUT_TIME_FORMAT = "%Y%m%d_%H%M%S"
PARSE_RETRY_COUNT = 3
DEFAULT_BROWSER_TIMEOUT = 30.0
DEFAULT_PROFILE_LIMIT = 100
PROFILE_MANIFEST_FILENAME = "profile_downloads.json"
PROFILE_VIDEO_FOLDER = "videos"
PROFILE_IMAGE_FOLDER = "images"
DEFAULT_PROFILE_INTERVAL = 5.0
INTERACTIVE_HISTORY_LIMIT = 1000
INTERACTIVE_HISTORY_ENV = "MEDIA_DOWNLOADER_HISTORY"
INTERACTIVE_PROMPT = "media> "
SCRIPT_DIRECTORY = Path(__file__).resolve().parent
CHROMIUM_BROWSER_EXECUTABLES = (
    "google-chrome",
    "google-chrome-stable",
    "chromium",
    "chromium-browser",
    "microsoft-edge",
    "microsoft-edge-stable",
    "msedge",
    "brave-browser",
    "brave",
    "vivaldi",
    "vivaldi-stable",
    "chrome.exe",
    "msedge.exe",
    "brave.exe",
    "/Applications/Google Chrome.app/Contents/MacOS/Google Chrome",
    "/Applications/Chromium.app/Contents/MacOS/Chromium",
    "/Applications/Microsoft Edge.app/Contents/MacOS/Microsoft Edge",
    "/Applications/Brave Browser.app/Contents/MacOS/Brave Browser",
)


class DouyinDownloadError(RuntimeError):
    """Raised when the download cannot be completed."""


_TASK_CONTEXT = threading.local()


def current_cancellation_token() -> CancellationToken | None:
    return getattr(_TASK_CONTEXT, "cancel_token", None)


def raise_if_task_cancelled() -> None:
    token = current_cancellation_token()
    if token is not None:
        token.raise_if_cancelled()


def run_task_subprocess(
    command: list[str],
    *,
    check: bool = False,
    capture_output: bool = False,
    text: bool = False,
    timeout: float | None = None,
) -> subprocess.CompletedProcess[Any]:
    """Run a child process normally, but make interactive tasks immediately cancellable."""
    token = current_cancellation_token()
    if token is None:
        return subprocess.run(
            command,
            check=check,
            capture_output=capture_output,
            text=text,
            timeout=timeout,
        )

    token.raise_if_cancelled()
    process = subprocess.Popen(
        command,
        stdout=subprocess.PIPE if capture_output else None,
        stderr=subprocess.PIPE if capture_output else None,
        text=text,
        start_new_session=os.name == "posix",
    )
    try:
        token.register_process(process)
        try:
            stdout, stderr = process.communicate(timeout=timeout)
        except subprocess.TimeoutExpired:
            terminate_process(process)
            try:
                stdout, stderr = process.communicate(timeout=1)
            except subprocess.TimeoutExpired:
                terminate_process(process, force=True)
                stdout, stderr = process.communicate()
            raise subprocess.TimeoutExpired(command, timeout, output=stdout, stderr=stderr)
        token.raise_if_cancelled()
    finally:
        token.unregister_process(process)

    completed = subprocess.CompletedProcess(command, process.returncode, stdout, stderr)
    if check and completed.returncode != 0:
        raise subprocess.CalledProcessError(
            completed.returncode,
            command,
            output=stdout,
            stderr=stderr,
        )
    return completed


@dataclass(frozen=True)
class FetchResult:
    url: str
    status: int
    content: bytes
    headers: dict[str, str]


@dataclass(frozen=True)
class Candidate:
    url: str
    source: str
    priority: int
    cookie: str | None = None
    referer: str | None = None
    live_photo_audio_url: str | None = None
    live_photo_duration: float | None = None


@dataclass(frozen=True)
class ImageCandidate:
    url: str
    source: str
    priority: int


@dataclass(frozen=True)
class DouyinProfilePost:
    item_id: str
    create_time: int
    payload: dict[str, Any] | None = None


@dataclass(frozen=True)
class DouyinProfileResult:
    sec_uid: str
    username: str
    posts: list[DouyinProfilePost]
    logs: list[str]


@dataclass(frozen=True)
class XiaohongshuProfilePost:
    item_id: str
    create_time: int
    xsec_token: str
    note_type: str
    payload: dict[str, Any] | None = None


@dataclass(frozen=True)
class XiaohongshuProfileResult:
    user_id: str
    username: str
    posts: list[XiaohongshuProfilePost]
    logs: list[str]


@dataclass(frozen=True)
class SystemBrowserCookieImport:
    browser: str
    profile: str
    cookie_count: int


def extract_urls(text: str) -> list[str]:
    """Extract HTTP URLs from a copied share message."""
    urls: list[str] = []
    for match in SHARE_URL_RE.finditer(text):
        url = match.group(0).rstrip(".,;:!?)]}>\"'，。；：！？）】》、")
        if url and url not in urls:
            urls.append(url)
    return urls


def extract_aweme_id(*parts: str) -> str | None:
    """Find a Douyin aweme/video id in URLs, HTML, JSON snippets, or share text."""
    for part in parts:
        if not part:
            continue
        decoded = urllib.parse.unquote(html.unescape(part))
        for pattern in AWEME_ID_PATTERNS:
            match = pattern.search(decoded)
            if match:
                return match.group(1)
    return None


def extract_douyin_profile_target(text: str) -> tuple[str, str] | None:
    """Return the first Douyin profile URL and its sec_uid."""
    urls = extract_urls(text)
    if not urls and text.strip().startswith(("http://", "https://")):
        urls = [text.strip()]
    for url in urls:
        parsed = urllib.parse.urlsplit(url)
        host = parsed.netloc.lower().split(":", 1)[0]
        if not any(host == domain or host.endswith(f".{domain}") for domain in DOUYIN_DOMAINS):
            continue
        match = DOUYIN_PROFILE_PATH_RE.search(parsed.path)
        if match:
            return url, urllib.parse.unquote(match.group(1))
    return None


def extract_xiaohongshu_profile_target(text: str) -> tuple[str, str] | None:
    """Return the first Xiaohongshu profile URL and its user id."""
    urls = extract_urls(text)
    if not urls and text.strip().startswith(("http://", "https://")):
        urls = [text.strip()]
    for url in urls:
        parsed = urllib.parse.urlsplit(url)
        host = parsed.netloc.lower().split(":", 1)[0]
        if not any(host == domain or host.endswith(f".{domain}") for domain in XIAOHONGSHU_DOMAINS):
            continue
        match = XIAOHONGSHU_PROFILE_PATH_RE.search(parsed.path)
        if match:
            return url, urllib.parse.unquote(match.group(1))
    return None


def extract_kuaishou_id(*parts: str) -> str | None:
    for part in parts:
        if not part:
            continue
        decoded = urllib.parse.unquote(html.unescape(part))
        for pattern in KUAISHOU_ID_PATTERNS:
            match = pattern.search(decoded)
            if match:
                return match.group(1)
    return None


def extract_xiaohongshu_id(*parts: str) -> str | None:
    for part in parts:
        if not part:
            continue
        decoded = urllib.parse.unquote(html.unescape(part))
        for pattern in XIAOHONGSHU_ID_PATTERNS:
            match = pattern.search(decoded)
            if match:
                return match.group(1)
    return None


def extract_tiktok_id(*parts: str) -> str | None:
    for part in parts:
        if not part:
            continue
        decoded = urllib.parse.unquote(html.unescape(part))
        for pattern in TIKTOK_ID_PATTERNS:
            match = pattern.search(decoded)
            if match:
                return match.group(1)
    return None


def extract_youtube_id(*parts: str) -> str | None:
    for part in parts:
        if not part:
            continue
        decoded = urllib.parse.unquote(html.unescape(part))
        for pattern in YOUTUBE_ID_PATTERNS:
            match = pattern.search(decoded)
            if match:
                return match.group(1)
    return None


def normalize_platform(platform: str) -> str:
    return PLATFORM_ALIASES.get(platform, platform)


def detect_platform(text: str) -> str | None:
    lowered = text.lower()
    if any(domain in lowered for domain in YOUTUBE_DOMAINS):
        return "youtube"
    if any(domain in lowered for domain in KUAISHOU_DOMAINS):
        return "kuaishou"
    if any(domain in lowered for domain in XIAOHONGSHU_DOMAINS):
        return "xiaohongshu"
    if any(domain in lowered for domain in TIKTOK_DOMAINS):
        return "tiktok"
    if any(domain in lowered for domain in DOUYIN_DOMAINS):
        return "douyin"
    return None


def normalize_cookie(cookie: str | None) -> str | None:
    """Accept either a raw cookie header or a path to a cookie text file."""
    if not cookie:
        return None
    maybe_path = Path(cookie).expanduser()
    if maybe_path.exists() and maybe_path.is_file():
        return maybe_path.read_text(encoding="utf-8").strip()
    return cookie.strip()


def build_headers(cookie: str | None = None, extra: dict[str, str] | None = None) -> dict[str, str]:
    headers = dict(DEFAULT_HEADERS)
    if cookie:
        headers["Cookie"] = cookie
    if extra:
        headers.update(extra)
    return headers


def combine_cookie_headers(*cookies: str | None) -> str | None:
    parts = [cookie.strip().rstrip(";") for cookie in cookies if cookie and cookie.strip()]
    return "; ".join(parts) if parts else None


def cookie_header_from_jar(cookie_jar: http.cookiejar.CookieJar) -> str | None:
    return combine_cookie_headers(*[f"{cookie.name}={cookie.value}" for cookie in cookie_jar])


def http_get(
    url: str,
    *,
    cookie: str | None = None,
    timeout: float = 20.0,
    max_bytes: int | None = None,
    extra_headers: dict[str, str] | None = None,
) -> FetchResult:
    """Fetch a URL with browser-like headers and return its final URL and body."""
    headers = build_headers(cookie, extra_headers)
    request = urllib.request.Request(url, headers=headers)
    try:
        with urllib.request.urlopen(request, timeout=timeout) as response:
            if max_bytes is None:
                content = response.read()
            else:
                content = response.read(max_bytes + 1)
            return FetchResult(
                url=response.geturl(),
                status=response.status,
                content=content,
                headers={k.lower(): v for k, v in response.headers.items()},
            )
    except urllib.error.HTTPError as exc:
        detail = exc.read(512).decode("utf-8", errors="replace")
        raise DouyinDownloadError(f"HTTP {exc.code} for {url}: {detail}") from exc
    except urllib.error.URLError as exc:
        raise DouyinDownloadError(f"Network error for {url}: {exc.reason}") from exc


def http_get_with_session_cookies(
    url: str,
    *,
    cookie: str | None = None,
    timeout: float = 20.0,
    max_bytes: int | None = None,
    extra_headers: dict[str, str] | None = None,
) -> tuple[FetchResult, str | None]:
    """Fetch a page and return cookies set during that same request."""
    cookie_jar = http.cookiejar.CookieJar()
    opener = urllib.request.build_opener(urllib.request.HTTPCookieProcessor(cookie_jar))
    headers = build_headers(cookie, extra_headers)
    request = urllib.request.Request(url, headers=headers)
    try:
        with opener.open(request, timeout=timeout) as response:
            if max_bytes is None:
                content = response.read()
            else:
                content = response.read(max_bytes + 1)
            result = FetchResult(
                url=response.geturl(),
                status=response.status,
                content=content,
                headers={k.lower(): v for k, v in response.headers.items()},
            )
            return result, combine_cookie_headers(cookie, cookie_header_from_jar(cookie_jar))
    except urllib.error.HTTPError as exc:
        detail = exc.read(512).decode("utf-8", errors="replace")
        raise DouyinDownloadError(f"HTTP {exc.code} for {url}: {detail}") from exc
    except urllib.error.URLError as exc:
        raise DouyinDownloadError(f"Network error for {url}: {exc.reason}") from exc


def decode_text(content: bytes, headers: dict[str, str]) -> str:
    content_type = headers.get("content-type", "")
    match = re.search(r"charset=([\w.-]+)", content_type, re.IGNORECASE)
    encoding = match.group(1) if match else "utf-8"
    try:
        return content.decode(encoding, errors="replace")
    except LookupError:
        return content.decode("utf-8", errors="replace")


def load_json_bytes(content: bytes) -> Any | None:
    try:
        return json.loads(content.decode("utf-8"))
    except (UnicodeDecodeError, json.JSONDecodeError):
        return None


def extract_balanced_json_text(text: str, start: int) -> str | None:
    if start < 0 or start >= len(text) or text[start] not in "{[":
        return None
    opening = text[start]
    closing = "}" if opening == "{" else "]"
    stack = [closing]
    in_string = False
    quote = ""
    escaped = False

    for index in range(start + 1, len(text)):
        char = text[index]
        if in_string:
            if escaped:
                escaped = False
            elif char == "\\":
                escaped = True
            elif char == quote:
                in_string = False
            continue

        if char in {"'", '"'}:
            in_string = True
            quote = char
            continue
        if char in "{[":
            stack.append("}" if char == "{" else "]")
            continue
        if char in "}]":
            if not stack or char != stack[-1]:
                return None
            stack.pop()
            if not stack:
                return text[start : index + 1]
    return None


def load_json_text(raw: str) -> Any | None:
    text = html.unescape(raw.strip())
    candidates = [text]
    try:
        candidates.append(text.encode("utf-8").decode("unicode_escape"))
    except UnicodeDecodeError:
        pass

    for candidate in candidates:
        try:
            return json.loads(candidate)
        except json.JSONDecodeError:
            continue
    return None


def extract_json_from_html(page_text: str) -> list[Any]:
    """Extract JSON payloads commonly embedded in short-video pages."""
    payloads: list[Any] = []

    for match in SCRIPT_RENDER_DATA_RE.finditer(page_text):
        raw = urllib.parse.unquote(html.unescape(match.group(1)))
        payload = load_json_text(raw)
        if payload is not None:
            payloads.append(payload)

    for match in JSON_SCRIPT_RE.finditer(page_text):
        payload = load_json_text(match.group(1))
        if payload is not None:
            payloads.append(payload)

    for match in SCRIPT_CONTENT_RE.finditer(page_text):
        script_text = html.unescape(match.group(1))
        for marker in JS_STATE_MARKERS:
            marker_index = script_text.find(marker)
            if marker_index < 0:
                continue
            equals_index = script_text.find("=", marker_index)
            search_start = equals_index + 1 if equals_index >= 0 else marker_index + len(marker)
            object_index = len(script_text)
            for opener in ("{", "["):
                index = script_text.find(opener, search_start)
                if index >= 0:
                    object_index = min(object_index, index)
            if object_index == len(script_text):
                continue
            raw_json = extract_balanced_json_text(script_text, object_index)
            if not raw_json:
                continue
            payload = load_json_text(raw_json)
            if payload is not None:
                payloads.append(payload)

    return payloads


def iter_dicts(value: Any) -> Iterable[dict[str, Any]]:
    if isinstance(value, dict):
        yield value
        for child in value.values():
            yield from iter_dicts(child)
    elif isinstance(value, list):
        for child in value:
            yield from iter_dicts(child)


def iter_strings(value: Any) -> Iterable[str]:
    if isinstance(value, str):
        yield value
    elif isinstance(value, dict):
        for child in value.values():
            yield from iter_strings(child)
    elif isinstance(value, list):
        for child in value:
            yield from iter_strings(child)


def unwrap_url(url: str, *, decode_percent: bool = True) -> str:
    """Decode escaped JSON/HTML URL strings and add a scheme when needed."""
    current = (
        html.unescape(url)
        .replace("\\u002F", "/")
        .replace("\\u002f", "/")
        .replace("\\u0026", "&")
        .replace("\\u003D", "=")
        .replace("\\u003d", "=")
        .replace("\\u003F", "?")
        .replace("\\u003f", "?")
        .replace("\\/", "/")
    )
    if decode_percent:
        current = urllib.parse.unquote(current)
    if current.startswith("//"):
        current = "https:" + current
    return current


def prefer_no_watermark_url(url: str) -> str:
    """Prefer Douyin's non-watermark playback form when a watermarked URL is found."""
    url = unwrap_url(url)
    url = url.replace("/playwm/", "/play/")
    url = url.replace("playwm", "play")

    parsed = urllib.parse.urlsplit(url)
    pairs = urllib.parse.parse_qsl(parsed.query, keep_blank_values=True)
    new_pairs = []
    for key, value in pairs:
        if key.lower() in {"watermark", "logo"} and value == "1":
            value = "0"
        new_pairs.append((key, value))
    query = urllib.parse.urlencode(new_pairs)
    return urllib.parse.urlunsplit((parsed.scheme, parsed.netloc, parsed.path, query, parsed.fragment))


def looks_like_play_url(url: str) -> bool:
    lowered = url.lower()
    return (
        lowered.startswith(("http://", "https://", "//"))
        and ("douyin" in lowered or "byte" in lowered or "ixigua" in lowered)
        and any(token in lowered for token in ("/play/", "/playwm/", "aweme/v1/play", "video_id="))
    )


def looks_like_douyin_browser_video_url(url: str) -> bool:
    lowered = unwrap_url(url).lower()
    if not lowered.startswith(("http://", "https://", "//")):
        return False
    parsed = urllib.parse.urlsplit(lowered)
    host = parsed.netloc
    path = parsed.path
    if "media-audio" in path or "mp4a" in path:
        return False
    if "douyinvod.com" in host:
        return "mime_type=video" in lowered or ".mp4" in path or "/video/tos/" in path
    if host.endswith("douyinstatic.com"):
        return False
    return "/video/tos/" in path and "mime_type=video" in lowered


def looks_like_video_only_stream_url(url: str) -> bool:
    path = urllib.parse.urlsplit(unwrap_url(url)).path.lower()
    return "media-video-" in path


def looks_like_kuaishou_video_url(url: str) -> bool:
    lowered = unwrap_url(url).lower()
    if not lowered.startswith(("http://", "https://", "//")):
        return False
    if not any(token in lowered for token in ("kuaishou", "gifshow", "kwai", "kwimg", "ksyuncdn")):
        return False
    return any(token in lowered for token in (".mp4", "video", "clientcachekey", "mvurl"))


def looks_like_xiaohongshu_video_url(url: str) -> bool:
    lowered = unwrap_url(url).lower()
    if not lowered.startswith(("http://", "https://", "//")):
        return False
    parsed = urllib.parse.urlsplit(lowered)
    host = parsed.netloc
    path = parsed.path
    if not path or path == "/":
        return False
    if path.endswith((".ico", ".json", ".pdf", ".js", ".css", ".png", ".jpg", ".jpeg", ".webp", ".svg")):
        return False
    if "sns-video" in host or "redcdn" in host:
        return ".mp4" in path or "stream" in path or "video" in path
    return ".mp4" in path and any(token in host for token in ("xiaohongshu", "xhscdn", "xhs"))


def looks_like_tiktok_video_url(url: str) -> bool:
    lowered = unwrap_url(url).lower()
    if not lowered.startswith(("http://", "https://", "//")):
        return False
    parsed = urllib.parse.urlsplit(lowered)
    host = parsed.netloc
    path = parsed.path
    query = parsed.query
    if not path or path == "/":
        return False
    if path.endswith((".ico", ".json", ".pdf", ".js", ".css", ".png", ".jpg", ".jpeg", ".webp", ".svg")):
        return False
    if "tiktok.com" in host and "/@" in path and "/video/" in path:
        return False

    cdn_host = (
        any(token in host for token in ("tiktokcdn", "tiktokv", "byteoversea", "muscdn", "snssdk", "akamaized"))
        or (host.startswith(("v16", "v19", "v26")) and "tiktok" in host)
        or ("tiktok.com" in host and "/video/tos/" in path)
    )
    if not cdn_host:
        return False
    return ".mp4" in path or "mime_type=video" in query or "/video/tos/" in path


def looks_like_platform_video_url(url: str, platform: str) -> bool:
    platform = normalize_platform(platform)
    if platform == "kuaishou":
        return looks_like_kuaishou_video_url(url)
    if platform == "xiaohongshu":
        return looks_like_xiaohongshu_video_url(url)
    if platform == "tiktok":
        return looks_like_tiktok_video_url(url)
    return looks_like_play_url(url) or looks_like_douyin_browser_video_url(url)


def looks_like_douyin_image_url(url: str) -> bool:
    normalized = unwrap_url(url, decode_percent=False)
    lowered = normalized.lower()
    if not lowered.startswith(("http://", "https://", "//")):
        return False
    parsed = urllib.parse.urlsplit(lowered)
    if not any(token in parsed.netloc for token in ("douyinpic.com", "byteimg.com")):
        return False
    if "biz_tag=aweme_images" not in lowered and "tplv-dy-aweme-images" not in lowered:
        return False
    if "tplv-dy-water" in lowered or "water-v" in lowered:
        return False
    if "aweme_comment" in lowered or "aweme-avatar" in lowered:
        return False
    return True


def looks_like_xiaohongshu_image_url(url: str) -> bool:
    normalized = unwrap_url(url, decode_percent=False)
    lowered = normalized.lower()
    if not lowered.startswith(("http://", "https://", "//")):
        return False
    parsed = urllib.parse.urlsplit(lowered)
    host = parsed.netloc
    path = parsed.path
    if not path or path == "/":
        return False
    if "xhscdn.com" not in host and "xhscdn.net" not in host and "xiaohongshu.com" not in host:
        return False
    if not any(token in host for token in ("sns-webpic", "sns-img", "ci.xiaohongshu", "sns-avatar")):
        return False
    if "sns-avatar" in host or "/avatar/" in path:
        return False
    if "sns-webpic" not in host and "sns-img" not in host and "ci.xiaohongshu" not in host:
        return False
    if "fe-video" in host or "sns-video" in host:
        return False
    if path.endswith((".ico", ".json", ".pdf", ".js", ".css", ".svg")):
        return False
    return True


def douyin_image_key(url: str) -> str:
    parsed = urllib.parse.urlsplit(unwrap_url(url, decode_percent=False))
    return parsed.path.split("~", 1)[0]


def douyin_image_quality_score(url: str) -> int:
    lowered = unwrap_url(url, decode_percent=False).lower()
    score = 0
    if "x-signature=" not in lowered:
        score += 100
    if "tplv-dy-aweme-images" not in lowered:
        score += 50
    if ".webp" in urllib.parse.urlsplit(lowered).path:
        score += 0
    elif ".jpeg" in urllib.parse.urlsplit(lowered).path or ".jpg" in urllib.parse.urlsplit(lowered).path:
        score += 5
    else:
        score += 20
    return score


def xiaohongshu_image_key(url: str) -> str:
    parsed = urllib.parse.urlsplit(unwrap_url(url, decode_percent=False))
    filename = parsed.path.rsplit("/", 1)[-1]
    if filename:
        return filename.split("!", 1)[0]
    return parsed.path


def xiaohongshu_image_quality_score(url: str) -> int:
    lowered = unwrap_url(url, decode_percent=False).lower()
    if "nd_dft" in lowered or "dft" in lowered:
        return 0
    if "nd_wm" in lowered or "watermark" in lowered:
        return 80
    if "nd_prv" in lowered or "prv" in lowered:
        return 100
    return 50


def add_image_candidate(
    candidates_by_key: dict[str, ImageCandidate],
    url: str,
    source: str,
    order: int,
) -> None:
    normalized = unwrap_url(url, decode_percent=False)
    if not looks_like_douyin_image_url(normalized):
        return
    key = douyin_image_key(normalized)
    quality = douyin_image_quality_score(normalized)
    existing = candidates_by_key.get(key)
    if existing:
        existing_order = existing.priority // 1000
        existing_quality = existing.priority % 1000
        if quality >= existing_quality:
            return
        order = existing_order
    candidates_by_key[key] = ImageCandidate(normalized, source, order * 1000 + quality)


def add_xiaohongshu_image_candidate(
    candidates_by_key: dict[str, ImageCandidate],
    url: str,
    source: str,
    order: int,
) -> None:
    normalized = unwrap_url(url, decode_percent=False)
    if not looks_like_xiaohongshu_image_url(normalized):
        return
    key = xiaohongshu_image_key(normalized)
    quality = xiaohongshu_image_quality_score(normalized)
    existing = candidates_by_key.get(key)
    if existing:
        existing_order = existing.priority // 1000
        existing_quality = existing.priority % 1000
        if quality >= existing_quality:
            return
        order = existing_order
    candidates_by_key[key] = ImageCandidate(normalized, source, order * 1000 + quality)


def normalize_embedded_url_text(page_text: str) -> str:
    return (
        html.unescape(page_text)
        .replace("\\u002F", "/")
        .replace("\\u002f", "/")
        .replace("\\u0026", "&")
        .replace("\\u003D", "=")
        .replace("\\u003d", "=")
        .replace("\\u003F", "?")
        .replace("\\u003f", "?")
        .replace("\\/", "/")
    )


def extract_douyin_image_candidates_from_text(page_text: str, source: str = "douyin.html-image") -> list[ImageCandidate]:
    normalized_text = normalize_embedded_url_text(page_text)
    url_pattern = re.compile(r"https?://[^\"'<>\\\s]+")
    candidates_by_key: dict[str, ImageCandidate] = {}
    order = 0
    for match in url_pattern.finditer(normalized_text):
        raw_url = match.group(0).rstrip(".,;:!?)]}>\"'，。；：！？）】》、")
        if not looks_like_douyin_image_url(raw_url):
            continue
        add_image_candidate(candidates_by_key, raw_url, source, order)
        order += 1
    return sorted(candidates_by_key.values(), key=lambda candidate: candidate.priority)


def extract_xiaohongshu_image_candidates_from_text(
    page_text: str,
    source: str = "xiaohongshu.html-image",
) -> list[ImageCandidate]:
    normalized_text = normalize_embedded_url_text(page_text)
    url_pattern = re.compile(r"https?://[^\"'<>\\\s]+")
    candidates_by_key: dict[str, ImageCandidate] = {}
    order = 0
    for match in url_pattern.finditer(normalized_text):
        raw_url = match.group(0).rstrip(".,;:!?)]}>\"'，。；：！？）】》、")
        if not looks_like_xiaohongshu_image_url(raw_url):
            continue
        add_xiaohongshu_image_candidate(candidates_by_key, raw_url, source, order)
        order += 1
    return sorted(candidates_by_key.values(), key=lambda candidate: candidate.priority)


def add_candidate(
    candidates: list[Candidate],
    seen: set[str],
    url: str,
    source: str,
    priority: int,
    *,
    cookie: str | None = None,
    referer: str | None = None,
    live_photo_audio_url: str | None = None,
    live_photo_duration: float | None = None,
) -> None:
    normalized = prefer_no_watermark_url(url)
    if not looks_like_play_url(normalized):
        return
    if normalized in seen:
        return
    seen.add(normalized)
    candidates.append(
        Candidate(
            normalized,
            source,
            priority,
            cookie,
            referer,
            live_photo_audio_url,
            live_photo_duration,
        )
    )


def add_platform_candidate(
    candidates: list[Candidate],
    seen: set[str],
    url: str,
    source: str,
    priority: int,
    platform: str,
    *,
    cookie: str | None = None,
    referer: str | None = None,
    live_photo_audio_url: str | None = None,
    live_photo_duration: float | None = None,
) -> None:
    normalized = unwrap_url(url)
    if not looks_like_platform_video_url(normalized, platform):
        return
    if normalized in seen:
        return
    seen.add(normalized)
    candidates.append(
        Candidate(
            normalized,
            source,
            priority,
            cookie,
            referer,
            live_photo_audio_url,
            live_photo_duration,
        )
    )


def extract_candidates_from_json(value: Any) -> list[Candidate]:
    candidates: list[Candidate] = []
    seen: set[str] = set()

    preferred_addr_keys = {
        "play_addr": 10,
        "play_addr_h264": 12,
        "download_addr": 30,
        "play_api": 20,
    }

    for item in iter_dicts(value):
        for key, priority in preferred_addr_keys.items():
            addr = item.get(key)
            if not isinstance(addr, dict):
                continue
            urls = addr.get("url_list")
            if isinstance(urls, list):
                for raw_url in urls:
                    if isinstance(raw_url, str):
                        add_candidate(candidates, seen, raw_url, key, priority)
            uri = addr.get("uri")
            if isinstance(uri, str) and uri.startswith(("http://", "https://", "//")):
                add_candidate(candidates, seen, uri, key, priority + 5)

        bit_rates = item.get("bit_rate")
        if isinstance(bit_rates, list):
            for index, bit_rate in enumerate(bit_rates):
                if not isinstance(bit_rate, dict):
                    continue
                addr = bit_rate.get("play_addr") or bit_rate.get("play_addr_h264")
                if not isinstance(addr, dict):
                    continue
                urls = addr.get("url_list")
                if isinstance(urls, list):
                    for raw_url in urls:
                        if isinstance(raw_url, str):
                            add_candidate(
                                candidates,
                                seen,
                                raw_url,
                                f"bit_rate[{index}]",
                                5 + index,
                            )

    for raw_url in iter_strings(value):
        if looks_like_play_url(unwrap_url(raw_url)):
            add_candidate(candidates, seen, raw_url, "json-string", 50)

    return sorted(candidates, key=lambda candidate: candidate.priority)


def find_douyin_aweme_payload(value: Any, item_id: str | None) -> dict[str, Any] | None:
    """Find the richest exact Aweme object in a page or API response."""
    if not item_id:
        return None
    matches = [
        item
        for item in iter_dicts(value)
        if str(item.get("aweme_id") or item.get("awemeId") or "") == item_id
    ]
    if not matches:
        return None

    def payload_score(item: dict[str, Any]) -> tuple[int, int, int]:
        images = item.get("images")
        video = item.get("video")
        media_score = 0
        if isinstance(images, list) and images:
            media_score += 4
        if isinstance(video, dict) and video:
            media_score += 2
        if item.get("author") or item.get("statistics"):
            media_score += 1
        return media_score, len(item), len(json.dumps(item, ensure_ascii=False, separators=(",", ":")))

    return max(matches, key=payload_score)


def find_douyin_aweme_payload_in_html(page_text: str, item_id: str | None) -> dict[str, Any] | None:
    matches = [
        payload
        for value in extract_json_from_html(page_text)
        if (payload := find_douyin_aweme_payload(value, item_id)) is not None
    ]
    if not matches:
        return None
    return max(matches, key=lambda item: len(item))


def douyin_flag_enabled(value: Any) -> bool:
    return value is True or value == 1 or (isinstance(value, str) and value.strip() == "1")


def douyin_payload_is_live_photo(payload: dict[str, Any]) -> bool:
    if douyin_flag_enabled(payload.get("is_live_photo")):
        return True
    images = payload.get("images")
    return isinstance(images, list) and any(
        isinstance(image, dict) and douyin_flag_enabled(image.get("live_photo_type"))
        for image in images
    )


def first_http_url_from_address(value: Any) -> str | None:
    if not isinstance(value, dict):
        return None
    urls = value.get("url_list") or value.get("urlList") or []
    if isinstance(urls, list):
        for raw_url in urls:
            if isinstance(raw_url, str):
                normalized = unwrap_url(raw_url)
                if normalized.startswith(("http://", "https://")):
                    return normalized
    uri = value.get("uri")
    if isinstance(uri, str):
        normalized = unwrap_url(uri)
        if normalized.startswith(("http://", "https://")):
            return normalized
    return None


def douyin_live_photo_composition(payload: dict[str, Any]) -> tuple[str | None, float | None]:
    album_music = payload.get("image_album_music_info")
    duration: float | None = None
    if isinstance(album_music, dict):
        try:
            begin_time = float(album_music.get("begin_time") or 0)
            end_time = float(album_music.get("end_time") or 0)
        except (TypeError, ValueError):
            begin_time = 0
            end_time = 0
        if end_time > begin_time:
            duration = (end_time - begin_time) / 1000.0

    music = payload.get("music")
    if duration is None and isinstance(music, dict):
        try:
            music_duration = float(music.get("duration") or 0)
        except (TypeError, ValueError):
            music_duration = 0
        if music_duration > 0:
            duration = music_duration

    audio_url: str | None = None
    if isinstance(music, dict):
        audio_url = first_http_url_from_address(music.get("play_url"))
    top_video = payload.get("video")
    if audio_url is None and isinstance(top_video, dict):
        audio_url = first_http_url_from_address(top_video.get("play_addr"))
    return audio_url, duration


def extract_douyin_video_payload_candidates(
    video_payload: dict[str, Any],
    *,
    source: str,
) -> list[Candidate]:
    candidates = list(extract_candidates_from_json(video_payload))
    seen = {candidate.url for candidate in candidates}
    for index, raw_url in enumerate(iter_strings(video_payload)):
        if not looks_like_platform_video_url(raw_url, "douyin"):
            continue
        add_platform_candidate(
            candidates,
            seen,
            prefer_no_watermark_url(raw_url),
            f"{source}.video-url",
            browser_candidate_priority(raw_url, "douyin", index),
            "douyin",
        )

    quality_ranked: list[Candidate] = []
    for index, candidate in enumerate(candidates):
        priority = douyin_payload_candidate_priority(candidate, index)
        candidate_source = candidate.source
        if not candidate_source.startswith(f"{source}."):
            candidate_source = f"{source}.{candidate_source}"
        quality_ranked.append(
            Candidate(
                candidate.url,
                candidate_source,
                priority,
                candidate.cookie,
                candidate.referer,
            )
        )
    return sorted(quality_ranked, key=lambda candidate: candidate.priority)


def extract_douyin_item_media_candidates(
    payload: dict[str, Any],
    *,
    source: str,
) -> tuple[list[Candidate], list[ImageCandidate]]:
    """Extract only this Aweme's media, including an embedded live-photo video."""
    raw_images = payload.get("images")
    if isinstance(raw_images, list) and raw_images:
        if douyin_payload_is_live_photo(payload):
            live_photo_images = [
                (index, image)
                for index, image in enumerate(raw_images)
                if isinstance(image, dict)
                and douyin_flag_enabled(image.get("live_photo_type"))
                and isinstance(image.get("video"), dict)
            ]
            if not live_photo_images and douyin_flag_enabled(payload.get("is_live_photo")):
                live_photo_images = [
                    (index, image)
                    for index, image in enumerate(raw_images)
                    if isinstance(image, dict) and isinstance(image.get("video"), dict)
                ]
            for index, image in live_photo_images:
                live_photo_candidates = extract_douyin_video_payload_candidates(
                    image["video"],
                    source=f"{source}.live-photo[{index}]",
                )
                if live_photo_candidates:
                    audio_url, duration = douyin_live_photo_composition(payload)
                    if duration is not None and duration > 0:
                        live_photo_candidates = [
                            Candidate(
                                candidate.url,
                                candidate.source,
                                candidate.priority,
                                candidate.cookie,
                                candidate.referer,
                                audio_url,
                                duration,
                            )
                            for candidate in live_photo_candidates
                        ]
                    return live_photo_candidates, []

        image_text = json.dumps(raw_images, ensure_ascii=False, separators=(",", ":"))
        image_candidates = extract_douyin_image_candidates_from_text(
            image_text,
            source=f"{source}.image",
        )
        if image_candidates:
            return [], image_candidates

    video_payload = payload.get("video")
    if not isinstance(video_payload, dict):
        return [], []
    return extract_douyin_video_payload_candidates(
        video_payload,
        source=source,
    ), []


def add_urls_from_value(
    candidates: list[Candidate],
    seen: set[str],
    value: Any,
    source: str,
    priority: int,
    platform: str,
) -> None:
    if isinstance(value, str):
        add_platform_candidate(candidates, seen, value, source, priority, platform)
    elif isinstance(value, list):
        for index, child in enumerate(value):
            add_urls_from_value(candidates, seen, child, f"{source}[{index}]", priority + index, platform)
    elif isinstance(value, dict):
        for key in (
            "url",
            "Url",
            "masterUrl",
            "backupUrl",
            "playUrl",
            "videoUrl",
            "src",
            "playAddr",
            "playAddrH264",
            "downloadAddr",
            "PlayAddr",
            "DownloadAddr",
        ):
            if key in value:
                add_urls_from_value(candidates, seen, value[key], f"{source}.{key}", priority, platform)
        for key in ("urls", "urlList", "UrlList", "url_list", "backupUrls", "mainMvUrls"):
            if key in value:
                add_urls_from_value(candidates, seen, value[key], f"{source}.{key}", priority + 5, platform)


def extract_kuaishou_candidates_from_json(value: Any) -> list[Candidate]:
    candidates: list[Candidate] = []
    seen: set[str] = set()

    preferred_keys = {
        "mainMvUrls": 5,
        "photoUrl": 10,
        "photoH264Url": 8,
        "photoH265Url": 12,
        "playUrl": 15,
        "videoUrl": 20,
        "h264Url": 8,
        "h265Url": 12,
        "url": 80,
    }
    for item in iter_dicts(value):
        for key, priority in preferred_keys.items():
            if key in item:
                add_urls_from_value(candidates, seen, item[key], f"kuaishou.{key}", priority, "kuaishou")

    for raw_url in iter_strings(value):
        if looks_like_kuaishou_video_url(raw_url):
            add_platform_candidate(candidates, seen, raw_url, "kuaishou.json-string", 90, "kuaishou")

    return sorted(candidates, key=lambda candidate: candidate.priority)


def extract_xiaohongshu_candidates_from_json(value: Any) -> list[Candidate]:
    candidates: list[Candidate] = []
    seen: set[str] = set()

    for item in iter_dicts(value):
        stream = item.get("stream")
        if isinstance(stream, dict):
            for codec_index, codec in enumerate(("h264", "h265", "av1")):
                streams = stream.get(codec)
                if isinstance(streams, list):
                    for stream_index, stream_item in enumerate(streams):
                        add_urls_from_value(
                            candidates,
                            seen,
                            stream_item,
                            f"xiaohongshu.stream.{codec}[{stream_index}]",
                            5 + codec_index * 10 + stream_index,
                            "xiaohongshu",
                        )

        for key, priority in (
            ("masterUrl", 8),
            ("backupUrls", 12),
            ("videoUrl", 20),
            ("video_url", 20),
            ("playUrl", 25),
            ("url", 90),
        ):
            if key in item:
                add_urls_from_value(candidates, seen, item[key], f"xiaohongshu.{key}", priority, "xiaohongshu")

    for raw_url in iter_strings(value):
        if looks_like_xiaohongshu_video_url(raw_url):
            add_platform_candidate(candidates, seen, raw_url, "xiaohongshu.json-string", 100, "xiaohongshu")

    return sorted(candidates, key=lambda candidate: candidate.priority)


def extract_tiktok_candidates_from_json(value: Any) -> list[Candidate]:
    candidates: list[Candidate] = []
    seen: set[str] = set()

    for item in iter_dicts(value):
        bitrate_info = item.get("bitrateInfo")
        if isinstance(bitrate_info, list):
            for index, bitrate_item in enumerate(bitrate_info):
                add_urls_from_value(
                    candidates,
                    seen,
                    bitrate_item,
                    f"tiktok.bitrateInfo[{index}]",
                    5 + index,
                    "tiktok",
                )

        for key, priority in (
            ("playAddr", 10),
            ("playAddrH264", 12),
            ("PlayAddr", 12),
            ("downloadAddr", 30),
            ("DownloadAddr", 30),
            ("UrlList", 15),
            ("urlList", 15),
            ("url_list", 15),
            ("urls", 20),
            ("url", 90),
            ("src", 90),
        ):
            if key in item:
                add_urls_from_value(candidates, seen, item[key], f"tiktok.{key}", priority, "tiktok")

    for raw_url in iter_strings(value):
        if looks_like_tiktok_video_url(raw_url):
            add_platform_candidate(candidates, seen, raw_url, "tiktok.json-string", 120, "tiktok")

    return sorted(candidates, key=lambda candidate: candidate.priority)


def extract_platform_candidates_from_html(page_text: str, platform: str) -> list[Candidate]:
    platform = normalize_platform(platform)
    candidates: list[Candidate] = []
    seen: set[str] = set()
    if platform == "kuaishou":
        extractor = extract_kuaishou_candidates_from_json
    elif platform == "xiaohongshu":
        extractor = extract_xiaohongshu_candidates_from_json
    elif platform == "tiktok":
        extractor = extract_tiktok_candidates_from_json
    else:
        extractor = extract_candidates_from_json

    for payload in extract_json_from_html(page_text):
        for candidate in extractor(payload):
            add_platform_candidate(candidates, seen, candidate.url, candidate.source, candidate.priority, platform)

    url_pattern = re.compile(r"https?:\\?/\\?/[^\"'<>\\\s]+|https?://[^\"'<>\\\s]+|//[^\"'<>\\\s]+")
    for match in url_pattern.finditer(page_text):
        raw_url = match.group(0)
        if looks_like_platform_video_url(raw_url, platform):
            add_platform_candidate(candidates, seen, raw_url, f"{platform}.html-url", 120, platform)

    return sorted(candidates, key=lambda candidate: candidate.priority)


def extract_candidates_from_html(page_text: str) -> list[Candidate]:
    candidates: list[Candidate] = []
    seen: set[str] = set()

    for payload in extract_json_from_html(page_text):
        for candidate in extract_candidates_from_json(payload):
            add_candidate(candidates, seen, candidate.url, candidate.source, candidate.priority)

    # Fallback for escaped URLs outside application/json script tags.
    url_pattern = re.compile(r"https?:\\?/\\?/[^\"'<>\\\s]+|https?://[^\"'<>\\\s]+")
    for match in url_pattern.finditer(page_text):
        raw_url = match.group(0)
        if looks_like_play_url(unwrap_url(raw_url)):
            add_candidate(candidates, seen, raw_url, "html-url", 60)

    return sorted(candidates, key=lambda candidate: candidate.priority)


def find_chrome_executable(chrome_path: str | None = None) -> str | None:
    if chrome_path:
        expanded = Path(chrome_path).expanduser()
        if expanded.exists():
            return str(expanded)
        return shutil.which(chrome_path)
    for executable in CHROMIUM_BROWSER_EXECUTABLES:
        executable_path = Path(executable).expanduser()
        if executable_path.exists():
            return str(executable_path)
        found = shutil.which(executable)
        if found:
            return found
    return None


def chromium_user_data_roots(chrome_path: str) -> list[Path]:
    """Return likely user-data roots for the selected Chromium-family executable."""
    executable = Path(chrome_path).name.lower()
    home = Path.home()
    roots: list[Path] = []

    if sys.platform == "darwin":
        base = home / "Library" / "Application Support"
        if "brave" in executable:
            roots.append(base / "BraveSoftware" / "Brave-Browser")
        elif "edge" in executable:
            roots.append(base / "Microsoft Edge")
        elif "chromium" in executable:
            roots.append(base / "Chromium")
        elif "vivaldi" in executable:
            roots.append(base / "Vivaldi")
        else:
            roots.append(base / "Google" / "Chrome")
    elif os.name == "nt":
        local_app_data = Path(os.environ.get("LOCALAPPDATA", home / "AppData" / "Local"))
        if "brave" in executable:
            roots.append(local_app_data / "BraveSoftware" / "Brave-Browser" / "User Data")
        elif "edge" in executable:
            roots.append(local_app_data / "Microsoft" / "Edge" / "User Data")
        elif "chromium" in executable:
            roots.append(local_app_data / "Chromium" / "User Data")
        elif "vivaldi" in executable:
            roots.append(local_app_data / "Vivaldi" / "User Data")
        else:
            roots.append(local_app_data / "Google" / "Chrome" / "User Data")
    else:
        config = home / ".config"
        if "brave" in executable:
            roots.append(config / "BraveSoftware" / "Brave-Browser")
        elif "edge" in executable:
            roots.append(config / "microsoft-edge")
        elif "chromium" in executable:
            roots.extend([config / "chromium", home / "snap" / "chromium" / "common" / "chromium"])
        elif "vivaldi" in executable:
            roots.append(config / "vivaldi")
        else:
            roots.append(config / "google-chrome")
    return list(dict.fromkeys(roots))


def chromium_profile_names(user_data_root: Path) -> list[str]:
    names: list[str] = []
    local_state = user_data_root / "Local State"
    try:
        payload = json.loads(local_state.read_text(encoding="utf-8"))
        profile = payload.get("profile") if isinstance(payload, dict) else None
        info_cache = profile.get("info_cache") if isinstance(profile, dict) else None
        if isinstance(info_cache, dict):
            names.extend(
                name
                for name in info_cache
                if isinstance(name, str) and Path(name).name == name
            )
    except (OSError, json.JSONDecodeError):
        pass
    for path in [user_data_root / "Default", *sorted(user_data_root.glob("Profile *"))]:
        if path.is_dir() and path.name not in names:
            names.append(path.name)
    return names


def cookie_host_filter(domains: Iterable[str]) -> tuple[str, list[str]]:
    normalized = [domain.strip().lower().lstrip(".") for domain in domains if domain.strip()]
    if not normalized:
        raise ValueError("At least one Cookie domain is required.")
    clauses: list[str] = []
    parameters: list[str] = []
    for domain in normalized:
        clauses.append("(host_key = ? or host_key like ?)")
        parameters.extend([domain, f"%.{domain}"])
    return " or ".join(clauses), parameters


def site_cookie_database_stats(path: Path, domains: Iterable[str]) -> tuple[int, int] | None:
    where_clause, parameters = cookie_host_filter(domains)
    try:
        source = sqlite3.connect(f"file:{path}?mode=ro", uri=True, timeout=2.0)
        try:
            row = source.execute(
                "select count(*), coalesce(max(last_access_utc), 0) from cookies "
                f"where {where_clause}",
                parameters,
            ).fetchone()
        finally:
            source.close()
    except sqlite3.Error:
        return None
    if not row or int(row[0]) <= 0:
        return None
    return int(row[0]), int(row[1])


def douyin_cookie_database_stats(path: Path) -> tuple[int, int] | None:
    return site_cookie_database_stats(path, ("douyin.com",))


def copy_filtered_cookie_database(
    source_path: Path,
    target_path: Path,
    domains: Iterable[str],
) -> int:
    """Copy only selected site rows into a fresh Chromium Cookies database."""
    where_clause, parameters = cookie_host_filter(domains)
    source = sqlite3.connect(f"file:{source_path}?mode=ro", uri=True, timeout=5.0)
    target_path.parent.mkdir(parents=True, exist_ok=True)
    target = sqlite3.connect(target_path)
    try:
        source.execute("pragma query_only=on")
        for table_name in ("meta", "cookies"):
            schema_row = source.execute(
                "select sql from sqlite_master where type='table' and name=?",
                (table_name,),
            ).fetchone()
            if not schema_row or not schema_row[0]:
                raise sqlite3.DatabaseError(f"missing {table_name} table")
            target.execute(str(schema_row[0]))

        meta_rows = source.execute("select * from meta").fetchall()
        meta_columns = len(source.execute("pragma table_info(meta)").fetchall())
        if meta_rows:
            target.executemany(
                f"insert into meta values ({','.join('?' for _ in range(meta_columns))})",
                meta_rows,
            )

        cookie_rows = source.execute(
            "select * from cookies " f"where {where_clause}",
            parameters,
        ).fetchall()
        cookie_columns = len(source.execute("pragma table_info(cookies)").fetchall())
        if cookie_rows:
            target.executemany(
                f"insert into cookies values ({','.join('?' for _ in range(cookie_columns))})",
                cookie_rows,
            )

        index_rows = source.execute(
            "select sql from sqlite_master "
            "where type='index' and tbl_name in ('meta', 'cookies') and sql is not null"
        ).fetchall()
        for (index_sql,) in index_rows:
            target.execute(str(index_sql))
        target.commit()
        return len(cookie_rows)
    finally:
        source.close()
        target.close()


def copy_filtered_douyin_cookie_database(source_path: Path, target_path: Path) -> int:
    return copy_filtered_cookie_database(source_path, target_path, ("douyin.com",))


def import_system_site_cookies(
    target_user_data_root: Path,
    chrome_path: str,
    domains: Iterable[str],
    *,
    user_data_roots: Iterable[Path] | None = None,
) -> SystemBrowserCookieImport | None:
    """Seed a temporary Chrome profile with only the requested site's Cookies."""
    selected_domains = tuple(domains)
    roots = list(user_data_roots) if user_data_roots is not None else chromium_user_data_roots(chrome_path)
    candidates: list[tuple[int, int, Path, str, Path]] = []
    for root in roots:
        for profile_name in chromium_profile_names(root):
            profile_dir = root / profile_name
            for relative_path in (Path("Network") / "Cookies", Path("Cookies")):
                cookie_db = profile_dir / relative_path
                if not cookie_db.is_file():
                    continue
                stats = site_cookie_database_stats(cookie_db, selected_domains)
                if stats is not None:
                    candidates.append((stats[1], stats[0], root, profile_name, relative_path))
                break
    if not candidates:
        return None

    _last_access, _count, source_root, profile_name, relative_path = max(candidates)
    source_db = source_root / profile_name / relative_path
    target_db = target_user_data_root / "Default" / relative_path
    cookie_count = copy_filtered_cookie_database(source_db, target_db, selected_domains)
    local_state = source_root / "Local State"
    if local_state.is_file():
        shutil.copy2(local_state, target_user_data_root / "Local State")
    return SystemBrowserCookieImport(Path(chrome_path).name, profile_name, cookie_count)


def import_system_douyin_cookies(
    target_user_data_root: Path,
    chrome_path: str,
    *,
    user_data_roots: Iterable[Path] | None = None,
) -> SystemBrowserCookieImport | None:
    """Seed a temporary Chrome profile with only Douyin cookies from the best local profile."""
    return import_system_site_cookies(
        target_user_data_root,
        chrome_path,
        ("douyin.com",),
        user_data_roots=user_data_roots,
    )


def import_system_xiaohongshu_cookies(
    target_user_data_root: Path,
    chrome_path: str,
    *,
    user_data_roots: Iterable[Path] | None = None,
) -> SystemBrowserCookieImport | None:
    """Seed a temporary Chrome profile with only Xiaohongshu cookies."""
    return import_system_site_cookies(
        target_user_data_root,
        chrome_path,
        ("xiaohongshu.com",),
        user_data_roots=user_data_roots,
    )


def browser_fallback_was_unavailable(logs: Iterable[str]) -> bool:
    return any("Browser fallback skipped:" in line for line in logs)


def iter_netlog_urls(payload: Any) -> Iterable[str]:
    url_pattern = re.compile(r"https?://[^\s\"'<>\\]+")
    for value in iter_strings(payload):
        if value.startswith(("http://", "https://")):
            yield value
            continue
        for match in url_pattern.finditer(value):
            yield match.group(0)


def browser_candidate_priority(url: str, platform: str, index: int) -> int:
    platform = normalize_platform(platform)
    if platform == "douyin":
        parsed = urllib.parse.urlsplit(unwrap_url(url))
        query = urllib.parse.parse_qs(parsed.query)
        for key in ("bt", "br"):
            raw_value = (query.get(key) or [""])[0]
            try:
                value = int(raw_value)
            except ValueError:
                continue
            if value > 0:
                return max(1, 1_000_000 - min(value, 999_999))
    return 5_000_000 + index


def douyin_payload_candidate_priority(candidate: Candidate, index: int) -> int:
    """Prefer exact structured play addresses over incidental player/CDN URLs."""
    priority = browser_candidate_priority(candidate.url, "douyin", index)
    source_name = candidate.source.rsplit(".", 1)[-1]
    structured_source = source_name.startswith(
        (
            "bit_rate[",
            "play_addr",
            "play_api",
            "download_addr",
        )
    )
    if not structured_source:
        # Arbitrary URL strings in an exact item payload may include the web
        # player's already-watermarked or truncated rendition. Structured
        # play_addr/bit_rate entries describe the actual work media.
        priority += 20_000_000
    if looks_like_video_only_stream_url(candidate.url):
        priority += 10_000_000
    return priority


def browser_candidate_matches_item(url: str, item_id: str | None) -> bool:
    if not item_id:
        return False
    parsed = urllib.parse.urlsplit(unwrap_url(url))
    query = urllib.parse.parse_qs(parsed.query)
    for key in ("__vid", "aweme_id", "item_id", "item_ids", "modal_id"):
        if item_id in query.get(key, []):
            return True
    return False


def extract_browser_candidates_from_netlog_payload(
    payload: Any,
    platform: str,
    *,
    item_id: str | None = None,
    require_item_match: bool = False,
) -> list[Candidate]:
    platform = normalize_platform(platform)
    candidates: list[Candidate] = []
    seen: set[str] = set()
    for index, raw_url in enumerate(iter_netlog_urls(payload)):
        if looks_like_platform_video_url(raw_url, platform):
            item_match = browser_candidate_matches_item(raw_url, item_id)
            if require_item_match and not item_match:
                continue
            priority = browser_candidate_priority(raw_url, platform, index)
            if item_match and platform != "douyin":
                priority = max(0, priority - 2_000_000)
            add_platform_candidate(
                candidates,
                seen,
                raw_url,
                f"{platform}.browser-netlog",
                priority,
                platform,
            )
    return sorted(candidates, key=lambda candidate: candidate.priority)


def prioritize_browser_target_urls(platform: str, item_id: str | None, urls: Iterable[str]) -> list[str]:
    """Try canonical item and share URLs before platform-specific fallback routes."""
    unique_urls = list(dict.fromkeys(urls))
    if normalize_platform(platform) != "douyin" or not item_id:
        return unique_urls

    canonical: list[str] = []
    share_urls: list[str] = []
    other: list[str] = []
    for url in unique_urls:
        parsed = urllib.parse.urlsplit(url)
        if re.search(rf"/(?:video|note)/{re.escape(item_id)}(?:[/?#]|$)", parsed.path):
            canonical.append(url)
        elif parsed.netloc.lower() == "v.douyin.com":
            share_urls.append(url)
        else:
            other.append(url)

    has_modal_route = any(
        item_id in urllib.parse.parse_qs(urllib.parse.urlsplit(url).query).get("modal_id", [])
        for url in unique_urls
    )
    if has_modal_route and not canonical:
        canonical.append(f"https://www.douyin.com/video/{urllib.parse.quote(item_id)}")

    fallback_url = f"https://www.douyin.com/jingxuan?modal_id={urllib.parse.quote(item_id)}"
    ordered = canonical + share_urls + other
    if fallback_url not in ordered:
        ordered.append(fallback_url)
    return ordered


def browser_target_requires_item_match(platform: str, target_url: str, item_id: str | None) -> bool:
    if normalize_platform(platform) != "douyin" or not item_id:
        return False
    parsed = urllib.parse.urlsplit(target_url)
    return not bool(re.search(rf"/(?:video|note)/{re.escape(item_id)}(?:[/?#]|$)", parsed.path))


def browser_candidates_are_sufficient(
    candidates: list[Candidate],
    image_candidates: list[ImageCandidate],
    *,
    require_audio: bool,
) -> bool:
    if not candidates and not image_candidates:
        return False
    if (
        require_audio
        and candidates
        and all(looks_like_video_only_stream_url(candidate.url) for candidate in candidates)
    ):
        return False
    return True


DOUYIN_ITEM_RENDER_DATA_SCRIPT = r"""
(() => {
  const node = document.querySelector('script#RENDER_DATA');
  return node ? (node.textContent || '') : '';
})()
"""


def parse_douyin_browser_payload_text(
    body: str,
    *,
    item_id: str,
    render_data: bool = False,
) -> dict[str, Any] | None:
    text = urllib.parse.unquote(html.unescape(body)) if render_data else body
    payload = load_json_text(text)
    return find_douyin_aweme_payload(payload, item_id)


def gather_douyin_browser_item_candidates(
    target_urls: Iterable[str],
    *,
    item_id: str,
    cookie: str | None,
    timeout: float,
    chrome_path: str,
    use_system_browser_cookies: bool = True,
) -> tuple[list[Candidate], list[ImageCandidate], list[str]]:
    """Read the exact requested Aweme payload from an authenticated browser page."""
    logs: list[str] = []
    token = current_cancellation_token()
    process: subprocess.Popen[Any] | None = None
    client: DevToolsConnection | None = None
    target_payload: dict[str, Any] | None = None

    with tempfile.TemporaryDirectory(prefix="media_downloader_item_", ignore_cleanup_errors=True) as tmpdir:
        profile_dir = Path(tmpdir)
        system_cookie_import: SystemBrowserCookieImport | None = None
        if not cookie and use_system_browser_cookies:
            try:
                system_cookie_import = import_system_douyin_cookies(profile_dir, chrome_path)
            except (OSError, sqlite3.Error) as exc:
                logs.append(f"douyin: system browser Cookie import failed: {exc}")
            if system_cookie_import is not None:
                logs.append(
                    "douyin: auto-loaded "
                    f"{system_cookie_import.cookie_count} douyin.com cookie(s) from "
                    f"system browser profile {system_cookie_import.profile!r}"
                )

        command = [
            chrome_path,
            "--headless=new",
            "--disable-gpu",
            "--no-sandbox",
            "--disable-dev-shm-usage",
            "--disable-blink-features=AutomationControlled",
            "--mute-audio",
            "--autoplay-policy=no-user-gesture-required",
            "--remote-debugging-port=0",
            "--remote-allow-origins=*",
            "--window-size=1280,2000",
            f"--user-agent={DEFAULT_HEADERS['User-Agent']}",
            f"--user-data-dir={tmpdir}",
            "--profile-directory=Default",
            "about:blank",
        ]
        try:
            process = subprocess.Popen(
                command,
                stdout=subprocess.DEVNULL,
                stderr=subprocess.DEVNULL,
                start_new_session=os.name == "posix",
            )
            if token is not None:
                token.register_process(process)
            websocket_url = wait_for_devtools_page_url(profile_dir, process, timeout=timeout)
            client = DevToolsConnection(websocket_url, timeout=min(max(timeout, 1.0), 10.0))
            logs.append("douyin: exact-item browser collector is ready")

            client.send(
                "Network.enable",
                {
                    "maxTotalBufferSize": 100 * 1024 * 1024,
                    "maxResourceBufferSize": 32 * 1024 * 1024,
                },
            )
            client.send("Page.enable")
            client.send("Runtime.enable")
            cookie_params = cookie_params_for_douyin(cookie)
            if cookie_params:
                client.send("Network.setCookies", {"cookies": cookie_params})
                logs.append(f"douyin: injected {len(cookie_params)} cookie(s) into item browser")
            elif system_cookie_import is not None:
                logs.append("douyin: using the system browser Douyin login state for this item")

            unique_targets = list(dict.fromkeys(target_urls))
            route_timeout = max(5.0, timeout)
            for route_index, target_url in enumerate(unique_targets, start=1):
                raise_if_task_cancelled()
                logs.append(
                    f"douyin: exact-item browser opening route {route_index}/{len(unique_targets)} "
                    f"{target_url}"
                )
                pending_requests: dict[str, str] = {}
                pending_commands: dict[int, tuple[str, str | None]] = {}
                render_data_requested = False
                client.send("Page.navigate", {"url": target_url})
                started_at = time.monotonic()
                deadline = started_at + route_timeout

                while time.monotonic() < deadline and target_payload is None:
                    raise_if_task_cancelled()
                    now = time.monotonic()
                    if not render_data_requested and now - started_at >= 2.0:
                        command_id = client.send(
                            "Runtime.evaluate",
                            {
                                "expression": DOUYIN_ITEM_RENDER_DATA_SCRIPT,
                                "returnByValue": True,
                            },
                        )
                        pending_commands[command_id] = ("render_data", None)
                        render_data_requested = True

                    try:
                        message = client.recv(timeout=min(0.5, max(0.05, deadline - now)))
                    except TimeoutError:
                        continue
                    if message is None:
                        break

                    method = message.get("method")
                    params = message.get("params") if isinstance(message.get("params"), dict) else {}
                    if method == "Network.responseReceived":
                        response = params.get("response") if isinstance(params.get("response"), dict) else {}
                        response_url = str(response.get("url") or "")
                        if (
                            "/aweme/v1/web/aweme/post/" in response_url
                            or "/aweme/v1/web/aweme/detail/" in response_url
                        ):
                            request_id = str(params.get("requestId") or "")
                            if request_id:
                                pending_requests[request_id] = response_url
                        continue
                    if method == "Network.loadingFinished":
                        request_id = str(params.get("requestId") or "")
                        response_url = pending_requests.pop(request_id, None)
                        if response_url:
                            command_id = client.send("Network.getResponseBody", {"requestId": request_id})
                            pending_commands[command_id] = ("response_body", response_url)
                        continue
                    if method == "Network.loadingFailed":
                        pending_requests.pop(str(params.get("requestId") or ""), None)
                        continue

                    command_id = message.get("id")
                    if not isinstance(command_id, int) or command_id not in pending_commands:
                        continue
                    kind, context = pending_commands.pop(command_id)
                    if message.get("error"):
                        logs.append(f"douyin: exact-item browser {kind} failed: {message['error']}")
                        continue
                    result = message.get("result") if isinstance(message.get("result"), dict) else {}
                    if kind == "render_data":
                        remote_result = result.get("result") if isinstance(result.get("result"), dict) else {}
                        raw_render_data = remote_result.get("value")
                        if isinstance(raw_render_data, str) and raw_render_data:
                            target_payload = parse_douyin_browser_payload_text(
                                raw_render_data,
                                item_id=item_id,
                                render_data=True,
                            )
                        continue

                    body = result.get("body")
                    if not isinstance(body, str) or not body:
                        continue
                    if result.get("base64Encoded"):
                        try:
                            body = base64.b64decode(body).decode("utf-8", errors="replace")
                        except (ValueError, UnicodeDecodeError) as exc:
                            logs.append(f"douyin: could not decode exact-item response: {exc}")
                            continue
                    target_payload = parse_douyin_browser_payload_text(body, item_id=item_id)
                    if target_payload is not None:
                        logs.append(
                            "douyin: matched requested item "
                            f"{item_id} in {urllib.parse.urlsplit(context or '').path or 'browser response'}"
                        )

                if target_payload is not None:
                    break
        except (DevToolsError, OSError, urllib.error.URLError) as exc:
            logs.append(f"douyin: exact-item browser collection failed: {exc}")
        finally:
            if client is not None:
                client.close()
            if process is not None:
                if token is not None:
                    token.unregister_process(process)
                if process.poll() is None:
                    terminate_process(process)
                    try:
                        process.wait(timeout=3)
                    except subprocess.TimeoutExpired:
                        terminate_process(process, force=True)
                        process.wait()

    if target_payload is None:
        logs.append(f"douyin: exact-item browser found no payload for {item_id}")
        return [], [], logs

    candidates, image_candidates = extract_douyin_item_media_candidates(
        target_payload,
        source="douyin.browser-item",
    )
    media_type = (
        "images"
        if image_candidates
        else "live-photo-video"
        if candidates and douyin_payload_is_live_photo(target_payload)
        else "video"
        if candidates
        else "unknown"
    )
    count = len(image_candidates) if image_candidates else len(candidates)
    logs.append(
        f"douyin: exact requested item media type={media_type} count={count} aweme_id={item_id}"
    )
    return candidates, image_candidates, logs


def gather_browser_candidates(
    share_text: str,
    *,
    platform: str,
    cookie: str | None = None,
    timeout: float = DEFAULT_BROWSER_TIMEOUT,
    chrome_path: str | None = None,
    require_audio: bool = False,
    use_system_browser_cookies: bool = True,
) -> tuple[str | None, list[Candidate], list[ImageCandidate], list[str]]:
    platform = normalize_platform(platform)
    urls = extract_urls(share_text)
    if not urls:
        if share_text.strip().startswith(("http://", "https://")):
            urls = [share_text.strip()]
        else:
            raise DouyinDownloadError("No URL found in the share text.")

    logs: list[str] = []
    chrome = find_chrome_executable(chrome_path)
    if not chrome:
        logs.append(
            "Browser fallback skipped: no Chromium-compatible browser was found. "
            "Install Chrome, Chromium, Edge, Brave, or pass --chrome-path."
        )
        return extract_platform_id(platform, share_text), [], [], logs
    if cookie and platform != "douyin":
        logs.append("Browser fallback uses a fresh local Chrome profile; --cookie only applies to direct HTTP parsing.")

    item_id = extract_platform_id(platform, share_text)
    candidates: list[Candidate] = []
    image_candidates: list[ImageCandidate] = []
    seen_image_urls: set[str] = set()
    seen: set[str] = set()
    target_urls: list[str] = []

    for share_url in urls:
        try:
            resolved = http_get(
                share_url,
                cookie=cookie,
                timeout=min(timeout, 20.0),
                max_bytes=256 * 1024,
                extra_headers={"Referer": platform_referer(platform)},
            )
            page_text = decode_text(resolved.content, resolved.headers)
            item_id = item_id or extract_platform_id(platform, resolved.url, page_text)
            if resolved.url not in target_urls:
                target_urls.append(resolved.url)
        except DouyinDownloadError as exc:
            logs.append(f"{platform}: browser fallback pre-resolve failed: {exc}")
        if share_url not in target_urls:
            target_urls.append(share_url)

    prioritized_target_urls = prioritize_browser_target_urls(platform, item_id, target_urls)
    if prioritized_target_urls != target_urls and prioritized_target_urls:
        logs.append(f"{platform}: browser fallback added the platform-specific item route")
    target_urls = prioritized_target_urls

    if platform == "douyin" and item_id:
        exact_candidates, exact_images, exact_logs = gather_douyin_browser_item_candidates(
            target_urls,
            item_id=item_id,
            cookie=cookie,
            timeout=timeout,
            chrome_path=chrome,
            use_system_browser_cookies=use_system_browser_cookies,
        )
        logs.extend(exact_logs)
        if exact_candidates or exact_images:
            return item_id, exact_candidates, exact_images, logs

    for target_url in target_urls:
        with tempfile.TemporaryDirectory(prefix="media_downloader_chrome_", ignore_cleanup_errors=True) as tmpdir:
            netlog_path = Path(tmpdir) / "netlog.json"
            require_item_match = browser_target_requires_item_match(platform, target_url, item_id)
            command = [
                chrome,
                "--headless=new",
                "--disable-gpu",
                "--no-sandbox",
                "--disable-dev-shm-usage",
                "--mute-audio",
                "--autoplay-policy=no-user-gesture-required",
                f"--user-data-dir={tmpdir}",
                f"--log-net-log={netlog_path}",
                "--net-log-capture-mode=IncludeSensitive",
                f"--virtual-time-budget={max(1000, int(timeout * 1000))}",
                "--dump-dom",
                target_url,
            ]
            logs.append(f"{platform}: browser fallback opening {target_url}")
            try:
                completed = run_task_subprocess(
                    command,
                    check=False,
                    capture_output=True,
                    text=True,
                    timeout=timeout + 10,
                )
            except subprocess.TimeoutExpired as exc:
                logs.append(f"{platform}: browser fallback timed out after {timeout:.1f}s")
                stdout = exc.output or ""
                stderr = exc.stderr or ""
                if isinstance(stdout, bytes):
                    stdout = stdout.decode("utf-8", errors="replace")
                if isinstance(stderr, bytes):
                    stderr = stderr.decode("utf-8", errors="replace")
                completed = subprocess.CompletedProcess(command, -1, stdout, stderr)
            except OSError as exc:
                logs.append(f"{platform}: browser fallback failed to start Chrome: {exc}")
                continue

            if completed.returncode != 0:
                stderr_line = completed.stderr.strip().splitlines()
                detail = f": {stderr_line[0]}" if stderr_line else ""
                logs.append(f"{platform}: browser fallback Chrome exited with {completed.returncode}{detail}")

            item_id = item_id or extract_platform_id(platform, target_url, completed.stdout)
            if platform in {"douyin", "xiaohongshu"}:
                image_extractor = (
                    extract_douyin_image_candidates_from_text
                    if platform == "douyin"
                    else extract_xiaohongshu_image_candidates_from_text
                )
                dom_image_candidates = image_extractor(
                    completed.stdout,
                    f"{platform}.browser-dom-image",
                )
                logs.append(f"{platform}: browser fallback found {len(dom_image_candidates)} image candidate(s)")
                for candidate in dom_image_candidates:
                    if candidate.url in seen_image_urls:
                        continue
                    seen_image_urls.add(candidate.url)
                    image_candidates.append(candidate)

            for index, candidate in enumerate(extract_platform_candidates_from_html(completed.stdout, platform)):
                item_match = browser_candidate_matches_item(candidate.url, item_id)
                if require_item_match and not item_match:
                    continue
                if platform == "douyin":
                    priority = browser_candidate_priority(candidate.url, platform, index)
                else:
                    priority = candidate.priority + 2_000_000
                    if item_match:
                        priority = max(0, priority - 4_000_000)
                add_platform_candidate(
                    candidates,
                    seen,
                    candidate.url,
                    f"{candidate.source}:browser-dom",
                    priority,
                    platform,
                )

            if not netlog_path.exists():
                logs.append(f"{platform}: browser fallback produced no network log")
                continue
            try:
                netlog_text = netlog_path.read_text(encoding="utf-8")
            except OSError as exc:
                logs.append(f"{platform}: browser fallback could not parse network log: {exc}")
                continue
            try:
                payload = json.loads(netlog_text)
            except json.JSONDecodeError as exc:
                logs.append(
                    f"{platform}: browser fallback network log was incomplete; "
                    f"scanning captured URLs directly ({exc})"
                )
                payload = netlog_text

            netlog_candidates = extract_browser_candidates_from_netlog_payload(
                payload,
                platform,
                item_id=item_id,
                require_item_match=require_item_match,
            )
            logs.append(f"{platform}: browser fallback found {len(netlog_candidates)} network video candidate(s)")
            for candidate in netlog_candidates:
                add_platform_candidate(candidates, seen, candidate.url, candidate.source, candidate.priority, platform)
            if candidates and not browser_candidates_are_sufficient(
                candidates,
                image_candidates,
                require_audio=require_audio,
            ):
                logs.append(
                    f"{platform}: browser fallback only found video-only adaptive streams; "
                    "continuing to another item route"
                )
                continue
            if browser_candidates_are_sufficient(
                candidates,
                image_candidates,
                require_audio=require_audio,
            ):
                break

    if (
        platform == "douyin"
        and image_candidates
        and item_id
        and any(
            re.search(rf"/note/{re.escape(item_id)}(?:[/?#]|$)", urllib.parse.urlsplit(url).path)
            for url in target_urls
        )
    ):
        if candidates:
            logs.append("douyin: resolved route is a note; ignoring page video streams from music/recommendations")
        candidates = []

    return (
        item_id,
        sorted(candidates, key=lambda candidate: candidate.priority),
        sorted(image_candidates, key=lambda candidate: candidate.priority),
        logs,
    )


def cookie_params_for_site(cookie: str | None, domain: str) -> list[dict[str, Any]]:
    params: list[dict[str, Any]] = []
    for part in (cookie or "").split(";"):
        name, separator, value = part.strip().partition("=")
        if not separator or not name or name.startswith("$"):
            continue
        params.append(
            {
                "name": name,
                "value": value,
                "domain": f".{domain.lstrip('.')}",
                "path": "/",
                "secure": True,
            }
        )
    return params


def cookie_params_for_douyin(cookie: str | None) -> list[dict[str, Any]]:
    return cookie_params_for_site(cookie, "douyin.com")


def cookie_params_for_xiaohongshu(cookie: str | None) -> list[dict[str, Any]]:
    return cookie_params_for_site(cookie, "xiaohongshu.com")


def wait_for_devtools_page_url(
    profile_dir: Path,
    process: subprocess.Popen[Any],
    *,
    timeout: float,
) -> str:
    active_port_path = profile_dir / "DevToolsActivePort"
    deadline = time.monotonic() + max(1.0, min(timeout, 15.0))
    last_error: Exception | None = None
    while time.monotonic() < deadline:
        raise_if_task_cancelled()
        if process.poll() is not None:
            raise DouyinDownloadError(
                f"Chrome exited before its DevTools endpoint was ready (exit={process.returncode})."
            )
        try:
            lines = active_port_path.read_text(encoding="utf-8").splitlines()
            port = int(lines[0])
            request = urllib.request.Request(f"http://127.0.0.1:{port}/json/list")
            with urllib.request.urlopen(request, timeout=2.0) as response:
                targets = json.load(response)
            for target in targets:
                if isinstance(target, dict) and target.get("type") == "page":
                    websocket_url = target.get("webSocketDebuggerUrl")
                    if isinstance(websocket_url, str):
                        return websocket_url
        except (OSError, ValueError, IndexError, json.JSONDecodeError, urllib.error.URLError) as exc:
            last_error = exc
        time.sleep(0.1)
    detail = f": {last_error}" if last_error else ""
    raise DouyinDownloadError(f"Timed out waiting for Chrome DevTools{detail}")


def douyin_profile_username_from_page_state(value: Any, sec_uid: str) -> str:
    if not isinstance(value, dict):
        return sec_uid
    title = str(value.get("title") or "").strip()
    title_match = re.match(r"^(.*?)的抖音(?:\s*-\s*抖音)?$", title)
    if title_match and title_match.group(1).strip():
        return title_match.group(1).strip()
    heading = str(value.get("heading") or "").strip()
    return heading or sec_uid


def extract_douyin_profile_video_candidates(video_payload: dict[str, Any]) -> list[Candidate]:
    candidates = list(extract_candidates_from_json(video_payload))
    seen = {candidate.url for candidate in candidates}
    for index, raw_url in enumerate(iter_strings(video_payload)):
        if not looks_like_platform_video_url(raw_url, "douyin"):
            continue
        add_platform_candidate(
            candidates,
            seen,
            prefer_no_watermark_url(raw_url),
            "video-url",
            browser_candidate_priority(raw_url, "douyin", index),
            "douyin",
        )
    quality_ranked: list[Candidate] = []
    for index, candidate in enumerate(candidates):
        priority = douyin_payload_candidate_priority(candidate, index)
        quality_ranked.append(
            Candidate(
                candidate.url,
                candidate.source,
                priority,
                candidate.cookie,
                candidate.referer,
            )
        )
    return sorted(quality_ranked, key=lambda candidate: candidate.priority)


def add_douyin_profile_payload(
    data: Any,
    *,
    sec_uid: str,
    posts_by_id: dict[str, DouyinProfilePost],
    logs: list[str],
) -> tuple[str | None, bool | None, int]:
    if not isinstance(data, dict):
        return None, None, 0
    status_code = data.get("status_code")
    if status_code not in (None, 0):
        logs.append(f"douyin profile API returned status_code={status_code}")
    aweme_list = data.get("aweme_list") or data.get("awemeList") or []
    if not isinstance(aweme_list, list):
        aweme_list = []

    username: str | None = None
    added = 0
    for aweme in aweme_list:
        if not isinstance(aweme, dict):
            continue
        author = aweme.get("author") if isinstance(aweme.get("author"), dict) else {}
        author_sec_uid = str(author.get("sec_uid") or author.get("secUid") or "")
        if author_sec_uid and author_sec_uid != sec_uid:
            continue
        if not username:
            raw_username = author.get("nickname") or author.get("nick_name")
            if isinstance(raw_username, str) and raw_username.strip():
                username = raw_username.strip()
        video_payload = aweme.get("video")
        has_video = isinstance(video_payload, dict) and bool(
            extract_douyin_profile_video_candidates(video_payload)
        )
        payload_text = json.dumps(aweme, ensure_ascii=False, separators=(",", ":"))
        has_images = bool(
            extract_douyin_image_candidates_from_text(
                payload_text,
                source="douyin.profile-image",
            )
        )
        if not has_video and not has_images:
            continue
        item_id = str(aweme.get("aweme_id") or aweme.get("awemeId") or "")
        if not item_id or item_id in posts_by_id:
            continue
        try:
            create_time = int(aweme.get("create_time") or aweme.get("createTime") or 0)
        except (TypeError, ValueError):
            create_time = 0
        posts_by_id[item_id] = DouyinProfilePost(item_id, create_time, aweme)
        added += 1

    raw_has_more = data.get("has_more")
    has_more = bool(raw_has_more) if raw_has_more is not None else None
    return username, has_more, added


DOUYIN_PROFILE_STATE_SCRIPT = r"""
(() => {
  const retry = Array.from(document.querySelectorAll('button,[role="button"],div')).find((node) => {
    const text = (node.innerText || '').trim();
    return node.children.length <= 3 && text.includes('重新刷新拉取数据');
  });
  if (retry) retry.click();
  const root = document.scrollingElement || document.documentElement;
  const scrollables = Array.from(document.querySelectorAll('*')).filter((node) => {
    const style = getComputedStyle(node);
    const rect = node.getBoundingClientRect();
    return (
      rect.bottom > 0 && rect.top < innerHeight &&
      node.clientHeight >= 300 &&
      node.scrollHeight > node.clientHeight + 100 &&
      (style.overflowY === 'auto' || style.overflowY === 'scroll')
    );
  });
  if (root.scrollHeight > root.clientHeight + 100) scrollables.push(root);
  const uniqueScrollables = Array.from(new Set(scrollables));
  const positions = uniqueScrollables.map((node) => {
    const before = node.scrollTop;
    node.scrollTop = node.scrollHeight;
    node.dispatchEvent(new Event('scroll', { bubbles: true }));
    return {
      tag: node.tagName,
      route: node.classList.contains('route-scroll-container'),
      before,
      after: node.scrollTop,
      height: node.scrollHeight,
      viewport: node.clientHeight,
    };
  });
  if (!uniqueScrollables.length) window.scrollTo(0, root.scrollHeight);
  return {
    title: document.title || '',
    heading: (document.querySelector('h1') || {}).innerText || '',
    height: root.scrollHeight,
    scrollTop: root.scrollTop,
    scrollTargets: positions,
    retried: Boolean(retry),
  };
})()
"""

DOUYIN_PROFILE_INFO_SCRIPT = r"""
(() => ({
  title: document.title || '',
  heading: (document.querySelector('h1') || {}).innerText || '',
}))()
"""


def gather_douyin_profile_posts(
    profile_url: str,
    sec_uid: str,
    *,
    limit: int | str = DEFAULT_PROFILE_LIMIT,
    interval: float = DEFAULT_PROFILE_INTERVAL,
    cookie: str | None = None,
    timeout: float = DEFAULT_BROWSER_TIMEOUT,
    chrome_path: str | None = None,
    progress: Callable[[str], None] | None = None,
    use_system_browser_cookies: bool = True,
) -> DouyinProfileResult:
    """Scroll a Douyin user page and collect its newest public post payloads."""
    collect_all = isinstance(limit, str) and limit.lower() == "all"
    if not collect_all and (not isinstance(limit, int) or limit <= 0):
        raise DouyinDownloadError("--profile-limit must be a positive integer or 'all'.")
    numeric_limit = None if collect_all else int(limit)
    target_label = "all" if collect_all else str(numeric_limit)
    chrome = find_chrome_executable(chrome_path)
    if not chrome:
        raise DouyinDownloadError(
            "A Chromium-compatible browser is required for Douyin profile downloads. "
            "Install Chrome, Chromium, Edge, Brave, or pass --chrome-path."
        )

    logs: list[str] = []

    def record(message: str) -> None:
        logs.append(message)
        if progress is not None:
            progress(message)

    record(f"douyin profile: opening {profile_url}")
    posts_by_id: dict[str, DouyinProfilePost] = {}
    username = sec_uid
    has_more: bool | None = None
    target_count = (
        None
        if numeric_limit is None
        else numeric_limit + min(10, max(2, numeric_limit // 10))
    )
    token = current_cancellation_token()
    process: subprocess.Popen[Any] | None = None
    client: DevToolsConnection | None = None

    with tempfile.TemporaryDirectory(prefix="media_downloader_profile_", ignore_cleanup_errors=True) as tmpdir:
        profile_dir = Path(tmpdir)
        system_cookie_import: SystemBrowserCookieImport | None = None
        if not cookie and use_system_browser_cookies:
            try:
                system_cookie_import = import_system_douyin_cookies(profile_dir, chrome)
            except (OSError, sqlite3.Error) as exc:
                record(f"douyin profile: system browser Cookie import failed: {exc}")
            if system_cookie_import is not None:
                record(
                    "douyin profile: auto-loaded "
                    f"{system_cookie_import.cookie_count} douyin.com cookie(s) from "
                    f"system browser profile {system_cookie_import.profile!r}"
                )
        command = [
            chrome,
            "--headless=new",
            "--disable-gpu",
            "--no-sandbox",
            "--disable-dev-shm-usage",
            "--disable-blink-features=AutomationControlled",
            "--mute-audio",
            "--autoplay-policy=no-user-gesture-required",
            "--remote-debugging-port=0",
            "--remote-allow-origins=*",
            "--window-size=1280,2000",
            f"--user-agent={DEFAULT_HEADERS['User-Agent']}",
            f"--user-data-dir={tmpdir}",
            "--profile-directory=Default",
            "about:blank",
        ]
        try:
            process = subprocess.Popen(
                command,
                stdout=subprocess.DEVNULL,
                stderr=subprocess.DEVNULL,
                start_new_session=os.name == "posix",
            )
            if token is not None:
                token.register_process(process)
            websocket_url = wait_for_devtools_page_url(profile_dir, process, timeout=timeout)
            client = DevToolsConnection(websocket_url, timeout=min(max(timeout, 1.0), 10.0))
            record("douyin profile: browser collector is ready")

            pending_profile_requests: dict[str, str] = {}
            pending_commands: dict[int, tuple[str, Any]] = {}
            received_profile_body = False
            last_new_post_at = time.monotonic()
            scroll_count = 0
            post_response_count = 0

            client.send(
                "Network.enable",
                {
                    "maxTotalBufferSize": 100 * 1024 * 1024,
                    "maxResourceBufferSize": 16 * 1024 * 1024,
                },
            )
            client.send("Page.enable")
            client.send("Runtime.enable")
            cookie_params = cookie_params_for_douyin(cookie)
            if cookie_params:
                pending_commands[client.send("Network.setCookies", {"cookies": cookie_params})] = (
                    "cookies",
                    None,
                )
                record(f"douyin profile: injected {len(cookie_params)} cookie(s) into Chrome")
            elif system_cookie_import is not None:
                record("douyin profile: using the system browser Douyin login state")
            else:
                record("douyin profile: no login Cookie was supplied")
            client.send("Page.navigate", {"url": profile_url})

            deadline = time.monotonic() + max(5.0, timeout)
            request_interval = max(1.0, interval)
            inactivity_timeout = max(8.0, request_interval * 3)
            next_scroll_at = time.monotonic() + max(3.0, request_interval)
            while time.monotonic() < deadline:
                raise_if_task_cancelled()
                now = time.monotonic()
                if (
                    posts_by_id
                    and now - last_new_post_at >= inactivity_timeout
                    and scroll_count >= 2
                ):
                    break
                if now >= next_scroll_at:
                    command_id = client.send(
                        "Runtime.evaluate",
                        {
                            "expression": DOUYIN_PROFILE_STATE_SCRIPT,
                            "returnByValue": True,
                        },
                    )
                    pending_commands[command_id] = ("page_state", None)
                    scroll_count += 1
                    record(
                        f"douyin profile: scrolling page {scroll_count}; "
                        f"collected={len(posts_by_id)} target={target_label}"
                    )
                    next_scroll_at = now + request_interval

                wait_time = min(0.5, max(0.05, deadline - now), max(0.05, next_scroll_at - now))
                try:
                    message = client.recv(timeout=wait_time)
                except TimeoutError:
                    continue
                if message is None:
                    record("douyin profile: Chrome DevTools connection closed before collection finished")
                    break

                method = message.get("method")
                params = message.get("params") if isinstance(message.get("params"), dict) else {}
                if method == "Network.responseReceived":
                    response = params.get("response") if isinstance(params.get("response"), dict) else {}
                    response_url = str(response.get("url") or "")
                    if "/aweme/v1/web/aweme/post/" in response_url or "/aweme/v1/web/user/post/" in response_url:
                        request_id = str(params.get("requestId") or "")
                        if request_id:
                            pending_profile_requests[request_id] = response_url
                    continue
                if method == "Network.loadingFinished":
                    request_id = str(params.get("requestId") or "")
                    response_url = pending_profile_requests.pop(request_id, None)
                    if response_url:
                        command_id = client.send("Network.getResponseBody", {"requestId": request_id})
                        pending_commands[command_id] = ("profile_body", response_url)
                    continue
                if method == "Network.loadingFailed":
                    request_id = str(params.get("requestId") or "")
                    if pending_profile_requests.pop(request_id, None):
                        record(
                            f"douyin profile: post request failed: {params.get('errorText') or 'unknown error'}"
                        )
                    continue

                command_id = message.get("id")
                if not isinstance(command_id, int) or command_id not in pending_commands:
                    continue
                kind, context = pending_commands.pop(command_id)
                if message.get("error"):
                    record(f"douyin profile: DevTools {kind} failed: {message['error']}")
                    continue
                result = message.get("result") if isinstance(message.get("result"), dict) else {}
                if kind == "cookies":
                    continue
                if kind == "page_state":
                    remote_result = result.get("result") if isinstance(result.get("result"), dict) else {}
                    value = remote_result.get("value")
                    page_username = douyin_profile_username_from_page_state(value, sec_uid)
                    if page_username != sec_uid:
                        if username != page_username:
                            record(f"douyin profile: detected username {page_username!r}")
                        username = page_username
                    continue
                if kind != "profile_body":
                    continue

                received_profile_body = True
                post_response_count += 1
                record(f"douyin profile: processing post response {post_response_count}")
                body = result.get("body")
                if not isinstance(body, str) or not body:
                    record("douyin profile: post API returned an empty response; retrying the page")
                    continue
                if result.get("base64Encoded"):
                    try:
                        body = base64.b64decode(body).decode("utf-8", errors="replace")
                    except (ValueError, UnicodeDecodeError) as exc:
                        record(f"douyin profile: could not decode post API response: {exc}")
                        continue
                try:
                    data = json.loads(body)
                except json.JSONDecodeError as exc:
                    record(f"douyin profile: post API returned non-JSON data: {exc}")
                    continue
                previous_log_count = len(logs)
                payload_username, payload_has_more, added = add_douyin_profile_payload(
                    data,
                    sec_uid=sec_uid,
                    posts_by_id=posts_by_id,
                    logs=logs,
                )
                if progress is not None:
                    for added_log in logs[previous_log_count:]:
                        progress(added_log)
                if payload_username:
                    if username != payload_username:
                        record(f"douyin profile: detected username {payload_username!r}")
                    username = payload_username
                if payload_has_more is not None:
                    has_more = payload_has_more
                if added:
                    last_new_post_at = time.monotonic()
                    if collect_all:
                        deadline = last_new_post_at + max(5.0, timeout)
                    record(
                        f"douyin profile: collected {len(posts_by_id)} post(s) after scroll {scroll_count}"
                    )

                if target_count is not None and len(posts_by_id) >= target_count:
                    break
                if has_more is False and not pending_profile_requests:
                    break
            final_state_id = client.send(
                "Runtime.evaluate",
                {"expression": DOUYIN_PROFILE_INFO_SCRIPT, "returnByValue": True},
            )
            final_state_deadline = time.monotonic() + 3.0
            while time.monotonic() < final_state_deadline:
                try:
                    message = client.recv(timeout=final_state_deadline - time.monotonic())
                except TimeoutError:
                    break
                if message is None:
                    break
                if message.get("id") != final_state_id:
                    continue
                result = message.get("result") if isinstance(message.get("result"), dict) else {}
                remote_result = result.get("result") if isinstance(result.get("result"), dict) else {}
                page_username = douyin_profile_username_from_page_state(remote_result.get("value"), sec_uid)
                if page_username != sec_uid:
                    if username != page_username:
                        record(f"douyin profile: detected username {page_username!r}")
                    username = page_username
                break

            if not posts_by_id and received_profile_body:
                record(
                    "douyin profile: no post payloads were available; the profile API may require a logged-in cookie"
                )
            elif not posts_by_id:
                record("douyin profile: timed out before the page returned a post payload")
        except (DevToolsError, OSError, urllib.error.URLError) as exc:
            raise DouyinDownloadError(f"Douyin profile browser collection failed: {exc}") from exc
        finally:
            if client is not None:
                client.close()
            if process is not None:
                if token is not None:
                    token.unregister_process(process)
                if process.poll() is None:
                    terminate_process(process)
                    try:
                        process.wait(timeout=3)
                    except subprocess.TimeoutExpired:
                        terminate_process(process, force=True)
                        process.wait()

    posts = sorted(posts_by_id.values(), key=lambda post: (post.create_time, post.item_id), reverse=True)
    if numeric_limit is not None:
        posts = posts[:numeric_limit]
    record(f"douyin profile: collection finished; username={username!r} posts={len(posts)}")
    return DouyinProfileResult(sec_uid, username, posts, logs)


XIAOHONGSHU_PROFILE_STATE_SCRIPT = r"""
(() => {
  const root = document.scrollingElement || document.documentElement;
  const scrollables = Array.from(document.querySelectorAll('*')).filter((node) => {
    const style = getComputedStyle(node);
    const rect = node.getBoundingClientRect();
    return (
      rect.bottom > 0 && rect.top < innerHeight &&
      node.clientHeight >= 300 &&
      node.scrollHeight > node.clientHeight + 100 &&
      (style.overflowY === 'auto' || style.overflowY === 'scroll')
    );
  });
  if (root.scrollHeight > root.clientHeight + 100) scrollables.push(root);
  const uniqueScrollables = Array.from(new Set(scrollables));
  const positions = uniqueScrollables.map((node) => {
    const before = node.scrollTop;
    node.scrollTop = node.scrollHeight;
    node.dispatchEvent(new Event('scroll', { bubbles: true }));
    return { before, after: node.scrollTop, height: node.scrollHeight };
  });
  if (!uniqueScrollables.length) window.scrollTo(0, root.scrollHeight);
  const state = window.__INITIAL_STATE__ || {};
  const userState = state.user || {};
  const noteColumns = Array.isArray(userState.notes) ? userState.notes : [];
  const notes = noteColumns.flatMap((column) => Array.isArray(column) ? column : []);
  const basicInfo = (((userState.userPageData || {}).basicInfo) || {});
  return {
    title: document.title || '',
    heading: (document.querySelector('h1') || {}).innerText || '',
    bodyText: (document.body.innerText || '').slice(0, 500),
    username: basicInfo.nickname || '',
    notes,
    scrollTargets: positions,
  };
})()
"""

XIAOHONGSHU_PROFILE_INFO_SCRIPT = r"""
(() => {
  const state = window.__INITIAL_STATE__ || {};
  const userState = state.user || {};
  const noteColumns = Array.isArray(userState.notes) ? userState.notes : [];
  const notes = noteColumns.flatMap((column) => Array.isArray(column) ? column : []);
  const basicInfo = (((userState.userPageData || {}).basicInfo) || {});
  return {
    title: document.title || '',
    heading: (document.querySelector('h1') || {}).innerText || '',
    username: basicInfo.nickname || '',
    notes,
  };
})()
"""


def xiaohongshu_note_create_time(note_id: str) -> int:
    """Decode the creation timestamp stored in a Xiaohongshu 24-hex note id."""
    if not re.fullmatch(r"[0-9a-fA-F]{24}", note_id):
        return 0
    try:
        timestamp = int(note_id[:8], 16)
    except ValueError:
        return 0
    if timestamp < 1_262_304_000 or timestamp > int(time.time()) + 86_400:
        return 0
    return timestamp


def extract_xiaohongshu_profile_initial_state(
    page_text: str,
) -> tuple[str | None, list[dict[str, Any]], bool | None]:
    """Extract the first profile page before the hydrated app clears its note columns."""
    marker_index = page_text.find("window.__INITIAL_STATE__")
    if marker_index < 0:
        return None, [], None
    equals_index = page_text.find("=", marker_index)
    object_index = page_text.find("{", equals_index + 1 if equals_index >= 0 else marker_index)
    if object_index < 0:
        return None, [], None
    raw_state = extract_balanced_json_text(page_text, object_index)
    if not raw_state:
        return None, [], None
    # Xiaohongshu embeds JavaScript `undefined` values inside an otherwise JSON
    # object. They are not relevant to profile notes and map cleanly to null.
    normalized_state = re.sub(
        r"(?<=[\[,: ])undefined(?=\s*[,}\]])",
        "null",
        raw_state,
    )
    try:
        state = json.loads(normalized_state)
    except json.JSONDecodeError:
        return None, [], None
    user_state = state.get("user") if isinstance(state, dict) else None
    if not isinstance(user_state, dict):
        return None, [], None

    page_data = user_state.get("userPageData")
    basic_info = page_data.get("basicInfo") if isinstance(page_data, dict) else None
    raw_username = basic_info.get("nickname") if isinstance(basic_info, dict) else None
    username = raw_username.strip() if isinstance(raw_username, str) and raw_username.strip() else None

    notes: list[dict[str, Any]] = []
    note_columns = user_state.get("notes")
    if isinstance(note_columns, list):
        for column in note_columns:
            if not isinstance(column, list):
                continue
            notes.extend(note for note in column if isinstance(note, dict))

    has_more: bool | None = None
    note_queries = user_state.get("noteQueries")
    if isinstance(note_queries, list) and note_queries:
        first_query = note_queries[0]
        if isinstance(first_query, dict):
            raw_has_more = first_query.get("hasMore")
            if raw_has_more is None:
                raw_has_more = first_query.get("has_more")
            if raw_has_more is not None:
                has_more = bool(raw_has_more)
    return username, notes, has_more


def xiaohongshu_profile_username_from_page_state(value: Any, user_id: str) -> str:
    if not isinstance(value, dict):
        return user_id
    username = str(value.get("username") or "").strip()
    if username:
        return username
    title = str(value.get("title") or "").strip()
    title_match = re.match(r"^(.*?)\s*-\s*小红书$", title)
    if title_match and title_match.group(1).strip():
        return title_match.group(1).strip()
    heading = str(value.get("heading") or "").strip()
    return heading or user_id


def add_xiaohongshu_profile_payload(
    data: Any,
    *,
    user_id: str,
    posts_by_id: dict[str, XiaohongshuProfilePost],
    logs: list[str],
) -> tuple[str | None, bool | None, str | None, int]:
    if not isinstance(data, dict):
        return None, None, None, 0
    if data.get("success") is False:
        logs.append(
            "xiaohongshu profile API failed: "
            f"code={data.get('code')} message={data.get('msg') or data.get('message') or '-'}"
        )
    payload = data.get("data") if isinstance(data.get("data"), dict) else data
    notes = payload.get("notes") or payload.get("note_list") or payload.get("noteList") or []
    if not isinstance(notes, list):
        notes = []

    username: str | None = None
    added = 0
    for raw_note in notes:
        if not isinstance(raw_note, dict):
            continue
        note = raw_note.get("note_card") or raw_note.get("noteCard") or raw_note
        if not isinstance(note, dict):
            continue
        item_id = str(
            note.get("note_id")
            or note.get("noteId")
            or raw_note.get("note_id")
            or raw_note.get("noteId")
            or raw_note.get("id")
            or ""
        )
        if not item_id or item_id in posts_by_id:
            continue
        author = note.get("user") if isinstance(note.get("user"), dict) else {}
        author_id = str(author.get("user_id") or author.get("userId") or "")
        if author_id and author_id != user_id:
            continue
        raw_username = author.get("nickname") or author.get("nickName")
        if isinstance(raw_username, str) and raw_username.strip() and not username:
            username = raw_username.strip()
        xsec_token = str(
            note.get("xsec_token")
            or note.get("xsecToken")
            or raw_note.get("xsec_token")
            or raw_note.get("xsecToken")
            or ""
        )
        note_type = str(note.get("type") or raw_note.get("type") or "unknown")
        posts_by_id[item_id] = XiaohongshuProfilePost(
            item_id=item_id,
            create_time=xiaohongshu_note_create_time(item_id),
            xsec_token=xsec_token,
            note_type=note_type,
            payload=raw_note,
        )
        added += 1

    raw_has_more = payload.get("has_more")
    if raw_has_more is None:
        raw_has_more = payload.get("hasMore")
    has_more = bool(raw_has_more) if raw_has_more is not None else None
    raw_cursor = payload.get("cursor")
    cursor = str(raw_cursor) if raw_cursor not in (None, "") else None
    return username, has_more, cursor, added


def gather_xiaohongshu_profile_posts(
    profile_url: str,
    user_id: str,
    *,
    limit: int | str = DEFAULT_PROFILE_LIMIT,
    interval: float = DEFAULT_PROFILE_INTERVAL,
    cookie: str | None = None,
    timeout: float = DEFAULT_BROWSER_TIMEOUT,
    chrome_path: str | None = None,
    progress: Callable[[str], None] | None = None,
    use_system_browser_cookies: bool = True,
) -> XiaohongshuProfileResult:
    """Scroll a Xiaohongshu user page and collect its public note references."""
    collect_all = isinstance(limit, str) and limit.lower() == "all"
    if not collect_all and (not isinstance(limit, int) or limit <= 0):
        raise DouyinDownloadError("--profile-limit must be a positive integer or 'all'.")
    numeric_limit = None if collect_all else int(limit)
    target_label = "all" if collect_all else str(numeric_limit)
    chrome = find_chrome_executable(chrome_path)
    if not chrome:
        raise DouyinDownloadError(
            "A Chromium-compatible browser is required for Xiaohongshu profile downloads."
        )

    logs: list[str] = []

    def record(message: str) -> None:
        logs.append(message)
        if progress is not None:
            progress(message)

    record(f"xiaohongshu profile: opening {profile_url}")
    posts_by_id: dict[str, XiaohongshuProfilePost] = {}
    username = user_id
    has_more: bool | None = None
    try:
        initial_response = http_get(
            profile_url,
            cookie=cookie,
            timeout=min(timeout, 20.0),
            max_bytes=4 * 1024 * 1024,
            extra_headers={"Referer": "https://www.xiaohongshu.com/"},
        )
        initial_text = decode_text(initial_response.content, initial_response.headers)
        initial_username, initial_notes, initial_has_more = extract_xiaohongshu_profile_initial_state(
            initial_text
        )
        if initial_notes:
            payload_username, _payload_has_more, _payload_cursor, initial_added = (
                add_xiaohongshu_profile_payload(
                    {"notes": initial_notes},
                    user_id=user_id,
                    posts_by_id=posts_by_id,
                    logs=logs,
                )
            )
            detected_username = initial_username or payload_username
            if detected_username:
                username = detected_username
            if initial_has_more is not None:
                has_more = initial_has_more
            if initial_added:
                record(
                    "xiaohongshu profile: captured initial HTML immediately; "
                    f"added={initial_added} collected={len(posts_by_id)} has_more={has_more}"
                )
            else:
                record(
                    "xiaohongshu profile: initial HTML note identifiers are unavailable; "
                    "waiting for the hydrated page state"
                )
        else:
            record("xiaohongshu profile: initial HTML contained no note state; using browser collection")
    except (DouyinDownloadError, OSError) as exc:
        record(f"xiaohongshu profile: initial page capture failed; using browser collection: {exc}")
    token = current_cancellation_token()
    process: subprocess.Popen[Any] | None = None
    client: DevToolsConnection | None = None

    with tempfile.TemporaryDirectory(prefix="media_downloader_xhs_profile_", ignore_cleanup_errors=True) as tmpdir:
        profile_dir = Path(tmpdir)
        system_cookie_import: SystemBrowserCookieImport | None = None
        if not cookie and use_system_browser_cookies:
            try:
                system_cookie_import = import_system_xiaohongshu_cookies(profile_dir, chrome)
            except (OSError, sqlite3.Error) as exc:
                record(f"xiaohongshu profile: system browser Cookie import failed: {exc}")
            if system_cookie_import is not None:
                record(
                    "xiaohongshu profile: auto-loaded "
                    f"{system_cookie_import.cookie_count} xiaohongshu.com cookie(s) from "
                    f"system browser profile {system_cookie_import.profile!r}"
                )
        command = [
            chrome,
            "--headless=new",
            "--disable-gpu",
            "--no-sandbox",
            "--disable-dev-shm-usage",
            "--disable-blink-features=AutomationControlled",
            "--mute-audio",
            "--remote-debugging-port=0",
            "--remote-allow-origins=*",
            "--window-size=1280,2000",
            f"--user-agent={DEFAULT_HEADERS['User-Agent']}",
            f"--user-data-dir={tmpdir}",
            "--profile-directory=Default",
            "about:blank",
        ]
        try:
            process = subprocess.Popen(
                command,
                stdout=subprocess.DEVNULL,
                stderr=subprocess.DEVNULL,
                start_new_session=os.name == "posix",
            )
            if token is not None:
                token.register_process(process)
            websocket_url = wait_for_devtools_page_url(profile_dir, process, timeout=timeout)
            client = DevToolsConnection(websocket_url, timeout=min(max(timeout, 1.0), 10.0))
            record("xiaohongshu profile: browser collector is ready")

            pending_profile_requests: dict[str, str] = {}
            pending_commands: dict[int, tuple[str, Any]] = {}
            received_profile_body = False
            last_new_post_at = time.monotonic()
            scroll_count = 0
            response_count = 0
            initial_state_complete = bool(posts_by_id)
            initial_state_attempts = 0

            client.send(
                "Network.enable",
                {
                    "maxTotalBufferSize": 100 * 1024 * 1024,
                    "maxResourceBufferSize": 16 * 1024 * 1024,
                },
            )
            client.send("Page.enable")
            client.send("Runtime.enable")
            cookie_params = cookie_params_for_xiaohongshu(cookie)
            if cookie_params:
                pending_commands[client.send("Network.setCookies", {"cookies": cookie_params})] = (
                    "cookies",
                    None,
                )
                record(f"xiaohongshu profile: injected {len(cookie_params)} cookie(s) into Chrome")
            elif system_cookie_import is not None:
                record("xiaohongshu profile: using the system browser Xiaohongshu login state")
            else:
                record("xiaohongshu profile: no login Cookie was supplied")
            client.send("Page.navigate", {"url": profile_url})

            deadline = time.monotonic() + max(10.0, timeout)
            request_interval = max(1.0, interval)
            inactivity_timeout = max(8.0, request_interval * 3)
            next_scroll_at = time.monotonic() + max(3.0, request_interval)
            next_initial_state_at = time.monotonic() + 0.5
            while time.monotonic() < deadline:
                raise_if_task_cancelled()
                now = time.monotonic()
                if posts_by_id and now - last_new_post_at >= inactivity_timeout and scroll_count >= 2:
                    break
                if (
                    not initial_state_complete
                    and initial_state_attempts < 20
                    and now >= next_initial_state_at
                ):
                    command_id = client.send(
                        "Runtime.evaluate",
                        {"expression": XIAOHONGSHU_PROFILE_INFO_SCRIPT, "returnByValue": True},
                    )
                    pending_commands[command_id] = ("initial_state", None)
                    initial_state_attempts += 1
                    next_initial_state_at = now + 0.5
                if now >= next_scroll_at:
                    command_id = client.send(
                        "Runtime.evaluate",
                        {"expression": XIAOHONGSHU_PROFILE_STATE_SCRIPT, "returnByValue": True},
                    )
                    pending_commands[command_id] = ("page_state", None)
                    scroll_count += 1
                    record(
                        f"xiaohongshu profile: scrolling page {scroll_count}; "
                        f"collected={len(posts_by_id)} target={target_label}"
                    )
                    next_scroll_at = now + request_interval

                wait_candidates = [
                    0.5,
                    max(0.05, deadline - now),
                    max(0.05, next_scroll_at - now),
                ]
                if not initial_state_complete and initial_state_attempts < 20:
                    wait_candidates.append(max(0.05, next_initial_state_at - now))
                wait_time = min(wait_candidates)
                try:
                    message = client.recv(timeout=wait_time)
                except TimeoutError:
                    continue
                if message is None:
                    record("xiaohongshu profile: Chrome DevTools connection closed early")
                    break
                method = message.get("method")
                params = message.get("params") if isinstance(message.get("params"), dict) else {}
                if method == "Network.responseReceived":
                    response = params.get("response") if isinstance(params.get("response"), dict) else {}
                    response_url = str(response.get("url") or "")
                    if "/api/sns/web/v1/user_posted" in response_url:
                        request_id = str(params.get("requestId") or "")
                        if request_id:
                            pending_profile_requests[request_id] = response_url
                    continue
                if method == "Network.loadingFinished":
                    request_id = str(params.get("requestId") or "")
                    response_url = pending_profile_requests.pop(request_id, None)
                    if response_url:
                        command_id = client.send("Network.getResponseBody", {"requestId": request_id})
                        pending_commands[command_id] = ("profile_body", response_url)
                    continue
                if method == "Network.loadingFailed":
                    request_id = str(params.get("requestId") or "")
                    if pending_profile_requests.pop(request_id, None):
                        record(
                            "xiaohongshu profile: note request failed: "
                            f"{params.get('errorText') or 'unknown error'}"
                        )
                    continue

                command_id = message.get("id")
                if not isinstance(command_id, int) or command_id not in pending_commands:
                    continue
                kind, _context = pending_commands.pop(command_id)
                if message.get("error"):
                    error = message["error"]
                    if kind == "profile_body" and "No resource with given identifier" in str(error):
                        record(
                            "xiaohongshu profile: cached initial response body is unavailable; "
                            "using the page state instead"
                        )
                    else:
                        record(f"xiaohongshu profile: DevTools {kind} failed: {error}")
                    continue
                result = message.get("result") if isinstance(message.get("result"), dict) else {}
                if kind == "cookies":
                    continue
                if kind in {"initial_state", "page_state"}:
                    remote_result = result.get("result") if isinstance(result.get("result"), dict) else {}
                    page_value = remote_result.get("value")
                    page_username = xiaohongshu_profile_username_from_page_state(
                        page_value,
                        user_id,
                    )
                    if page_username != user_id:
                        if username != page_username:
                            record(f"xiaohongshu profile: detected username {page_username!r}")
                        username = page_username
                    if isinstance(page_value, dict) and isinstance(page_value.get("notes"), list):
                        _state_username, _state_has_more, _state_cursor, state_added = (
                            add_xiaohongshu_profile_payload(
                                {"notes": page_value["notes"]},
                                user_id=user_id,
                                posts_by_id=posts_by_id,
                                logs=logs,
                            )
                        )
                        if state_added:
                            initial_state_complete = True
                            last_new_post_at = time.monotonic()
                            if collect_all:
                                deadline = last_new_post_at + max(10.0, timeout)
                            record(
                                "xiaohongshu profile: collected "
                                f"{len(posts_by_id)} post(s) from hydrated page state"
                            )
                    if numeric_limit is not None and len(posts_by_id) >= numeric_limit:
                        break
                    continue
                if kind != "profile_body":
                    continue

                received_profile_body = True
                response_count += 1
                record(f"xiaohongshu profile: processing note response {response_count}")
                body = result.get("body")
                if not isinstance(body, str) or not body:
                    record("xiaohongshu profile: note API returned an empty response")
                    continue
                if result.get("base64Encoded"):
                    try:
                        body = base64.b64decode(body).decode("utf-8", errors="replace")
                    except (ValueError, UnicodeDecodeError) as exc:
                        record(f"xiaohongshu profile: could not decode note API response: {exc}")
                        continue
                try:
                    data = json.loads(body)
                except json.JSONDecodeError as exc:
                    record(f"xiaohongshu profile: note API returned non-JSON data: {exc}")
                    continue
                previous_log_count = len(logs)
                payload_username, payload_has_more, _cursor, added = add_xiaohongshu_profile_payload(
                    data,
                    user_id=user_id,
                    posts_by_id=posts_by_id,
                    logs=logs,
                )
                if progress is not None:
                    for added_log in logs[previous_log_count:]:
                        progress(added_log)
                if payload_username:
                    if username != payload_username:
                        record(f"xiaohongshu profile: detected username {payload_username!r}")
                    username = payload_username
                if payload_has_more is not None:
                    has_more = payload_has_more
                if added:
                    last_new_post_at = time.monotonic()
                    if collect_all:
                        deadline = last_new_post_at + max(10.0, timeout)
                    record(
                        f"xiaohongshu profile: collected {len(posts_by_id)} post(s) "
                        f"after scroll {scroll_count}"
                    )
                record(
                    f"xiaohongshu profile: note page {response_count} "
                    f"added={added} has_more={payload_has_more} cursor={_cursor or '-'}"
                )
                if numeric_limit is not None and len(posts_by_id) >= numeric_limit:
                    break
                if has_more is False and not pending_profile_requests:
                    break

            if not posts_by_id and received_profile_body:
                record("xiaohongshu profile: no note payloads were available; login may be required")
            elif not posts_by_id:
                record("xiaohongshu profile: timed out before the page returned note data")
        except (DevToolsError, OSError, urllib.error.URLError) as exc:
            raise DouyinDownloadError(f"Xiaohongshu profile browser collection failed: {exc}") from exc
        finally:
            if client is not None:
                client.close()
            if process is not None:
                if token is not None:
                    token.unregister_process(process)
                if process.poll() is None:
                    terminate_process(process)
                    try:
                        process.wait(timeout=3)
                    except subprocess.TimeoutExpired:
                        terminate_process(process, force=True)
                        process.wait()

    posts = sorted(posts_by_id.values(), key=lambda post: (post.create_time, post.item_id), reverse=True)
    if numeric_limit is not None:
        posts = posts[:numeric_limit]
    record(f"xiaohongshu profile: collection finished; username={username!r} posts={len(posts)}")
    return XiaohongshuProfileResult(user_id, username, posts, logs)


def detail_api_urls(aweme_id: str) -> list[str]:
    encoded = urllib.parse.quote(aweme_id)
    return [
        f"https://www.iesdouyin.com/web/api/v2/aweme/iteminfo/?item_ids={encoded}",
        (
            "https://www.douyin.com/aweme/v1/web/aweme/detail/"
            f"?aweme_id={encoded}&aid=6383&device_platform=webapp"
        ),
    ]


def gather_candidates(
    share_text: str,
    *,
    cookie: str | None = None,
    timeout: float = 20.0,
) -> tuple[str | None, list[Candidate], list[ImageCandidate], list[str]]:
    """Resolve a share message and gather candidate video URLs."""
    urls = extract_urls(share_text)
    if not urls:
        if share_text.strip().startswith(("http://", "https://")):
            urls = [share_text.strip()]
        else:
            raise DouyinDownloadError("No URL found in the share text.")

    logs: list[str] = []
    all_candidates: list[Candidate] = []
    image_candidates: list[ImageCandidate] = []
    seen_image_urls: set[str] = set()
    seen_candidates: set[str] = set()
    aweme_id = extract_aweme_id(share_text)
    exact_payload_found = False

    for share_url in urls:
        logs.append(f"Resolving {share_url}")
        resolved = http_get(share_url, cookie=cookie, timeout=timeout, max_bytes=4 * 1024 * 1024)
        page_text = decode_text(resolved.content, resolved.headers)
        logs.append(f"Final URL: {resolved.url}")
        aweme_id = aweme_id or extract_aweme_id(resolved.url, page_text)

        exact_payload = find_douyin_aweme_payload_in_html(page_text, aweme_id)
        if exact_payload is not None:
            exact_candidates, exact_images = extract_douyin_item_media_candidates(
                exact_payload,
                source="douyin.page-item",
            )
            if exact_candidates or exact_images:
                if not exact_payload_found:
                    all_candidates.clear()
                    image_candidates.clear()
                    seen_candidates.clear()
                    seen_image_urls.clear()
                exact_payload_found = True
                logs.append(f"Matched exact requested item payload in page HTML: {aweme_id}")
                for candidate in exact_candidates:
                    add_candidate(
                        all_candidates,
                        seen_candidates,
                        candidate.url,
                        candidate.source,
                        candidate.priority,
                        cookie=candidate.cookie,
                        referer=candidate.referer,
                        live_photo_audio_url=candidate.live_photo_audio_url,
                        live_photo_duration=candidate.live_photo_duration,
                    )
                for image_candidate in exact_images:
                    if image_candidate.url in seen_image_urls:
                        continue
                    seen_image_urls.add(image_candidate.url)
                    image_candidates.append(image_candidate)
        elif not exact_payload_found:
            for candidate in extract_candidates_from_html(page_text):
                add_candidate(all_candidates, seen_candidates, candidate.url, candidate.source, candidate.priority)
            for image_candidate in extract_douyin_image_candidates_from_text(page_text):
                if image_candidate.url in seen_image_urls:
                    continue
                seen_image_urls.add(image_candidate.url)
                image_candidates.append(image_candidate)

    if aweme_id:
        for api_url in detail_api_urls(aweme_id):
            try:
                logs.append(f"Fetching detail API: {api_url}")
                result = http_get(api_url, cookie=cookie, timeout=timeout, max_bytes=8 * 1024 * 1024)
            except DouyinDownloadError as exc:
                logs.append(str(exc))
                continue

            payload = load_json_bytes(result.content)
            if payload is None:
                logs.append(f"Detail API returned non-JSON: {result.url}")
                continue
            if isinstance(payload, dict):
                status_code = payload.get("status_code")
                status_msg = payload.get("status_msg")
                if status_code not in (None, 0):
                    logs.append(f"Detail API returned status_code={status_code}: {status_msg or '<no message>'}")

            exact_payload = find_douyin_aweme_payload(payload, aweme_id)
            if exact_payload is not None:
                exact_candidates, exact_images = extract_douyin_item_media_candidates(
                    exact_payload,
                    source="douyin.detail-item",
                )
                if exact_candidates or exact_images:
                    if not exact_payload_found:
                        all_candidates.clear()
                        image_candidates.clear()
                        seen_candidates.clear()
                        seen_image_urls.clear()
                    exact_payload_found = True
                    logs.append(f"Matched exact requested item payload in detail API: {aweme_id}")
                    for candidate in exact_candidates:
                        add_candidate(
                            all_candidates,
                            seen_candidates,
                            candidate.url,
                            candidate.source,
                            candidate.priority,
                            cookie=candidate.cookie,
                            referer=candidate.referer,
                            live_photo_audio_url=candidate.live_photo_audio_url,
                            live_photo_duration=candidate.live_photo_duration,
                        )
                    for image_candidate in exact_images:
                        if image_candidate.url in seen_image_urls:
                            continue
                        seen_image_urls.add(image_candidate.url)
                        image_candidates.append(image_candidate)
                    continue

            if not exact_payload_found:
                for candidate in extract_candidates_from_json(payload):
                    add_candidate(all_candidates, seen_candidates, candidate.url, candidate.source, candidate.priority)

    return (
        aweme_id,
        sorted(all_candidates, key=lambda candidate: candidate.priority),
        sorted(image_candidates, key=lambda candidate: candidate.priority),
        logs,
    )


def platform_referer(platform: str) -> str:
    platform = normalize_platform(platform)
    return {
        "douyin": "https://www.douyin.com/",
        "kuaishou": "https://www.kuaishou.com/",
        "xiaohongshu": "https://www.xiaohongshu.com/",
        "tiktok": "https://www.tiktok.com/",
        "youtube": "https://www.youtube.com/",
    }.get(platform, "https://www.douyin.com/")


def extract_platform_id(platform: str, *parts: str) -> str | None:
    platform = normalize_platform(platform)
    if platform == "kuaishou":
        return extract_kuaishou_id(*parts)
    if platform == "xiaohongshu":
        return extract_xiaohongshu_id(*parts)
    if platform == "tiktok":
        return extract_tiktok_id(*parts)
    if platform == "youtube":
        return extract_youtube_id(*parts)
    return extract_aweme_id(*parts)


def gather_youtube_candidates(share_text: str) -> tuple[str | None, list[Candidate], list[ImageCandidate], list[str]]:
    urls = extract_urls(share_text)
    if not urls:
        if share_text.strip().startswith(("http://", "https://")):
            urls = [share_text.strip()]
        else:
            raise DouyinDownloadError("No URL found in the share text.")

    url = urls[0]
    return (
        extract_youtube_id(share_text, url),
        [Candidate(url, "youtube.yt-dlp", 1, referer=platform_referer("youtube"))],
        [],
        [f"youtube: using yt-dlp for {url}"],
    )


def gather_web_platform_candidates(
    share_text: str,
    *,
    platform: str,
    cookie: str | None = None,
    timeout: float = 20.0,
) -> tuple[str | None, list[Candidate], list[ImageCandidate], list[str]]:
    platform = normalize_platform(platform)
    urls = extract_urls(share_text)
    if not urls:
        if share_text.strip().startswith(("http://", "https://")):
            urls = [share_text.strip()]
        else:
            raise DouyinDownloadError("No URL found in the share text.")

    logs: list[str] = []
    all_candidates: list[Candidate] = []
    image_candidates: list[ImageCandidate] = []
    seen_image_urls: set[str] = set()
    seen_candidates: set[str] = set()
    item_id = extract_platform_id(platform, share_text)
    headers = {
        "Referer": platform_referer(platform),
        "Origin": platform_referer(platform).rstrip("/"),
    }

    for share_url in urls:
        logs.append(f"{platform}: resolving {share_url}")
        if platform == "tiktok":
            resolved, session_cookie = http_get_with_session_cookies(
                share_url,
                cookie=cookie,
                timeout=timeout,
                max_bytes=6 * 1024 * 1024,
                extra_headers=headers,
            )
        else:
            resolved = http_get(
                share_url,
                cookie=cookie,
                timeout=timeout,
                max_bytes=6 * 1024 * 1024,
                extra_headers=headers,
            )
            session_cookie = None
        page_text = decode_text(resolved.content, resolved.headers)
        logs.append(f"{platform}: final URL: {resolved.url}")
        item_id = item_id or extract_platform_id(platform, resolved.url, page_text)

        for candidate in extract_platform_candidates_from_html(page_text, platform):
            add_platform_candidate(
                all_candidates,
                seen_candidates,
                candidate.url,
                candidate.source,
                candidate.priority,
                platform,
                cookie=session_cookie if platform == "tiktok" else candidate.cookie,
                referer=resolved.url if platform == "tiktok" else candidate.referer,
            )
        if platform == "xiaohongshu":
            for image_candidate in extract_xiaohongshu_image_candidates_from_text(page_text):
                if image_candidate.url in seen_image_urls:
                    continue
                seen_image_urls.add(image_candidate.url)
                image_candidates.append(image_candidate)

    return (
        item_id,
        sorted(all_candidates, key=lambda candidate: candidate.priority),
        sorted(image_candidates, key=lambda candidate: candidate.priority),
        logs,
    )


def gather_candidates_for_request(
    share_text: str,
    *,
    platform: str,
    cookie: str | None = None,
    timeout: float = 20.0,
    browser_fallback: bool = True,
    browser_timeout: float = DEFAULT_BROWSER_TIMEOUT,
    chrome_path: str | None = None,
    require_audio: bool = False,
    use_system_browser_cookies: bool = True,
) -> tuple[str, str | None, list[Candidate], list[ImageCandidate], list[str]]:
    platform = normalize_platform(platform)
    resolved_platform = detect_platform(share_text) if platform == "auto" else platform
    if not resolved_platform:
        raise DouyinDownloadError(
            "Cannot detect platform from the share text. Pass --platform douyin, "
            "--platform kuaishou, --platform xiaohongshu, --platform tiktok, or --platform youtube."
        )

    if resolved_platform == "youtube":
        item_id, candidates, image_candidates, logs = gather_youtube_candidates(share_text)
        return resolved_platform, item_id, candidates, image_candidates, logs

    if resolved_platform == "douyin":
        item_id, candidates, image_candidates, logs = gather_candidates(share_text, cookie=cookie, timeout=timeout)
        exact_direct_match = any(
            line.startswith("Matched exact requested item payload")
            for line in logs
        )
        needs_browser = (
            not candidates
            and not image_candidates
        ) or bool(item_id and not exact_direct_match)
        if browser_fallback and needs_browser:
            if candidates or image_candidates:
                logs.append(
                    "douyin: direct media was not verified against the requested item ID; "
                    "trying exact-item browser collection"
                )
            else:
                logs.append("douyin: direct extraction found no candidates, trying browser fallback")
            browser_item_id, browser_candidates, browser_image_candidates, browser_logs = gather_browser_candidates(
                share_text,
                platform=resolved_platform,
                cookie=cookie,
                timeout=browser_timeout,
                chrome_path=chrome_path,
                require_audio=require_audio,
                use_system_browser_cookies=use_system_browser_cookies,
            )
            logs.extend(browser_logs)
            item_id = item_id or browser_item_id
            if browser_candidates or browser_image_candidates:
                candidates = browser_candidates
                image_candidates = browser_image_candidates
        return resolved_platform, item_id, candidates, image_candidates, logs

    if resolved_platform in {"kuaishou", "xiaohongshu", "tiktok"}:
        item_id, candidates, image_candidates, platform_logs = gather_web_platform_candidates(
            share_text,
            platform=resolved_platform,
            cookie=cookie,
            timeout=timeout,
        )
        if not candidates and not image_candidates and browser_fallback:
            platform_logs.append(f"{resolved_platform}: direct extraction found no candidates, trying browser fallback")
            browser_item_id, browser_candidates, browser_image_candidates, browser_logs = gather_browser_candidates(
                share_text,
                platform=resolved_platform,
                cookie=cookie,
                timeout=browser_timeout,
                chrome_path=chrome_path,
                require_audio=require_audio,
                use_system_browser_cookies=use_system_browser_cookies,
            )
            platform_logs.extend(browser_logs)
            item_id = item_id or browser_item_id
            candidates = browser_candidates
            image_candidates = browser_image_candidates
        return resolved_platform, item_id, candidates, image_candidates, platform_logs

    raise DouyinDownloadError(f"Unsupported platform: {resolved_platform}")


def unique_output_path(path: Path) -> Path:
    if not path.exists():
        return path
    stem = path.stem
    suffix = path.suffix
    parent = path.parent
    for index in range(1, 1000):
        candidate = parent / f"{stem}.{index}{suffix}"
        if not candidate.exists():
            return candidate
    raise DouyinDownloadError(f"Cannot find a free output filename near {path}")


def timestamp_output_name() -> str:
    return f"{time.strftime(OUTPUT_TIME_FORMAT)}.mp4"


def timestamp_output_stem() -> str:
    return time.strftime(OUTPUT_TIME_FORMAT)


def content_type_is_video(headers: dict[str, str]) -> bool:
    content_type = headers.get("content-type", "").lower()
    if not content_type:
        return True
    return content_type.startswith("video/") or "octet-stream" in content_type


def content_type_is_image(headers: dict[str, str]) -> bool:
    content_type = headers.get("content-type", "").lower()
    if not content_type:
        return True
    return content_type.startswith("image/") or "octet-stream" in content_type


def image_suffix_from_url(url: str) -> str:
    path = urllib.parse.urlsplit(unwrap_url(url, decode_percent=False)).path.lower()
    for suffix in (".webp", ".jpg", ".jpeg", ".png", ".avif", ".gif"):
        if suffix in path:
            return ".jpg" if suffix == ".jpeg" else suffix
    return ".jpg"


def image_suffix_from_content_type(content_type: str | None, fallback: str) -> str:
    lowered = (content_type or "").lower().split(";", 1)[0].strip()
    return {
        "image/webp": ".webp",
        "image/jpeg": ".jpg",
        "image/jpg": ".jpg",
        "image/png": ".png",
        "image/avif": ".avif",
        "image/gif": ".gif",
    }.get(lowered, fallback)


def download_candidate(
    candidate: Candidate,
    output_path: Path,
    *,
    cookie: str | None = None,
    timeout: float = 30.0,
    referer: str | None = None,
) -> Path:
    raise_if_task_cancelled()
    effective_cookie = candidate.cookie or cookie
    effective_referer = candidate.referer or referer or "https://www.douyin.com/"
    headers = build_headers(
        effective_cookie,
        {
            "Accept": "*/*",
            "Referer": effective_referer,
        },
    )
    request = urllib.request.Request(candidate.url, headers=headers)

    try:
        with urllib.request.urlopen(request, timeout=timeout) as response:
            response_headers = {k.lower(): v for k, v in response.headers.items()}
            if not content_type_is_video(response_headers):
                content_type = response_headers.get("content-type", "unknown")
                raise DouyinDownloadError(
                    f"Candidate returned non-video content ({content_type}) from {candidate.source}"
                )

            tmp_path = output_path.with_suffix(output_path.suffix + ".part")
            try:
                with tmp_path.open("wb") as fp:
                    while True:
                        raise_if_task_cancelled()
                        chunk = response.read(1024 * 256)
                        if not chunk:
                            break
                        fp.write(chunk)
            except OperationCancelled:
                tmp_path.unlink(missing_ok=True)
                raise
            tmp_path.replace(output_path)
            return output_path
    except urllib.error.HTTPError as exc:
        raise DouyinDownloadError(f"HTTP {exc.code} while downloading {candidate.url}") from exc
    except urllib.error.URLError as exc:
        raise DouyinDownloadError(f"Network error while downloading {candidate.url}: {exc.reason}") from exc


def download_live_photo_audio(
    url: str,
    output_path: Path,
    *,
    cookie: str | None,
    timeout: float,
    referer: str,
) -> Path:
    headers = build_headers(
        cookie,
        {
            "Accept": "audio/*,*/*;q=0.8",
            "Referer": referer,
        },
    )
    request = urllib.request.Request(url, headers=headers)
    print("live_photo_audio: downloading complete soundtrack", file=sys.stderr, flush=True)
    try:
        with urllib.request.urlopen(request, timeout=timeout) as response:
            try:
                total_bytes = int(response.headers.get("Content-Length") or 0)
            except (TypeError, ValueError):
                total_bytes = 0
            downloaded_bytes = 0
            last_reported_percent = -10
            temporary_path = output_path.with_suffix(output_path.suffix + ".part")
            try:
                with temporary_path.open("wb") as fp:
                    while True:
                        raise_if_task_cancelled()
                        chunk = response.read(1024 * 256)
                        if not chunk:
                            break
                        fp.write(chunk)
                        downloaded_bytes += len(chunk)
                        if total_bytes > 0:
                            percent = min(100, int(downloaded_bytes * 100 / total_bytes))
                            if percent >= last_reported_percent + 10 or percent == 100:
                                last_reported_percent = percent
                                print(
                                    f"live_photo_audio_progress: {percent}% "
                                    f"({downloaded_bytes}/{total_bytes} bytes)",
                                    file=sys.stderr,
                                    flush=True,
                                )
                temporary_path.replace(output_path)
            except OperationCancelled:
                temporary_path.unlink(missing_ok=True)
                raise
    except urllib.error.HTTPError as exc:
        raise DouyinDownloadError(f"HTTP {exc.code} while downloading live-photo audio") from exc
    except urllib.error.URLError as exc:
        raise DouyinDownloadError(
            f"Network error while downloading live-photo audio: {exc.reason}"
        ) from exc
    return output_path


def compose_live_photo_video(
    clip_path: Path,
    candidate: Candidate,
    *,
    cookie: str | None,
    timeout: float,
    referer: str,
    verbose: bool,
) -> Path:
    duration = candidate.live_photo_duration
    if duration is None:
        return clip_path
    if duration <= 0 or duration > 24 * 60 * 60:
        raise DouyinDownloadError(f"Invalid live-photo playback duration: {duration}")

    ffmpeg = shutil.which("ffmpeg")
    if not ffmpeg:
        raise DouyinDownloadError(
            "ffmpeg is required to reconstruct a complete Douyin live-photo video."
        )

    audio_path: Path | None = None
    audio_descriptor: int | None = None
    output_descriptor: int | None = None
    composed_path: Path | None = None
    try:
        if candidate.live_photo_audio_url:
            audio_descriptor, raw_audio_path = tempfile.mkstemp(
                prefix=f".{clip_path.stem}_",
                suffix=".live-photo-audio",
                dir=clip_path.parent,
            )
            os.close(audio_descriptor)
            audio_descriptor = None
            audio_path = Path(raw_audio_path)
            download_live_photo_audio(
                candidate.live_photo_audio_url,
                audio_path,
                cookie=candidate.cookie or cookie,
                timeout=timeout,
                referer=candidate.referer or referer,
            )

        output_descriptor, raw_composed_path = tempfile.mkstemp(
            prefix=f".{clip_path.stem}_",
            suffix=".live-photo.mp4",
            dir=clip_path.parent,
        )
        os.close(output_descriptor)
        output_descriptor = None
        composed_path = Path(raw_composed_path)

        command = [
            ffmpeg,
            "-hide_banner",
            "-loglevel",
            "error",
            "-y",
            "-stream_loop",
            "-1",
            "-i",
            str(clip_path),
        ]
        if audio_path is not None:
            command.extend(["-i", str(audio_path)])
        command.extend(
            [
                "-t",
                f"{duration:.3f}",
                "-map",
                "0:v:0",
            ]
        )
        if audio_path is not None:
            command.extend(
                [
                    "-map",
                    "1:a:0",
                    "-c:v",
                    "copy",
                    "-c:a",
                    "aac",
                    "-b:a",
                    "192k",
                ]
            )
        else:
            command.extend(["-map", "0:a?", "-c", "copy"])
        command.extend(["-movflags", "+faststart", str(composed_path)])

        print(
            f"live_photo_compose: looping {duration:.3f}s clip with "
            f"{'complete soundtrack' if audio_path is not None else 'embedded audio'}",
            file=sys.stderr,
            flush=True,
        )
        if verbose:
            print("live_photo_ffmpeg: " + shlex.join(command), file=sys.stderr, flush=True)
        completed = run_task_subprocess(
            command,
            check=False,
            capture_output=True,
            text=True,
        )
        if completed.returncode != 0:
            detail = (completed.stderr or completed.stdout or "").strip()
            raise DouyinDownloadError(
                "ffmpeg could not reconstruct the complete live-photo video"
                + (f": {detail}" if detail else "")
            )
        if not composed_path.is_file() or composed_path.stat().st_size == 0:
            raise DouyinDownloadError(
                "ffmpeg completed without creating the live-photo video."
            )
        composed_path.replace(clip_path)
        composed_path = None
        print(
            f"live_photo_completed: duration={duration:.3f}s path={clip_path}",
            file=sys.stderr,
            flush=True,
        )
        return clip_path
    finally:
        if audio_descriptor is not None:
            os.close(audio_descriptor)
        if output_descriptor is not None:
            os.close(output_descriptor)
        if audio_path is not None:
            audio_path.unlink(missing_ok=True)
            audio_path.with_suffix(audio_path.suffix + ".part").unlink(missing_ok=True)
        if composed_path is not None:
            composed_path.unlink(missing_ok=True)


def find_yt_dlp_binary(yt_dlp_bin: str | None = None) -> str:
    if yt_dlp_bin:
        expanded = Path(yt_dlp_bin).expanduser()
        if expanded.exists():
            return str(expanded)
        found = shutil.which(yt_dlp_bin)
        if found:
            return found
        raise DouyinDownloadError(f"yt-dlp binary was not found: {yt_dlp_bin}")

    found = shutil.which("yt-dlp")
    if not found:
        raise DouyinDownloadError("yt-dlp is required for YouTube downloads but was not found in PATH.")
    return found


def output_stem_exists(output_dir: Path, stem: str) -> bool:
    return any(path.is_file() and path.stem == stem for path in output_dir.iterdir())


def unique_output_stem(output_dir: Path, stem: str) -> str:
    if not output_stem_exists(output_dir, stem):
        return stem
    for index in range(1, 1000):
        candidate = f"{stem}.{index}"
        if not output_stem_exists(output_dir, candidate):
            return candidate
    raise DouyinDownloadError(f"Cannot find a free output filename stem near {output_dir / stem}")


def youtube_output_template(output_dir: Path, output_name: str | None, *, overwrite: bool = False) -> Path:
    output_dir.mkdir(parents=True, exist_ok=True)
    if output_name:
        output_path = output_dir / output_name
        stem = output_path.stem if output_path.suffix else output_path.name
    else:
        stem = timestamp_output_stem()

    if not overwrite:
        stem = unique_output_stem(output_dir, stem)
    return output_dir / f"{stem}.%(ext)s"


def build_youtube_command(
    yt_dlp_bin: str,
    url: str,
    *,
    output_template: Path | None = None,
    format_selector: str = DEFAULT_YOUTUBE_FORMAT,
    cookie: str | None = None,
    timeout: float = 20.0,
    overwrite: bool = False,
    print_url: bool = False,
) -> list[str]:
    command = [
        yt_dlp_bin,
        "--no-playlist",
        "--socket-timeout",
        f"{timeout:g}",
        "-f",
        format_selector,
    ]
    if cookie:
        command.extend(["--add-header", f"Cookie: {cookie}"])
    if print_url:
        command.append("--get-url")
    else:
        if output_template is None:
            raise DouyinDownloadError("YouTube output template is required for download.")
        command.extend(
            [
                "--merge-output-format",
                "mp4",
                "--print",
                "after_move:filepath",
                "-o",
                str(output_template),
            ]
        )
        command.append("--force-overwrites" if overwrite else "--no-overwrites")
    command.append(url)
    return command


def ytdlp_error_message(completed: subprocess.CompletedProcess[str]) -> str:
    detail = completed.stderr.strip() or completed.stdout.strip()
    if not detail:
        return f"yt-dlp failed with exit code {completed.returncode}"
    return f"yt-dlp failed with exit code {completed.returncode}: {detail.splitlines()[-1]}"


def ytdlp_stdout_path(stdout: str) -> Path | None:
    lines = [line.strip() for line in stdout.splitlines() if line.strip()]
    if not lines:
        return None
    return Path(lines[-1]).expanduser()


def find_downloaded_youtube_output(output_template: Path) -> Path | None:
    template_name = output_template.name
    marker = "%(ext)s"
    if marker not in template_name:
        return output_template if output_template.exists() else None
    prefix, suffix = template_name.split(marker, 1)
    matches = sorted(
        (
            path
            for path in output_template.parent.iterdir()
            if path.is_file() and path.name.startswith(prefix) and path.name.endswith(suffix)
        ),
        key=lambda path: path.stat().st_mtime,
    )
    return matches[-1] if matches else None


def download_youtube_video(
    candidate: Candidate,
    output_dir: Path,
    *,
    output_name: str | None = None,
    yt_dlp_bin: str | None = None,
    format_selector: str = DEFAULT_YOUTUBE_FORMAT,
    cookie: str | None = None,
    timeout: float = 20.0,
    overwrite: bool = False,
    verbose: bool = False,
) -> Path:
    executable = find_yt_dlp_binary(yt_dlp_bin)
    output_template = youtube_output_template(output_dir, output_name, overwrite=overwrite)
    command = build_youtube_command(
        executable,
        candidate.url,
        output_template=output_template,
        format_selector=format_selector,
        cookie=candidate.cookie or cookie,
        timeout=timeout,
        overwrite=overwrite,
    )
    if verbose:
        print(" ".join(command), file=sys.stderr)
    completed = run_task_subprocess(command, check=False, capture_output=True, text=True)
    if completed.returncode != 0:
        raise DouyinDownloadError(ytdlp_error_message(completed))

    saved_path = ytdlp_stdout_path(completed.stdout) or find_downloaded_youtube_output(output_template)
    if saved_path is None:
        raise DouyinDownloadError("yt-dlp completed but did not report an output file.")
    return saved_path


def print_youtube_media_urls(
    candidate: Candidate,
    *,
    yt_dlp_bin: str | None = None,
    format_selector: str = DEFAULT_YOUTUBE_FORMAT,
    cookie: str | None = None,
    timeout: float = 20.0,
    verbose: bool = False,
) -> None:
    executable = find_yt_dlp_binary(yt_dlp_bin)
    command = build_youtube_command(
        executable,
        candidate.url,
        format_selector=format_selector,
        cookie=candidate.cookie or cookie,
        timeout=timeout,
        print_url=True,
    )
    if verbose:
        print(" ".join(command), file=sys.stderr)
    completed = run_task_subprocess(command, check=False, capture_output=True, text=True)
    if completed.returncode != 0:
        raise DouyinDownloadError(ytdlp_error_message(completed))
    print(completed.stdout.rstrip())


def download_image_candidate(
    candidate: ImageCandidate,
    output_path: Path,
    *,
    cookie: str | None = None,
    timeout: float = 30.0,
    referer: str | None = None,
) -> Path:
    raise_if_task_cancelled()
    headers = build_headers(
        cookie,
        {
            "Accept": "image/avif,image/webp,image/apng,image/*,*/*;q=0.8",
            "Referer": referer or "https://www.douyin.com/",
        },
    )
    request = urllib.request.Request(candidate.url, headers=headers)

    try:
        with urllib.request.urlopen(request, timeout=timeout) as response:
            response_headers = {k.lower(): v for k, v in response.headers.items()}
            if not content_type_is_image(response_headers):
                content_type = response_headers.get("content-type", "unknown")
                raise DouyinDownloadError(
                    f"Candidate returned non-image content ({content_type}) from {candidate.source}"
                )

            suffix = image_suffix_from_content_type(
                response_headers.get("content-type"),
                output_path.suffix or image_suffix_from_url(candidate.url),
            )
            if output_path.suffix.lower() != suffix:
                output_path = output_path.with_suffix(suffix)
            tmp_path = output_path.with_suffix(output_path.suffix + ".part")
            try:
                with tmp_path.open("wb") as fp:
                    while True:
                        raise_if_task_cancelled()
                        chunk = response.read(1024 * 256)
                        if not chunk:
                            break
                        fp.write(chunk)
            except OperationCancelled:
                tmp_path.unlink(missing_ok=True)
                raise
            tmp_path.replace(output_path)
            return output_path
    except urllib.error.HTTPError as exc:
        raise DouyinDownloadError(f"HTTP {exc.code} while downloading image {candidate.url}") from exc
    except urllib.error.URLError as exc:
        raise DouyinDownloadError(f"Network error while downloading image {candidate.url}: {exc.reason}") from exc


def download_image_candidates(
    candidates: list[ImageCandidate],
    output_dir: Path,
    *,
    output_name: str | None = None,
    overwrite: bool = False,
    cookie: str | None = None,
    timeout: float = 30.0,
    referer: str | None = None,
) -> list[Path]:
    output_dir.mkdir(parents=True, exist_ok=True)
    base_name = Path(output_name).stem if output_name else timestamp_output_stem()
    saved_paths: list[Path] = []
    multiple = len(candidates) > 1
    for index, candidate in enumerate(candidates, start=1):
        suffix = image_suffix_from_url(candidate.url)
        filename = f"{base_name}_{index:02d}{suffix}" if multiple else f"{base_name}{suffix}"
        output_path = output_dir / filename
        if output_path.exists() and not overwrite:
            output_path = unique_output_path(output_path)
        saved_paths.append(
            download_image_candidate(
                candidate,
                output_path,
                cookie=cookie,
                timeout=timeout,
                referer=referer,
            )
        )
    return saved_paths


def candidate_metadata(candidate: Candidate) -> dict[str, Any]:
    metadata: dict[str, Any] = {
        "url": candidate.url,
        "source": candidate.source,
        "priority": candidate.priority,
    }
    if candidate.live_photo_duration is not None:
        metadata["live_photo"] = {
            "audio_url": candidate.live_photo_audio_url,
            "duration": candidate.live_photo_duration,
        }
    return metadata


def save_metadata(
    path: Path,
    platform: str,
    item_id: str | None,
    candidates: list[Candidate],
    image_candidates: list[ImageCandidate] | None,
    logs: list[str],
) -> None:
    metadata = {
        "platform": platform,
        "item_id": item_id,
        "created_at": time.strftime("%Y-%m-%dT%H:%M:%S%z"),
        "candidates": [candidate_metadata(candidate) for candidate in candidates],
        "image_candidates": [candidate.__dict__ for candidate in (image_candidates or [])],
        "logs": logs,
    }
    path.write_text(json.dumps(metadata, ensure_ascii=False, indent=2), encoding="utf-8")


def probe_media_info(path: Path) -> dict[str, Any]:
    """Return basic video metadata using ffprobe when available."""
    info: dict[str, Any] = {"file_size": path.stat().st_size}
    ffprobe = shutil.which("ffprobe")
    if not ffprobe:
        info["warning"] = "ffprobe is not installed; only file size is available."
        return info

    command = [
        ffprobe,
        "-v",
        "error",
        "-select_streams",
        "v:0",
        "-show_entries",
        "stream=width,height,codec_name,bit_rate,duration",
        "-show_entries",
        "format=size,duration,bit_rate",
        "-of",
        "json",
        str(path),
    ]
    completed = run_task_subprocess(command, check=False, capture_output=True, text=True)
    if completed.returncode != 0:
        info["warning"] = completed.stderr.strip() or "ffprobe failed."
        return info

    payload = json.loads(completed.stdout)
    stream = (payload.get("streams") or [{}])[0]
    fmt = payload.get("format") or {}
    info.update(
        {
            "codec": stream.get("codec_name"),
            "width": stream.get("width"),
            "height": stream.get("height"),
            "duration": stream.get("duration") or fmt.get("duration"),
            "video_bit_rate": stream.get("bit_rate"),
            "format_bit_rate": fmt.get("bit_rate"),
        }
    )
    return info


def print_media_info(path: Path) -> None:
    info = probe_media_info(path)
    print(f"file: {path}")
    if info.get("width") and info.get("height"):
        print(f"resolution: {info['width']}x{info['height']}")
    if info.get("codec"):
        print(f"codec: {info['codec']}")
    if info.get("duration"):
        print(f"duration: {float(info['duration']):.2f}s")
    if info.get("file_size") is not None:
        print(f"size: {int(info['file_size']) / 1024 / 1024:.2f} MiB ({info['file_size']} bytes)")
    if info.get("format_bit_rate"):
        print(f"bitrate: {int(info['format_bit_rate']) / 1000:.0f} kbps")
    if info.get("warning"):
        print(f"warning: {info['warning']}", file=sys.stderr)


def make_x_compatible_if_needed(path: Path, args: argparse.Namespace) -> Path | None:
    """Create an X-compatible copy when the downloaded video is not upload-friendly."""
    try:
        import x_transcoder
    except ImportError as exc:
        raise DouyinDownloadError("x_transcoder.py is required for --x-compatible.") from exc

    options = x_transcoder.default_options(
        overwrite=args.x_overwrite,
        crf=args.x_crf,
        verbose=args.verbose,
        subprocess_runner=run_task_subprocess,
    )
    info = x_transcoder.probe_media(path)
    compatibility = x_transcoder.check_with_options(info, options)
    if compatibility.ok and not args.x_force:
        print("x_compatible: yes")
        return path

    if compatibility.ok and args.x_force:
        print("x_compatible: yes")
        print("x_force: transcoding anyway")
    else:
        print("x_compatible: no")
        for reason in compatibility.reasons:
            print(f"- {reason}")

    output_dir = args.x_output_dir or None
    output_path = x_transcoder.output_path_for(path, None, output_dir, "_x")
    if output_path.exists() and not args.x_overwrite:
        output_path = unique_output_path(output_path)

    converted = x_transcoder.transcode(options, path, output_path, info)
    output_info = x_transcoder.probe_media(converted)
    output_compatibility = x_transcoder.check_with_options(output_info, options)
    print(f"x_output: {converted}")
    if not output_compatibility.ok:
        print("x_output_compatible: no", file=sys.stderr)
        for reason in output_compatibility.reasons:
            print(f"- {reason}", file=sys.stderr)
    else:
        print("x_output_compatible: yes")
    return converted


def process_x_file(args: argparse.Namespace) -> int:
    try:
        import x_transcoder
    except ImportError as exc:
        raise DouyinDownloadError("x_transcoder.py is required for :x-file.") from exc

    raw_path = getattr(args, "x_file", None)
    if not raw_path:
        raise DouyinDownloadError("No video file was supplied for :x-file.")
    path = resolve_x_file_path(
        str(raw_path),
        getattr(args, "output_dir", None),
    )
    if not path.exists():
        raise DouyinDownloadError(f"X input video does not exist: {path}")
    if not path.is_file():
        raise DouyinDownloadError(f"X input video is not a file: {path}")
    if path.suffix.lower() not in x_transcoder.VIDEO_EXTENSIONS:
        supported = ", ".join(sorted(x_transcoder.VIDEO_EXTENSIONS))
        raise DouyinDownloadError(
            f"Unsupported X input video extension {path.suffix or '<none>'}; "
            f"supported: {supported}"
        )

    task_args = copy.copy(args)
    # Manual file mode fills compatibility gaps just like folder mode. A file
    # that already passes the X checks is never duplicated.
    task_args.x_force = False
    print(f"x_file_start: {path}", flush=True)
    try:
        output = make_x_compatible_if_needed(path, task_args)
    except x_transcoder.TranscodeError as exc:
        raise DouyinDownloadError(f"X file processing failed: {exc}") from exc
    if output == path:
        print(f"x_file_skipped: already_compatible path={path}", flush=True)
    else:
        print(f"x_file_completed: input={path} output={output}", flush=True)
    return 0


def resolve_x_file_path(
    file_text: str,
    output_dir: str | Path | None = None,
) -> Path:
    return resolve_local_file_path(file_text, output_dir)


def resolve_local_file_path(
    file_text: str,
    output_dir: str | Path | None = None,
) -> Path:
    path = Path(file_text).expanduser()
    if path.is_absolute():
        return path.resolve()

    script_path = (SCRIPT_DIRECTORY / path).resolve()
    if script_path.exists() or output_dir is None:
        return script_path

    download_root = Path(output_dir).expanduser()
    if not download_root.is_absolute():
        download_root = SCRIPT_DIRECTORY / download_root
    download_path = (download_root / path).resolve()
    if download_path.exists():
        return download_path
    return script_path


def resolve_ocr_input_path(
    file_text: str,
    output_dir: str | Path | None = None,
) -> Path:
    path = resolve_local_file_path(file_text, output_dir)
    if not path.exists():
        raise DouyinDownloadError(f"OCR input image does not exist: {path}")
    if not path.is_file():
        raise DouyinDownloadError(f"OCR input image is not a file: {path}")
    if path.suffix.lower() not in image_ocr.IMAGE_EXTENSIONS:
        supported = ", ".join(sorted(image_ocr.IMAGE_EXTENSIONS))
        raise DouyinDownloadError(
            f"Unsupported OCR input image extension {path.suffix or '<none>'}; "
            f"supported: {supported}"
        )
    return path


def process_ocr_file(args: argparse.Namespace) -> int:
    raw_path = getattr(args, "ocr_file", None)
    if not raw_path:
        raise DouyinDownloadError("No image file was supplied for OCR.")
    path = resolve_ocr_input_path(
        str(raw_path),
        getattr(args, "output_dir", None),
    )

    print(f"ocr_file_start: {path}", flush=True)
    raise_if_task_cancelled()
    try:
        output_path = image_ocr.ocr_images(
            [path],
            output=args.ocr_output,
            tesseract_bin=args.ocr_bin,
            language=args.ocr_language,
            psm=args.ocr_psm,
            preprocess=args.ocr_preprocess,
            min_line_confidence=args.ocr_min_line_confidence,
            overwrite=args.overwrite,
            verbose=args.verbose,
            print_progress=True,
        )
    except image_ocr.ImageOcrError as exc:
        raise DouyinDownloadError(f"OCR file processing failed: {exc}") from exc
    raise_if_task_cancelled()
    print(f"ocr: {output_path}", flush=True)
    print(
        f"ocr_file_completed: input={path} output={output_path}",
        flush=True,
    )
    return 0


def process_x_folder(args: argparse.Namespace) -> int:
    try:
        import x_transcoder
    except ImportError as exc:
        raise DouyinDownloadError("x_transcoder.py is required for --x-folder.") from exc

    options = x_transcoder.default_options(
        overwrite=args.x_overwrite,
        crf=args.x_crf,
        verbose=args.verbose,
        # Folder mode only fills compatibility gaps; it never duplicates a
        # video that already satisfies the X upload checks.
        force=False,
        check=False,
        output=None,
        output_dir=args.x_output_dir,
        suffix=x_transcoder.DEFAULT_SUFFIX,
        recursive=True,
        subprocess_runner=run_task_subprocess,
    )
    try:
        summary = x_transcoder.process_directory(
            options,
            Path(args.x_folder).expanduser(),
        )
    except x_transcoder.TranscodeError as exc:
        raise DouyinDownloadError(f"X folder processing failed: {exc}") from exc
    return 1 if summary.failed else 0


def audio_text_processing_requested(args: argparse.Namespace) -> bool:
    return bool(args.extract_audio or args.transcribe)


def extract_audio_and_transcribe_if_needed(path: Path, args: argparse.Namespace) -> None:
    if not audio_text_processing_requested(args):
        return
    raise_if_task_cancelled()
    cancel_token = current_cancellation_token()

    audio_path = video_transcriber.output_path_for(
        path,
        args.audio_output,
        None,
        video_transcriber.DEFAULT_AUDIO_SUFFIX,
        ".wav",
    )
    try:
        reuse_audio = bool(args.transcribe and args.audio_output is None and audio_path.exists())
        saved_audio = video_transcriber.extract_audio(
            path,
            audio_path,
            overwrite=args.overwrite,
            reuse_audio=reuse_audio,
            sample_rate=args.audio_sample_rate,
            channels=args.audio_channels,
            cancel_token=cancel_token,
            verbose=args.verbose,
        )
    except OperationCancelled:
        raise
    except Exception as exc:
        raise DouyinDownloadError(f"Audio extraction failed: {exc}") from exc
    print(f"audio: {saved_audio}")

    if not args.transcribe:
        return

    transcript_path = video_transcriber.output_path_for(
        path,
        args.text_output,
        None,
        video_transcriber.DEFAULT_TRANSCRIPT_SUFFIX,
        ".txt",
    )
    try:
        whisper_bin = (
            video_transcriber.find_whisper_binary(args.whisper_bin)
            if args.transcribe_engine == "whisper"
            else None
        )
        model_path = (
            video_transcriber.find_whisper_model(args.whisper_model)
            if args.transcribe_engine == "whisper"
            else None
        )
        transcript = video_transcriber.transcribe_audio_with_engine(
            saved_audio,
            transcript_path,
            engine=args.transcribe_engine,
            whisper_bin=whisper_bin,
            whisper_model_path=model_path,
            language=args.whisper_language,
            threads=args.whisper_threads,
            translate=args.whisper_translate,
            fast=args.whisper_fast,
            no_gpu=args.whisper_no_gpu,
            no_timestamps=not args.whisper_timestamps,
            print_progress=args.whisper_progress,
            funasr_model=args.funasr_model,
            funasr_device=args.funasr_device,
            funasr_vad_model=args.funasr_vad_model,
            funasr_punc_model=args.funasr_punc_model,
            funasr_batch_size_s=args.funasr_batch_size_s,
            funasr_rich_text=args.funasr_rich_text,
            simplify_chinese=args.simplify_chinese,
            cancel_token=cancel_token,
            overwrite=args.overwrite,
            verbose=args.verbose,
        )
    except OperationCancelled:
        raise
    except Exception as exc:
        raise DouyinDownloadError(f"Audio transcription failed: {exc}") from exc
    print(f"transcript: {transcript}")


def ocr_images_if_needed(paths: list[Path], output_stem: str, args: argparse.Namespace) -> None:
    if not args.ocr_images:
        return
    raise_if_task_cancelled()
    try:
        ocr_path = image_ocr.ocr_images(
            paths,
            output=args.ocr_output,
            output_stem=output_stem,
            tesseract_bin=args.ocr_bin,
            language=args.ocr_language,
            psm=args.ocr_psm,
            preprocess=args.ocr_preprocess,
            min_line_confidence=args.ocr_min_line_confidence,
            overwrite=args.overwrite,
            verbose=args.verbose,
            print_progress=True,
        )
        raise_if_task_cancelled()
    except OperationCancelled:
        raise
    except Exception as exc:
        print(f"warning: Image OCR skipped: {exc}", file=sys.stderr)
        return
    print(f"ocr: {ocr_path}")


def read_share_text(args: argparse.Namespace) -> str:
    if args.input_file:
        return Path(args.input_file).expanduser().read_text(encoding="utf-8")
    if args.share:
        return args.share
    if not sys.stdin.isatty():
        return sys.stdin.read()
    raise DouyinDownloadError("Pass share text as an argument, --input-file, or stdin.")


def media_type_message(platform: str, candidates: list[Candidate], image_candidates: list[ImageCandidate]) -> str:
    if candidates:
        return f"detected_media: video (platform={platform}, candidates={len(candidates)})"
    if image_candidates:
        return f"detected_media: images (platform={platform}, count={len(image_candidates)})"
    return f"detected_media: unknown (platform={platform})"


def handle_downloaded_video(path: Path, args: argparse.Namespace) -> None:
    raise_if_task_cancelled()
    if args.show_info:
        print_media_info(path)
    else:
        print(path)
    extract_audio_and_transcribe_if_needed(path, args)
    raise_if_task_cancelled()
    if args.x_compatible:
        try:
            make_x_compatible_if_needed(path, args)
            raise_if_task_cancelled()
        except OperationCancelled:
            raise
        except Exception as exc:
            raise DouyinDownloadError(f"X-compatible transcode failed: {exc}") from exc


def gather_candidates_for_request_with_retries(
    args: argparse.Namespace,
    share_text: str,
    cookie: str | None,
) -> tuple[str, str | None, list[Candidate], list[ImageCandidate], list[str]]:
    max_attempts = PARSE_RETRY_COUNT + 1
    last_result: tuple[str, str | None, list[Candidate], list[ImageCandidate], list[str]] | None = None
    for attempt in range(1, max_attempts + 1):
        raise_if_task_cancelled()
        print(f"parse_attempt: {attempt}/{max_attempts}", file=sys.stderr)
        try:
            result = gather_candidates_for_request(
                share_text,
                platform=args.platform,
                cookie=cookie,
                timeout=args.timeout,
                browser_fallback=args.browser_fallback,
                browser_timeout=args.browser_timeout,
                chrome_path=args.chrome_path,
                require_audio=audio_text_processing_requested(args),
                use_system_browser_cookies=args.system_browser_cookies,
            )
        except (DouyinDownloadError, OSError) as exc:
            if attempt >= max_attempts:
                raise
            print(f"parse_failed: attempt {attempt}/{max_attempts}: {exc}", file=sys.stderr)
            continue

        platform, _item_id, candidates, image_candidates, _logs = result
        if candidates or image_candidates:
            return result

        last_result = result
        if attempt < max_attempts:
            print(
                f"parse_failed: attempt {attempt}/{max_attempts}: "
                f"no downloadable media found for platform={platform}",
                file=sys.stderr,
            )

    if last_result is None:
        raise DouyinDownloadError("Parsing failed before producing a result.")
    return last_result


def profile_limit_argument(value: str) -> int | str:
    if value.strip().lower() == "all":
        return "all"
    try:
        parsed = int(value)
    except ValueError as exc:
        raise argparse.ArgumentTypeError("must be a positive integer or 'all'") from exc
    if parsed <= 0:
        raise argparse.ArgumentTypeError("must be a positive integer or 'all'")
    return parsed


def nonnegative_float_argument(value: str) -> float:
    try:
        parsed = float(value)
    except ValueError as exc:
        raise argparse.ArgumentTypeError("must be a number") from exc
    if parsed < 0:
        raise argparse.ArgumentTypeError("must be zero or greater")
    return parsed


def parse_args(argv: list[str]) -> argparse.Namespace:
    parser = argparse.ArgumentParser(
        description=(
            "Download accessible Douyin, Kuaishou, Xiaohongshu, TikTok, or YouTube media "
            "from copied share text."
        ),
    )
    parser.add_argument(
        "share",
        nargs="?",
        help="Copied share text or URL.",
    )
    parser.add_argument(
        "--interactive",
        "-I",
        action="store_true",
        help="Start an input loop. Paste share text, press Enter to download, then continue.",
    )
    parser.add_argument(
        "-i",
        "--input-file",
        help="Read share text from a UTF-8 file.",
    )
    parser.add_argument(
        "-o",
        "--output-dir",
        default="downloads",
        help="Directory for downloaded videos. Default: downloads",
    )
    parser.add_argument(
        "--output-name",
        help="Output filename. Default: current local time, e.g. 20260624_153012.mp4",
    )
    parser.add_argument(
        "--cookie",
        help="Optional raw Cookie header or path to a file containing one.",
    )
    system_cookie_group = parser.add_mutually_exclusive_group()
    system_cookie_group.add_argument(
        "--system-browser-cookies",
        dest="system_browser_cookies",
        action="store_true",
        default=True,
        help=(
            "Automatically reuse matching site cookies from the local browser profile "
            "for Douyin items/user pages and Xiaohongshu user pages. Default: enabled"
        ),
    )
    system_cookie_group.add_argument(
        "--no-system-browser-cookies",
        dest="system_browser_cookies",
        action="store_false",
        help="Do not read Douyin/Xiaohongshu cookies from a local browser profile.",
    )
    parser.add_argument(
        "--platform",
        choices=PLATFORM_CHOICES,
        default="auto",
        help="Platform to parse. Default: auto",
    )
    parser.add_argument(
        "--timeout",
        type=float,
        default=20.0,
        help="Network timeout in seconds. Default: 20",
    )
    browser_group = parser.add_mutually_exclusive_group()
    browser_group.add_argument(
        "--browser-fallback",
        dest="browser_fallback",
        action="store_true",
        default=True,
        help="Use local Chrome/Chromium network-log fallback when direct extraction finds no video. Default: enabled",
    )
    browser_group.add_argument(
        "--no-browser-fallback",
        dest="browser_fallback",
        action="store_false",
        help="Disable the local Chrome/Chromium fallback.",
    )
    parser.add_argument(
        "--browser-timeout",
        type=float,
        default=DEFAULT_BROWSER_TIMEOUT,
        help=f"Seconds to let browser fallback load the page. Default: {DEFAULT_BROWSER_TIMEOUT:g}",
    )
    parser.add_argument(
        "--profile-limit",
        type=profile_limit_argument,
        default=DEFAULT_PROFILE_LIMIT,
        help=(
            "Maximum recent posts for a Douyin or Xiaohongshu profile page, "
            "or 'all' for every available post. "
            f"Default: {DEFAULT_PROFILE_LIMIT}"
        ),
    )
    parser.add_argument(
        "--profile-interval",
        type=nonnegative_float_argument,
        default=DEFAULT_PROFILE_INTERVAL,
        help=(
            "Minimum seconds between profile pagination/download requests. "
            f"Default: {DEFAULT_PROFILE_INTERVAL:g}"
        ),
    )
    parser.add_argument(
        "--chrome-path",
        help="Path or executable name for the Chromium-compatible browser used by fallback.",
    )
    parser.add_argument(
        "--yt-dlp-bin",
        help="Path or executable name for yt-dlp used by YouTube downloads.",
    )
    parser.add_argument(
        "--youtube-format",
        default=DEFAULT_YOUTUBE_FORMAT,
        help=f"yt-dlp format selector for YouTube downloads. Default: {DEFAULT_YOUTUBE_FORMAT}",
    )
    parser.add_argument(
        "--print-url",
        action="store_true",
        help="Print the best extracted media URL without downloading.",
    )
    parser.add_argument(
        "--save-meta",
        action="store_true",
        help="Save extraction metadata next to the output file.",
    )
    parser.add_argument(
        "--show-info",
        action="store_true",
        help="Print resolution, duration, codec, bitrate, and file size after download.",
    )
    ocr_group = parser.add_mutually_exclusive_group()
    ocr_group.add_argument(
        "--ocr-images",
        action="store_true",
        default=True,
        help="After downloading image posts, run local OCR and save recognized text to a TXT file. Default: enabled",
    )
    ocr_group.add_argument(
        "--no-ocr-images",
        dest="ocr_images",
        action="store_false",
        help="Disable OCR after downloading image posts.",
    )
    parser.add_argument(
        "--ocr-output",
        help="Output TXT path for --ocr-images. Default: image output stem plus _ocr.txt",
    )
    parser.add_argument(
        "--ocr-language",
        default=image_ocr.DEFAULT_LANGUAGE,
        help=f"Tesseract language list for --ocr-images. Default: {image_ocr.DEFAULT_LANGUAGE}",
    )
    parser.add_argument(
        "--ocr-bin",
        help="Path or executable name for tesseract used by --ocr-images.",
    )
    parser.add_argument(
        "--ocr-psm",
        type=int,
        default=image_ocr.DEFAULT_PSM,
        help=f"Tesseract page segmentation mode for --ocr-images. Default: {image_ocr.DEFAULT_PSM}",
    )
    parser.add_argument(
        "--ocr-min-line-confidence",
        type=float,
        default=image_ocr.DEFAULT_MIN_LINE_CONFIDENCE,
        help=(
            "Drop OCR lines whose weighted confidence is below this value. "
            f"Default: {image_ocr.DEFAULT_MIN_LINE_CONFIDENCE}; set below 0 to disable."
        ),
    )
    ocr_preprocess_group = parser.add_mutually_exclusive_group()
    ocr_preprocess_group.add_argument(
        "--ocr-preprocess",
        action="store_true",
        default=image_ocr.DEFAULT_PREPROCESS,
        help="Try enhanced images as OCR candidates and keep the best confidence result. Default: enabled",
    )
    ocr_preprocess_group.add_argument(
        "--no-ocr-preprocess",
        dest="ocr_preprocess",
        action="store_false",
        help="Disable image enhancement before OCR.",
    )
    parser.add_argument(
        "--extract-audio",
        action="store_true",
        help="After a successful video download, extract a separate WAV audio file.",
    )
    parser.add_argument(
        "--transcribe",
        action="store_true",
        help="After a successful video download, extract audio and transcribe it with a local ASR engine.",
    )
    parser.add_argument(
        "--audio-output",
        help="Output WAV path for --extract-audio or --transcribe. Default: downloaded video stem plus _audio.wav",
    )
    parser.add_argument(
        "--text-output",
        help="Output transcript TXT path for --transcribe. Default: downloaded video stem plus _transcript.txt",
    )
    parser.add_argument(
        "--audio-sample-rate",
        type=int,
        default=video_transcriber.DEFAULT_SAMPLE_RATE,
        help="Sample rate for extracted WAV audio. Default: 16000",
    )
    parser.add_argument(
        "--audio-channels",
        type=int,
        default=video_transcriber.DEFAULT_CHANNELS,
        help="Channel count for extracted WAV audio. Default: 1",
    )
    parser.add_argument(
        "--whisper-bin",
        help="Path or executable name for whisper.cpp whisper-cli used by --transcribe.",
    )
    parser.add_argument(
        "--transcribe-engine",
        choices=video_transcriber.TRANSCRIBE_ENGINES,
        default=video_transcriber.DEFAULT_TRANSCRIBE_ENGINE,
        help=f"Local transcription engine for --transcribe. Default: {video_transcriber.DEFAULT_TRANSCRIBE_ENGINE}",
    )
    parser.add_argument(
        "--whisper-model",
        help="Path to a whisper.cpp ggml model used by --transcribe.",
    )
    parser.add_argument(
        "--whisper-language",
        default=video_transcriber.DEFAULT_LANGUAGE,
        help="Spoken language for whisper.cpp, or auto. Default: auto",
    )
    parser.add_argument(
        "--whisper-threads",
        type=int,
        default=video_transcriber.default_whisper_threads(),
        help=f"Thread count passed to whisper.cpp. Default: auto, capped at {video_transcriber.DEFAULT_MAX_THREADS}",
    )
    parser.add_argument(
        "--whisper-translate",
        action="store_true",
        help="Ask whisper.cpp to translate speech to English.",
    )
    parser.add_argument(
        "--whisper-fast",
        action="store_true",
        help="Use faster greedy whisper.cpp decoding. May reduce transcription quality.",
    )
    parser.add_argument(
        "--whisper-no-gpu",
        action="store_true",
        help="Pass --no-gpu to whisper.cpp.",
    )
    parser.add_argument(
        "--whisper-timestamps",
        action="store_true",
        help="Keep timestamps in whisper.cpp text output.",
    )
    parser.add_argument(
        "--whisper-no-progress",
        dest="whisper_progress",
        action="store_false",
        default=True,
        help="Disable whisper.cpp progress output.",
    )
    simplify_group = parser.add_mutually_exclusive_group()
    simplify_group.add_argument(
        "--simplify-chinese",
        dest="simplify_chinese",
        action="store_true",
        default=video_transcriber.DEFAULT_SIMPLIFY_CHINESE,
        help="Convert transcript text from traditional to simplified Chinese with OpenCC. Default: enabled",
    )
    simplify_group.add_argument(
        "--no-simplify-chinese",
        dest="simplify_chinese",
        action="store_false",
        help="Keep the ASR engine's original Chinese script without OpenCC conversion.",
    )
    parser.add_argument(
        "--funasr-model",
        default=video_transcriber.DEFAULT_FUNASR_MODEL,
        help=f"FunASR model id or local path used by --transcribe-engine funasr. Default: {video_transcriber.DEFAULT_FUNASR_MODEL}",
    )
    parser.add_argument(
        "--funasr-device",
        default=video_transcriber.DEFAULT_FUNASR_DEVICE,
        help=f"FunASR device used by --transcribe-engine funasr. Default: {video_transcriber.DEFAULT_FUNASR_DEVICE}",
    )
    parser.add_argument(
        "--funasr-vad-model",
        default=video_transcriber.DEFAULT_FUNASR_VAD_MODEL,
        help=f"FunASR VAD model, or none/off. Default: {video_transcriber.DEFAULT_FUNASR_VAD_MODEL}",
    )
    parser.add_argument(
        "--funasr-punc-model",
        default=video_transcriber.DEFAULT_FUNASR_PUNC_MODEL,
        help="Optional FunASR punctuation model, or none/off. Default: none",
    )
    parser.add_argument(
        "--funasr-batch-size-s",
        type=int,
        default=video_transcriber.DEFAULT_FUNASR_BATCH_SIZE_S,
        help=f"FunASR batch duration in seconds. Default: {video_transcriber.DEFAULT_FUNASR_BATCH_SIZE_S}",
    )
    parser.add_argument(
        "--funasr-rich-text",
        action="store_true",
        default=video_transcriber.DEFAULT_FUNASR_RICH_TEXT,
        help="Keep SenseVoice rich transcription emoji for emotion and audio events. Default: off",
    )
    parser.add_argument(
        "--x-folder",
        help=(
            "Recursively check every video in this folder and create an X-compatible "
            "_x.mp4 copy only when needed."
        ),
    )
    parser.add_argument(
        "--ocr-file",
        help=(
            "Run OCR on one local image and save an _ocr.txt file. "
            "Relative paths are resolved from the script directory or --output-dir."
        ),
    )
    parser.add_argument(
        "--x-compatible",
        action="store_true",
        default=False,
        help="After download, check and auto-transcode unsupported formats to X-compatible H.264/AAC MP4.",
    )
    parser.add_argument(
        "--no-x-compatible",
        dest="x_compatible",
        action="store_false",
        help=argparse.SUPPRESS,
    )
    parser.add_argument(
        "--x-force",
        action="store_true",
        help="With --x-compatible, transcode even if the downloaded file already looks compatible.",
    )
    parser.add_argument(
        "--x-output-dir",
        help="Directory for X-compatible converted files. Default: same directory as downloaded file.",
    )
    parser.add_argument(
        "--x-overwrite",
        action="store_true",
        help="Overwrite existing X-compatible output file.",
    )
    parser.add_argument(
        "--x-crf",
        type=int,
        default=23,
        help="x264 CRF for --x-compatible conversion. Lower is larger/better. Default: 23",
    )
    parser.add_argument(
        "--overwrite",
        action="store_true",
        help="Overwrite the output file if it already exists.",
    )
    parser.add_argument(
        "-v",
        "--verbose",
        action="store_true",
        help="Print resolver logs, candidate URLs, and ASR command/config details.",
    )
    return parser.parse_args(argv)


def interactive_history_path() -> Path:
    configured = os.environ.get(INTERACTIVE_HISTORY_ENV)
    if configured:
        return Path(configured).expanduser()
    return Path.home() / ".media_downloader_history"


def setup_interactive_history(*, force: bool = False) -> Path | None:
    if readline is None:
        return None
    if not force and not sys.stdin.isatty():
        return None

    history_path = interactive_history_path()
    try:
        history_path.parent.mkdir(parents=True, exist_ok=True)
        if history_path.exists():
            readline.read_history_file(str(history_path))
        readline.set_history_length(INTERACTIVE_HISTORY_LIMIT)
        setup_interactive_completion()
    except OSError as exc:
        print(f"warning: could not load interactive history: {exc}", file=sys.stderr)
        return None
    return history_path


def save_interactive_history(history_path: Path | None) -> None:
    if readline is None or history_path is None:
        return
    try:
        readline.set_history_length(INTERACTIVE_HISTORY_LIMIT)
        readline.write_history_file(str(history_path))
    except OSError as exc:
        print(f"warning: could not save interactive history: {exc}", file=sys.stderr)


def add_interactive_history(text: str) -> None:
    if readline is None:
        return
    entry = text.strip()
    if not entry:
        return
    history_length = readline.get_current_history_length()
    if history_length > 0 and readline.get_history_item(history_length) == entry:
        return
    readline.add_history(entry)


def print_interactive_history(limit: int = 20) -> None:
    if readline is None:
        print("interactive_history: unavailable")
        return
    history_length = readline.get_current_history_length()
    if history_length <= 0:
        print("interactive_history: empty")
        return
    start = max(1, history_length - limit + 1)
    print("interactive_history:")
    for index in range(start, history_length + 1):
        item = readline.get_history_item(index)
        if item:
            print(f"  {index}: {item}")


INTERACTIVE_BASE_COMMANDS = (
    "cancel",
    "help",
    "history",
    "jobs",
    "queue",
    "status",
    "on",
    "off",
    "toggle",
    "set",
    "clear",
    "quit",
    "exit",
    "q",
    "ocr-file",
    "x",
    "x-file",
    "x-folder",
)

INTERACTIVE_BOOL_OPTIONS = {
    "browser-fallback": "browser_fallback",
    "system-browser-cookies": "system_browser_cookies",
    "extract-audio": "extract_audio",
    "audio": "extract_audio",
    "print-url": "print_url",
    "save-meta": "save_meta",
    "show-info": "show_info",
    "ocr-images": "ocr_images",
    "ocr": "ocr_images",
    "ocr-preprocess": "ocr_preprocess",
    "transcribe": "transcribe",
    "stt": "transcribe",
    "simplify-chinese": "simplify_chinese",
    "verbose": "verbose",
    "funasr-rich-text": "funasr_rich_text",
    "whisper-fast": "whisper_fast",
    "whisper-no-gpu": "whisper_no_gpu",
    "whisper-progress": "whisper_progress",
    "whisper-timestamps": "whisper_timestamps",
    "whisper-translate": "whisper_translate",
    "overwrite": "overwrite",
    "x-compatible": "x_compatible",
    "x-force": "x_force",
    "x-overwrite": "x_overwrite",
}

INTERACTIVE_BOOL_DEFAULTS = {
    "browser-fallback": True,
    "system-browser-cookies": True,
    "funasr-rich-text": video_transcriber.DEFAULT_FUNASR_RICH_TEXT,
    "ocr": True,
    "ocr-images": True,
    "ocr-preprocess": image_ocr.DEFAULT_PREPROCESS,
    "simplify-chinese": video_transcriber.DEFAULT_SIMPLIFY_CHINESE,
    "whisper-progress": True,
}

INTERACTIVE_VALUE_OPTIONS = {
    "audio-channels": ("audio_channels", int, video_transcriber.DEFAULT_CHANNELS),
    "audio-output": ("audio_output", str, None),
    "audio-sample-rate": ("audio_sample_rate", int, video_transcriber.DEFAULT_SAMPLE_RATE),
    "browser-timeout": ("browser_timeout", float, DEFAULT_BROWSER_TIMEOUT),
    "chrome-path": ("chrome_path", str, None),
    "cookie": ("cookie", str, None),
    "funasr-batch-size-s": ("funasr_batch_size_s", int, video_transcriber.DEFAULT_FUNASR_BATCH_SIZE_S),
    "funasr-device": ("funasr_device", str, video_transcriber.DEFAULT_FUNASR_DEVICE),
    "funasr-model": ("funasr_model", str, video_transcriber.DEFAULT_FUNASR_MODEL),
    "funasr-punc-model": ("funasr_punc_model", str, video_transcriber.DEFAULT_FUNASR_PUNC_MODEL),
    "funasr-vad-model": ("funasr_vad_model", str, video_transcriber.DEFAULT_FUNASR_VAD_MODEL),
    "output-dir": ("output_dir", str, "downloads"),
    "output-name": ("output_name", str, None),
    "ocr-bin": ("ocr_bin", str, None),
    "ocr-language": ("ocr_language", str, image_ocr.DEFAULT_LANGUAGE),
    "ocr-min-line-confidence": (
        "ocr_min_line_confidence",
        float,
        image_ocr.DEFAULT_MIN_LINE_CONFIDENCE,
    ),
    "ocr-output": ("ocr_output", str, None),
    "ocr-psm": ("ocr_psm", int, image_ocr.DEFAULT_PSM),
    "platform": ("platform", str, "auto"),
    "profile-interval": ("profile_interval", float, DEFAULT_PROFILE_INTERVAL),
    "profile-limit": ("profile_limit", profile_limit_argument, DEFAULT_PROFILE_LIMIT),
    "text-output": ("text_output", str, None),
    "timeout": ("timeout", float, 20.0),
    "transcribe-engine": ("transcribe_engine", str, video_transcriber.DEFAULT_TRANSCRIBE_ENGINE),
    "whisper-bin": ("whisper_bin", str, None),
    "whisper-language": ("whisper_language", str, video_transcriber.DEFAULT_LANGUAGE),
    "whisper-model": ("whisper_model", str, None),
    "whisper-threads": ("whisper_threads", int, video_transcriber.default_whisper_threads()),
    "x-crf": ("x_crf", int, 23),
    "x-output-dir": ("x_output_dir", str, None),
    "yt-dlp-bin": ("yt_dlp_bin", str, None),
    "youtube-format": ("youtube_format", str, DEFAULT_YOUTUBE_FORMAT),
}

INTERACTIVE_STATUS_OPTIONS = [
    "platform",
    "output-dir",
    "output-name",
    "print-url",
    "save-meta",
    "show-info",
    "ocr-images",
    "ocr-output",
    "ocr-language",
    "ocr-bin",
    "ocr-psm",
    "ocr-min-line-confidence",
    "ocr-preprocess",
    "overwrite",
    "browser-fallback",
    "system-browser-cookies",
    "timeout",
    "browser-timeout",
    "profile-interval",
    "profile-limit",
    "extract-audio",
    "yt-dlp-bin",
    "youtube-format",
    "transcribe",
    "audio-output",
    "text-output",
    "transcribe-engine",
    "simplify-chinese",
    "funasr-rich-text",
    "funasr-model",
    "funasr-device",
    "funasr-vad-model",
    "funasr-punc-model",
    "funasr-batch-size-s",
    "whisper-language",
    "whisper-threads",
    "whisper-fast",
    "whisper-no-gpu",
    "whisper-progress",
    "x-compatible",
    "x-force",
    "x-output-dir",
    "verbose",
]

INTERACTIVE_COMMAND_OPTIONS = tuple(
    sorted(set(INTERACTIVE_BASE_COMMANDS) | set(INTERACTIVE_BOOL_OPTIONS) | set(INTERACTIVE_VALUE_OPTIONS))
)
INTERACTIVE_OPTION_NAMES = tuple(sorted(set(INTERACTIVE_BOOL_OPTIONS) | set(INTERACTIVE_VALUE_OPTIONS)))
INTERACTIVE_BOOL_VALUES = ("on", "off", "toggle")
INTERACTIVE_PLATFORM_VALUES = ("auto", "douyin", "kuaishou", "xiaohongshu", "tiktok", "youtube")
INTERACTIVE_TRANSCRIBE_ENGINE_VALUES = video_transcriber.TRANSCRIBE_ENGINES


def interactive_option_key(name: str) -> str:
    return name.strip().lower().replace("_", "-")


def matching_completions(prefix: str, candidates: Iterable[str]) -> list[str]:
    normalized_prefix = interactive_option_key(prefix)
    return [candidate for candidate in candidates if candidate.startswith(normalized_prefix)]


def whitespace_tokens_before_cursor(line: str, endidx: int) -> list[str]:
    before_cursor = line[:endidx]
    if not before_cursor.strip():
        return []
    return before_cursor.split()


def interactive_value_completions(option: str, prefix: str) -> list[str]:
    normalized = interactive_option_key(option)
    if normalized in INTERACTIVE_BOOL_OPTIONS:
        return matching_completions(prefix, INTERACTIVE_BOOL_VALUES)
    if normalized == "platform":
        return matching_completions(prefix, INTERACTIVE_PLATFORM_VALUES)
    if normalized == "transcribe-engine":
        return matching_completions(prefix, INTERACTIVE_TRANSCRIBE_ENGINE_VALUES)
    return []


def interactive_completion_candidates(line: str, begidx: int, endidx: int) -> list[str]:
    stripped = line.lstrip()
    if not stripped.startswith((':', '/')):
        return []

    leading_offset = len(line) - len(stripped)
    command_prefix = stripped[0]
    cursor = max(0, endidx - leading_offset)
    command_line = stripped[1:]
    command_endidx = max(0, cursor - 1)
    tokens = whitespace_tokens_before_cursor(command_line, command_endidx)
    current = command_line[max(0, begidx - leading_offset - 1) : command_endidx]
    current = current.strip()

    if not tokens:
        return [f"{command_prefix}{candidate} " for candidate in INTERACTIVE_COMMAND_OPTIONS]

    first = interactive_option_key(tokens[0])
    completing_first_token = len(tokens) == 1 and not command_line[:command_endidx].endswith((" ", "\t"))
    if completing_first_token:
        return [f"{command_prefix}{candidate} " for candidate in matching_completions(first, INTERACTIVE_COMMAND_OPTIONS)]

    if first in {"on", "off", "toggle", "clear"}:
        return [f"{candidate} " for candidate in matching_completions(current, INTERACTIVE_OPTION_NAMES)]

    if first == "set":
        if len(tokens) <= 2 and not command_line[:command_endidx].endswith((" ", "\t")):
            return [f"{candidate} " for candidate in matching_completions(current, INTERACTIVE_OPTION_NAMES)]
        if len(tokens) == 1 and command_line[:command_endidx].endswith((" ", "\t")):
            return [f"{candidate} " for candidate in INTERACTIVE_OPTION_NAMES]
        option = tokens[1] if len(tokens) >= 2 else ""
        return [f"{candidate} " for candidate in interactive_value_completions(option, current)]

    return [f"{candidate} " for candidate in interactive_value_completions(first, current)]


def make_interactive_completer(readline_module: Any | None = None) -> Any:
    reader = readline_module or readline
    matches: list[str] = []

    def completer(text: str, state: int) -> str | None:
        nonlocal matches
        if reader is None:
            return None
        if state == 0:
            line = reader.get_line_buffer()
            begidx = reader.get_begidx()
            endidx = reader.get_endidx()
            matches = interactive_completion_candidates(line, begidx, endidx)
        if state < len(matches):
            return matches[state]
        return None

    return completer


def setup_interactive_completion() -> None:
    if readline is None:
        return
    try:
        readline.set_completer(make_interactive_completer(readline))
        if hasattr(readline, "set_completer_delims"):
            readline.set_completer_delims(" \t\n")
        readline.parse_and_bind("tab: complete")
    except (OSError, AttributeError) as exc:
        print(f"warning: could not enable interactive completion: {exc}", file=sys.stderr)


def parse_interactive_bool(value: str) -> bool:
    lowered = value.strip().lower()
    if lowered in {"1", "true", "on", "yes", "y", "enable", "enabled"}:
        return True
    if lowered in {"0", "false", "off", "no", "n", "disable", "disabled"}:
        return False
    raise DouyinDownloadError(f"Expected on/off, got: {value}")


def format_interactive_value(args: argparse.Namespace, key: str) -> str:
    normalized = interactive_option_key(key)
    if normalized in INTERACTIVE_BOOL_OPTIONS:
        value = getattr(args, INTERACTIVE_BOOL_OPTIONS[normalized])
        return "on" if value else "off"
    if normalized in INTERACTIVE_VALUE_OPTIONS:
        dest, _converter, _default = INTERACTIVE_VALUE_OPTIONS[normalized]
        value = getattr(args, dest)
        if normalized == "cookie":
            return "<set>" if value else "<empty>"
        return "<empty>" if value is None else str(value)
    raise DouyinDownloadError(f"Unknown interactive option: {key}")


def print_interactive_setting(args: argparse.Namespace, key: str) -> None:
    print(f"{interactive_option_key(key)}: {format_interactive_value(args, key)}")


def print_interactive_status(args: argparse.Namespace) -> None:
    print("interactive_settings:")
    for key in INTERACTIVE_STATUS_OPTIONS:
        print(f"  {key}: {format_interactive_value(args, key)}")


def print_interactive_help() -> None:
    print(
        "\n".join(
            [
                "Interactive commands:",
                "  :help                         show this help",
                "  :status                       show current settings",
                "  :history                      show recent input history",
                "  :queue / :jobs                show background task queue",
                "  :cancel                       cancel running and queued tasks",
                "  :ocr-file <image>             queue OCR for one local image",
                "  :x-file <video>               queue one video for X compatibility processing",
                "  :x-folder <directory>         queue recursive X compatibility processing",
                "  :x <directory>                shortcut for :x-folder",
                "  :on <option>                  turn a boolean option on",
                "  :off <option>                 turn a boolean option off",
                "  :toggle <option>              toggle a boolean option",
                "  :set <option> <value>         set an option value",
                "  :clear <option>               reset an option to its default",
                "  :<option> on|off              shortcut for booleans",
                "  :<option> <value>             shortcut for values",
                "  <profile URL> all             queue all Douyin/Xiaohongshu profile posts",
                "  :quit                         cancel tasks and exit interactive mode",
                "Boolean options:",
                "  "
                + ", ".join(
                    [
                        "browser-fallback",
                        "system-browser-cookies",
                        "extract-audio",
                        "print-url",
                        "save-meta",
                        "show-info",
                        "ocr-images",
                        "ocr-preprocess",
                        "transcribe",
                        "simplify-chinese",
                        "verbose",
                        "funasr-rich-text",
                        "whisper-fast",
                        "whisper-no-gpu",
                        "whisper-progress",
                        "whisper-timestamps",
                        "whisper-translate",
                        "overwrite",
                        "x-compatible",
                        "x-force",
                        "x-overwrite",
                    ]
                ),
                "Value options:",
                "  "
                + ", ".join(
                    [
                        "platform",
                        "output-dir",
                        "output-name",
                        "ocr-output",
                        "ocr-language",
                        "ocr-bin",
                        "ocr-psm",
                        "ocr-min-line-confidence",
                        "timeout",
                        "browser-timeout",
                        "profile-limit",
                        "profile-interval",
                        "chrome-path",
                        "cookie",
                        "yt-dlp-bin",
                        "youtube-format",
                        "audio-output",
                        "text-output",
                        "audio-sample-rate",
                        "audio-channels",
                        "transcribe-engine",
                        "funasr-model",
                        "funasr-device",
                        "funasr-vad-model",
                        "funasr-punc-model",
                        "funasr-batch-size-s",
                        "whisper-bin",
                        "whisper-model",
                        "whisper-language",
                        "whisper-threads",
                        "x-crf",
                        "x-output-dir",
                    ]
                ),
            ]
        )
    )


def set_interactive_option(
    args: argparse.Namespace,
    key: str,
    raw_value: str,
    cookie: str | None,
) -> str | None:
    normalized = interactive_option_key(key)
    if normalized in INTERACTIVE_BOOL_OPTIONS:
        setattr(args, INTERACTIVE_BOOL_OPTIONS[normalized], parse_interactive_bool(raw_value))
        print_interactive_setting(args, normalized)
        return cookie

    if normalized not in INTERACTIVE_VALUE_OPTIONS:
        raise DouyinDownloadError(f"Unknown interactive option: {key}")

    dest, converter, _default = INTERACTIVE_VALUE_OPTIONS[normalized]
    try:
        value = converter(raw_value)
    except (ValueError, argparse.ArgumentTypeError) as exc:
        raise DouyinDownloadError(f"Invalid value for {normalized}: {raw_value}") from exc
    if normalized == "platform":
        value = normalize_platform(str(value))
        if value not in PLATFORMS:
            raise DouyinDownloadError(
                "Invalid platform. Use auto, douyin, kuaishou, xiaohongshu, tiktok, or youtube."
            )
    if normalized == "transcribe-engine":
        value = str(value).lower()
        if value not in video_transcriber.TRANSCRIBE_ENGINES:
            raise DouyinDownloadError(
                f"Invalid transcribe-engine. Use {', '.join(video_transcriber.TRANSCRIBE_ENGINES)}."
            )
    if normalized == "profile-interval" and float(value) < 0:
        raise DouyinDownloadError("profile-interval must be zero or greater.")
    setattr(args, dest, value)
    if normalized == "cookie":
        cookie = normalize_cookie(str(value))
    print_interactive_setting(args, normalized)
    return cookie


def clear_interactive_option(args: argparse.Namespace, key: str, cookie: str | None) -> str | None:
    normalized = interactive_option_key(key)
    if normalized in INTERACTIVE_BOOL_OPTIONS:
        setattr(args, INTERACTIVE_BOOL_OPTIONS[normalized], INTERACTIVE_BOOL_DEFAULTS.get(normalized, False))
        print_interactive_setting(args, normalized)
        return cookie
    if normalized not in INTERACTIVE_VALUE_OPTIONS:
        raise DouyinDownloadError(f"Unknown interactive option: {key}")

    dest, _converter, default = INTERACTIVE_VALUE_OPTIONS[normalized]
    setattr(args, dest, default)
    if normalized == "cookie":
        cookie = None
    print_interactive_setting(args, normalized)
    return cookie


def toggle_interactive_option(args: argparse.Namespace, key: str) -> None:
    normalized = interactive_option_key(key)
    if normalized not in INTERACTIVE_BOOL_OPTIONS:
        raise DouyinDownloadError(f"Option is not a boolean toggle: {key}")
    dest = INTERACTIVE_BOOL_OPTIONS[normalized]
    setattr(args, dest, not getattr(args, dest))
    print_interactive_setting(args, normalized)


def interactive_command_tokens(command_text: str) -> list[str]:
    try:
        return shlex.split(command_text)
    except ValueError as exc:
        raise DouyinDownloadError(f"Invalid interactive command: {exc}") from exc


def handle_interactive_command(
    args: argparse.Namespace,
    raw_text: str,
    cookie: str | None,
    task_queue: InteractiveTaskQueue | None = None,
) -> tuple[bool, str | None]:
    command_text = raw_text[1:].strip()
    if not command_text:
        print_interactive_help()
        return True, cookie

    tokens = interactive_command_tokens(command_text)
    if not tokens:
        print_interactive_help()
        return True, cookie

    command = interactive_option_key(tokens[0])
    if command in {"exit", "quit", "q"}:
        return False, cookie
    if command in {"help", "h", "?"}:
        print_interactive_help()
        return True, cookie
    if command in {"status", "settings"}:
        print_interactive_status(args)
        return True, cookie
    if command in {"queue", "jobs"}:
        if len(tokens) != 1:
            raise DouyinDownloadError("Usage: :queue")
        if task_queue is None:
            print("task_queue: unavailable")
        else:
            task_queue.print_snapshot()
        return True, cookie
    if command == "cancel":
        if len(tokens) != 1:
            raise DouyinDownloadError("Usage: :cancel")
        if task_queue is None:
            print("task_cancel: unavailable")
        else:
            running_id, queued_ids = task_queue.cancel_all()
            if running_id is None and not queued_ids:
                print("task_cancel: no running or queued tasks")
            else:
                running = f"#{running_id}" if running_id is not None else "none"
                queued = ",".join(f"#{task_id}" for task_id in queued_ids) or "none"
                print(f"task_cancel: running={running} queued={queued}")
                task_queue.print_snapshot()
        return True, cookie
    if command in {"x", "x-folder"}:
        if len(tokens) < 2:
            raise DouyinDownloadError(f"Usage: :{command} <directory>")
        if task_queue is None:
            raise DouyinDownloadError("Interactive task queue is unavailable.")
        task_queue.enqueue_x_folder(args, " ".join(tokens[1:]))
        return True, cookie
    if command == "x-file":
        if len(tokens) < 2:
            raise DouyinDownloadError("Usage: :x-file <video>")
        if task_queue is None:
            raise DouyinDownloadError("Interactive task queue is unavailable.")
        task_queue.enqueue_x_file(args, " ".join(tokens[1:]))
        return True, cookie
    if command == "ocr-file":
        if len(tokens) < 2:
            raise DouyinDownloadError("Usage: :ocr-file <image>")
        if task_queue is None:
            raise DouyinDownloadError("Interactive task queue is unavailable.")
        task_queue.enqueue_ocr_file(args, " ".join(tokens[1:]))
        return True, cookie
    if command in {"history", "hist"}:
        limit = 20
        if len(tokens) > 2:
            raise DouyinDownloadError("Usage: :history [limit]")
        if len(tokens) == 2:
            try:
                limit = int(tokens[1])
            except ValueError as exc:
                raise DouyinDownloadError(f"Invalid history limit: {tokens[1]}") from exc
            if limit <= 0:
                raise DouyinDownloadError("History limit must be greater than 0.")
        print_interactive_history(limit)
        return True, cookie
    if command in {"on", "enable"}:
        if len(tokens) != 2:
            raise DouyinDownloadError("Usage: :on <option>")
        cookie = set_interactive_option(args, tokens[1], "on", cookie)
        return True, cookie
    if command in {"off", "disable"}:
        if len(tokens) != 2:
            raise DouyinDownloadError("Usage: :off <option>")
        cookie = set_interactive_option(args, tokens[1], "off", cookie)
        return True, cookie
    if command == "toggle":
        if len(tokens) != 2:
            raise DouyinDownloadError("Usage: :toggle <option>")
        toggle_interactive_option(args, tokens[1])
        return True, cookie
    if command == "set":
        if len(tokens) < 3:
            raise DouyinDownloadError("Usage: :set <option> <value>")
        cookie = set_interactive_option(args, tokens[1], " ".join(tokens[2:]), cookie)
        return True, cookie
    if command == "clear":
        if len(tokens) != 2:
            raise DouyinDownloadError("Usage: :clear <option>")
        cookie = clear_interactive_option(args, tokens[1], cookie)
        return True, cookie

    if command in INTERACTIVE_BOOL_OPTIONS:
        if len(tokens) == 1:
            toggle_interactive_option(args, command)
        elif len(tokens) == 2 and interactive_option_key(tokens[1]) == "toggle":
            toggle_interactive_option(args, command)
        elif len(tokens) == 2:
            cookie = set_interactive_option(args, command, tokens[1], cookie)
        else:
            raise DouyinDownloadError(f"Usage: :{command} [on|off|toggle]")
        return True, cookie

    if command in INTERACTIVE_VALUE_OPTIONS:
        if len(tokens) == 1:
            print_interactive_setting(args, command)
        else:
            cookie = set_interactive_option(args, command, " ".join(tokens[1:]), cookie)
        return True, cookie

    raise DouyinDownloadError(f"Unknown interactive command: {tokens[0]}. Type :help for commands.")


def is_interactive_command(text: str) -> bool:
    return text.startswith((':', '/'))


def sanitize_profile_folder_name(value: str, fallback: str) -> str:
    cleaned = re.sub(r'[\x00-\x1f<>:"/\\|?*]+', "_", value)
    cleaned = re.sub(r"\s+", " ", cleaned).strip(" .")
    if not cleaned or cleaned in {".", ".."}:
        cleaned = fallback
    return cleaned[:80].rstrip(" .") or fallback


def profile_post_media_candidates(
    post: DouyinProfilePost,
) -> tuple[list[Candidate], list[ImageCandidate]]:
    if not post.payload:
        return [], []
    return extract_douyin_item_media_candidates(
        post.payload,
        source="douyin.profile",
    )


def profile_publish_time(create_time: int, item_id: str) -> str:
    if create_time <= 0:
        return f"unknown-date_{item_id}"
    return time.strftime("%Y-%m-%d_%H-%M-%S", time.localtime(create_time))


def profile_item_output_name(
    args: argparse.Namespace,
    username: str,
    create_time: int,
    item_id: str,
) -> str:
    prefix_source = Path(args.output_name).stem if args.output_name else username
    prefix = sanitize_profile_folder_name(prefix_source, "user")
    return f"{prefix}_{profile_publish_time(create_time, item_id)}.mp4"


def load_profile_manifest(
    path: Path,
    profile_id: str,
    username: str,
    *,
    platform: str = "douyin",
) -> dict[str, Any]:
    id_key = "sec_uid" if platform == "douyin" else "user_id"
    if not path.exists():
        return {
            "version": 1,
            "platform": platform,
            id_key: profile_id,
            "username": username,
            "downloaded": {},
        }
    try:
        payload = json.loads(path.read_text(encoding="utf-8"))
    except (OSError, json.JSONDecodeError) as exc:
        raise DouyinDownloadError(f"Could not read profile download manifest {path}: {exc}") from exc
    if not isinstance(payload, dict):
        raise DouyinDownloadError(f"Profile download manifest is not a JSON object: {path}")
    existing_platform = str(payload.get("platform") or "douyin")
    if existing_platform != platform:
        raise DouyinDownloadError(
            f"Profile folder manifest belongs to {existing_platform}, not {platform}: {path.parent}"
        )
    existing_uid = str(payload.get(id_key) or "")
    if existing_uid and existing_uid != profile_id:
        raise DouyinDownloadError(
            f"Profile folder manifest belongs to another account ({existing_uid}): {path.parent}"
        )
    if not isinstance(payload.get("downloaded"), dict):
        payload["downloaded"] = {}
    payload.update(
        {
            "version": 1,
            "platform": platform,
            id_key: profile_id,
            "username": username,
        }
    )
    return payload


def save_profile_manifest(path: Path, manifest: dict[str, Any]) -> None:
    manifest["updated_at"] = time.strftime("%Y-%m-%dT%H:%M:%S%z")
    temporary_path = path.with_name(f"{path.name}.tmp")
    temporary_path.write_text(
        json.dumps(manifest, ensure_ascii=False, indent=2),
        encoding="utf-8",
    )
    temporary_path.replace(path)


def profile_output_files(profile_output_dir: Path, output_name: str) -> list[str]:
    prefix = Path(output_name).stem
    return sorted(
        path.relative_to(profile_output_dir).as_posix()
        for path in profile_output_dir.rglob("*")
        if path.is_file()
        and path.name.startswith(prefix)
        and not path.name.endswith(".part")
        and not path.name.endswith("_ocr.txt")
    )


def profile_media_output_dir(
    profile_output_dir: Path,
    candidates: list[Candidate],
    image_candidates: list[ImageCandidate],
) -> Path:
    folder_name = PROFILE_VIDEO_FOLDER if candidates else PROFILE_IMAGE_FOLDER
    return profile_output_dir / folder_name


def wait_for_profile_interval(seconds: float) -> None:
    if seconds <= 0:
        return
    token = current_cancellation_token()
    if token is not None:
        if token.wait(seconds):
            token.raise_if_cancelled()
        return
    time.sleep(seconds)


def handle_douyin_profile(
    args: argparse.Namespace,
    profile_url: str,
    sec_uid: str,
    cookie: str | None,
) -> int:
    if not args.browser_fallback:
        raise DouyinDownloadError(
            "Douyin /user/ pages require browser collection; remove --no-browser-fallback."
        )
    printed_collection_logs: list[str] = []

    def print_collection_progress(message: str) -> None:
        printed_collection_logs.append(message)
        print(message, file=sys.stderr, flush=True)

    result = gather_douyin_profile_posts(
        profile_url,
        sec_uid,
        limit=args.profile_limit,
        interval=args.profile_interval,
        cookie=cookie,
        timeout=args.browser_timeout,
        chrome_path=args.chrome_path,
        progress=print_collection_progress,
        use_system_browser_cookies=args.system_browser_cookies,
    )
    if args.verbose:
        for line in result.logs:
            if line not in printed_collection_logs:
                print(line, file=sys.stderr, flush=True)
    if not result.posts:
        cookie_hint = (
            " Export a logged-in douyin.com Cookie and pass it with --cookie; "
            "the public profile API returned no post data."
            if not cookie
            else " The supplied Cookie may be expired or the profile may not expose public posts."
        )
        raise DouyinDownloadError(f"No public posts were collected from this Douyin profile.{cookie_hint}")

    folder_name = sanitize_profile_folder_name(result.username, result.sec_uid)
    profile_output_dir = Path(args.output_dir).expanduser() / folder_name
    manifest_path = profile_output_dir / PROFILE_MANIFEST_FILENAME
    manifest: dict[str, Any] | None = None
    downloaded_items: dict[str, Any] = {}
    if not args.print_url:
        profile_output_dir.mkdir(parents=True, exist_ok=True)
        manifest = load_profile_manifest(manifest_path, result.sec_uid, result.username)
        downloaded_items = manifest["downloaded"]
        print(
            f"profile_manifest: path={manifest_path} downloaded={len(downloaded_items)}",
            file=sys.stderr,
            flush=True,
        )
    print(
        f"profile_detected: platform=douyin user={result.username!r} "
        f"posts={len(result.posts)} limit={args.profile_limit} interval={args.profile_interval:g}s "
        f"output_dir={profile_output_dir}",
        file=sys.stderr,
        flush=True,
    )

    succeeded = 0
    failed = 0
    skipped = 0
    for index, post in enumerate(result.posts, start=1):
        raise_if_task_cancelled()
        if not args.print_url and post.item_id in downloaded_items:
            skipped += 1
            print(
                f"profile_item_skipped: {index}/{len(result.posts)} "
                f"aweme_id={post.item_id} already_downloaded",
                file=sys.stderr,
                flush=True,
            )
            continue
        if succeeded + failed > 0 and args.profile_interval > 0:
            print(
                f"profile_throttle: waiting {args.profile_interval:g}s before next item",
                file=sys.stderr,
                flush=True,
            )
            wait_for_profile_interval(args.profile_interval)
        item_url = f"https://www.douyin.com/video/{post.item_id}"
        item_args = copy.deepcopy(args)
        item_args.share = item_url
        published_at = profile_publish_time(post.create_time, post.item_id)
        item_args.output_name = profile_item_output_name(
            args,
            result.username,
            post.create_time,
            post.item_id,
        )
        print(
            f"profile_item: {index}/{len(result.posts)} aweme_id={post.item_id} "
            f"published_at={published_at}",
            file=sys.stderr,
            flush=True,
        )

        candidates, image_candidates = profile_post_media_candidates(post)
        item_logs = [
            f"douyin profile: user={result.username}",
            f"douyin profile: item {index}/{len(result.posts)}",
        ]
        try:
            if not candidates and not image_candidates:
                print(
                    f"profile_item_media: aweme_id={post.item_id} type=unknown resolving_item_page",
                    file=sys.stderr,
                    flush=True,
                )
                (
                    _resolved_platform,
                    _resolved_item_id,
                    candidates,
                    image_candidates,
                    resolved_logs,
                ) = gather_candidates_for_request_with_retries(item_args, item_url, cookie)
                item_logs.extend(resolved_logs)

            media_output_dir = profile_media_output_dir(
                profile_output_dir,
                candidates,
                image_candidates,
            )
            item_args.output_dir = str(media_output_dir)
            if candidates:
                print(
                    f"profile_item_media: aweme_id={post.item_id} type=video "
                    f"candidates={len(candidates)} quality=highest output_dir={media_output_dir}",
                    file=sys.stderr,
                    flush=True,
                )
            elif image_candidates:
                print(
                    f"profile_item_media: aweme_id={post.item_id} type=images "
                    f"count={len(image_candidates)} output_dir={media_output_dir}",
                    file=sys.stderr,
                    flush=True,
                )
            else:
                raise DouyinDownloadError(f"No media candidates were found for profile item {post.item_id}.")

            try:
                handle_resolved_media(
                    item_args,
                    item_url,
                    cookie,
                    "douyin",
                    post.item_id,
                    candidates,
                    image_candidates,
                    item_logs,
                )
            except DouyinDownloadError as exc:
                print(
                    f"profile_item_retry: {post.item_id}: {exc}",
                    file=sys.stderr,
                    flush=True,
                )
                wait_for_profile_interval(max(1.0, args.profile_interval))
                handle_share_text(item_args, item_url, cookie)
        except OperationCancelled:
            raise
        except (DouyinDownloadError, OSError) as exc:
            failed += 1
            print(f"profile_item_failed: {post.item_id}: {exc}", file=sys.stderr, flush=True)
            continue
        succeeded += 1
        files: list[str] = []
        if manifest is not None:
            files = profile_output_files(profile_output_dir, item_args.output_name)
            downloaded_items[post.item_id] = {
                "create_time": post.create_time,
                "published_at": published_at,
                "downloaded_at": time.strftime("%Y-%m-%dT%H:%M:%S%z"),
                "media_type": (
                    "images" if image_candidates and not candidates else "video" if candidates else "unknown"
                ),
                "files": files,
            }
            save_profile_manifest(manifest_path, manifest)
        print(
            f"profile_item_completed: {index}/{len(result.posts)} aweme_id={post.item_id} "
            f"files={','.join(files) if files else '-'}",
            file=sys.stderr,
            flush=True,
        )

    print(
        f"profile_completed: user={result.username!r} succeeded={succeeded} "
        f"skipped={skipped} failed={failed} "
        f"output_dir={profile_output_dir}",
        file=sys.stderr,
        flush=True,
    )
    if succeeded == 0 and skipped == 0:
        raise DouyinDownloadError(f"All {failed} collected profile posts failed.")
    return 0


def xiaohongshu_profile_item_url(post: XiaohongshuProfilePost) -> str:
    query = {"xsec_source": "pc_user"}
    if post.xsec_token:
        query["xsec_token"] = post.xsec_token
    return f"https://www.xiaohongshu.com/explore/{post.item_id}?{urllib.parse.urlencode(query)}"


def handle_xiaohongshu_profile(
    args: argparse.Namespace,
    profile_url: str,
    user_id: str,
    cookie: str | None,
) -> int:
    if not args.browser_fallback:
        raise DouyinDownloadError(
            "Xiaohongshu /user/profile/ pages require browser collection; "
            "remove --no-browser-fallback."
        )
    printed_collection_logs: list[str] = []

    def print_collection_progress(message: str) -> None:
        printed_collection_logs.append(message)
        print(message, file=sys.stderr, flush=True)

    result = gather_xiaohongshu_profile_posts(
        profile_url,
        user_id,
        limit=args.profile_limit,
        interval=args.profile_interval,
        cookie=cookie,
        timeout=args.browser_timeout,
        chrome_path=args.chrome_path,
        progress=print_collection_progress,
        use_system_browser_cookies=args.system_browser_cookies,
    )
    if args.verbose:
        for line in result.logs:
            if line not in printed_collection_logs:
                print(line, file=sys.stderr, flush=True)
    if not result.posts:
        cookie_hint = (
            " Log in to xiaohongshu.com in the system browser or pass a Cookie with --cookie."
            if not cookie
            else " The supplied Cookie may be expired or the profile may not expose public notes."
        )
        raise DouyinDownloadError(
            f"No public notes were collected from this Xiaohongshu profile.{cookie_hint}"
        )

    folder_name = sanitize_profile_folder_name(result.username, result.user_id)
    profile_output_dir = Path(args.output_dir).expanduser() / folder_name
    manifest_path = profile_output_dir / PROFILE_MANIFEST_FILENAME
    manifest: dict[str, Any] | None = None
    downloaded_items: dict[str, Any] = {}
    if not args.print_url:
        profile_output_dir.mkdir(parents=True, exist_ok=True)
        manifest = load_profile_manifest(
            manifest_path,
            result.user_id,
            result.username,
            platform="xiaohongshu",
        )
        downloaded_items = manifest["downloaded"]
        print(
            f"profile_manifest: path={manifest_path} downloaded={len(downloaded_items)}",
            file=sys.stderr,
            flush=True,
        )
    print(
        f"profile_detected: platform=xiaohongshu user={result.username!r} "
        f"posts={len(result.posts)} limit={args.profile_limit} interval={args.profile_interval:g}s "
        f"output_dir={profile_output_dir}",
        file=sys.stderr,
        flush=True,
    )

    succeeded = 0
    failed = 0
    skipped = 0
    for index, post in enumerate(result.posts, start=1):
        raise_if_task_cancelled()
        if not args.print_url and post.item_id in downloaded_items:
            skipped += 1
            print(
                f"profile_item_skipped: {index}/{len(result.posts)} "
                f"note_id={post.item_id} already_downloaded",
                file=sys.stderr,
                flush=True,
            )
            continue
        if succeeded + failed > 0 and args.profile_interval > 0:
            print(
                f"profile_throttle: waiting {args.profile_interval:g}s before next item",
                file=sys.stderr,
                flush=True,
            )
            wait_for_profile_interval(args.profile_interval)

        item_url = xiaohongshu_profile_item_url(post)
        item_args = copy.deepcopy(args)
        item_args.share = item_url
        published_at = profile_publish_time(post.create_time, post.item_id)
        item_args.output_name = profile_item_output_name(
            args,
            result.username,
            post.create_time,
            post.item_id,
        )
        print(
            f"profile_item: {index}/{len(result.posts)} note_id={post.item_id} "
            f"published_at={published_at} listed_type={post.note_type}",
            file=sys.stderr,
            flush=True,
        )

        item_logs = [
            f"xiaohongshu profile: user={result.username}",
            f"xiaohongshu profile: item {index}/{len(result.posts)}",
        ]
        candidates: list[Candidate] = []
        image_candidates: list[ImageCandidate] = []
        try:
            (
                _resolved_platform,
                _resolved_item_id,
                candidates,
                image_candidates,
                resolved_logs,
            ) = gather_candidates_for_request_with_retries(item_args, item_url, cookie)
            item_logs.extend(resolved_logs)
            media_output_dir = profile_media_output_dir(
                profile_output_dir,
                candidates,
                image_candidates,
            )
            item_args.output_dir = str(media_output_dir)
            if candidates:
                print(
                    f"profile_item_media: note_id={post.item_id} type=video "
                    f"candidates={len(candidates)} quality=highest output_dir={media_output_dir}",
                    file=sys.stderr,
                    flush=True,
                )
            elif image_candidates:
                print(
                    f"profile_item_media: note_id={post.item_id} type=images "
                    f"count={len(image_candidates)} output_dir={media_output_dir}",
                    file=sys.stderr,
                    flush=True,
                )
            else:
                raise DouyinDownloadError(
                    f"No media candidates were found for Xiaohongshu note {post.item_id}."
                )
            handle_resolved_media(
                item_args,
                item_url,
                cookie,
                "xiaohongshu",
                post.item_id,
                candidates,
                image_candidates,
                item_logs,
            )
        except OperationCancelled:
            raise
        except (DouyinDownloadError, OSError) as exc:
            failed += 1
            print(f"profile_item_failed: {post.item_id}: {exc}", file=sys.stderr, flush=True)
            continue

        succeeded += 1
        files: list[str] = []
        if manifest is not None:
            files = profile_output_files(profile_output_dir, item_args.output_name)
            downloaded_items[post.item_id] = {
                "create_time": post.create_time,
                "published_at": published_at,
                "downloaded_at": time.strftime("%Y-%m-%dT%H:%M:%S%z"),
                "media_type": (
                    "images" if image_candidates and not candidates else "video" if candidates else "unknown"
                ),
                "listed_type": post.note_type,
                "files": files,
            }
            save_profile_manifest(manifest_path, manifest)
        print(
            f"profile_item_completed: {index}/{len(result.posts)} note_id={post.item_id} "
            f"files={','.join(files) if files else '-'}",
            file=sys.stderr,
            flush=True,
        )

    print(
        f"profile_completed: platform=xiaohongshu user={result.username!r} "
        f"succeeded={succeeded} skipped={skipped} failed={failed} "
        f"output_dir={profile_output_dir}",
        file=sys.stderr,
        flush=True,
    )
    if succeeded == 0 and skipped == 0:
        raise DouyinDownloadError(f"All {failed} collected profile notes failed.")
    return 0


def handle_share_text(args: argparse.Namespace, share_text: str, cookie: str | None) -> int:
    if not share_text.strip():
        raise DouyinDownloadError("No share text was provided.")
    raise_if_task_cancelled()

    profile_target = extract_douyin_profile_target(share_text)
    configured_platform = normalize_platform(args.platform)
    if profile_target and configured_platform in {"auto", "douyin"}:
        return handle_douyin_profile(args, profile_target[0], profile_target[1], cookie)
    xiaohongshu_profile_target = extract_xiaohongshu_profile_target(share_text)
    if xiaohongshu_profile_target and configured_platform in {"auto", "xiaohongshu"}:
        return handle_xiaohongshu_profile(
            args,
            xiaohongshu_profile_target[0],
            xiaohongshu_profile_target[1],
            cookie,
        )

    platform, item_id, candidates, image_candidates, logs = gather_candidates_for_request_with_retries(
        args,
        share_text,
        cookie,
    )
    raise_if_task_cancelled()

    return handle_resolved_media(
        args,
        share_text,
        cookie,
        platform,
        item_id,
        candidates,
        image_candidates,
        logs,
    )


def handle_resolved_media(
    args: argparse.Namespace,
    share_text: str,
    cookie: str | None,
    platform: str,
    item_id: str | None,
    candidates: list[Candidate],
    image_candidates: list[ImageCandidate],
    logs: list[str],
) -> int:
    raise_if_task_cancelled()

    if args.verbose:
        print(f"Platform: {platform}", file=sys.stderr)
        for line in logs:
            print(line, file=sys.stderr)
        for index, candidate in enumerate(candidates, start=1):
            print(
                f"Candidate {index}: priority={candidate.priority} source={candidate.source} {candidate.url}",
                file=sys.stderr,
            )
        for index, candidate in enumerate(image_candidates, start=1):
            print(
                f"Image candidate {index}: priority={candidate.priority} source={candidate.source} {candidate.url}",
                file=sys.stderr,
            )

    if not candidates and not image_candidates:
        message = (
            "No downloadable video URL was found. The post may be private, unavailable, "
            "or the platform may require a browser cookie for this page."
        )
        if args.browser_fallback and browser_fallback_was_unavailable(logs):
            message += (
                " Direct extraction still ran, but the optional browser fallback was unavailable "
                "because no Chromium-compatible browser was found. Install Chrome, Chromium, "
                "Edge, Brave, or pass --chrome-path; use --no-browser-fallback to skip this check."
        )
        raise DouyinDownloadError(message)

    print(media_type_message(platform, candidates, image_candidates), file=sys.stderr)

    if args.print_url and audio_text_processing_requested(args):
        raise DouyinDownloadError("--print-url cannot be used with --extract-audio or --transcribe.")

    if args.print_url:
        if platform == "youtube" and candidates:
            print_youtube_media_urls(
                candidates[0],
                yt_dlp_bin=args.yt_dlp_bin,
                format_selector=args.youtube_format,
                cookie=cookie,
                timeout=args.timeout,
                verbose=args.verbose,
            )
            return 0
        if candidates:
            print(candidates[0].url)
        else:
            for candidate in image_candidates:
                print(candidate.url)
        return 0

    if image_candidates and not candidates:
        output_dir = Path(args.output_dir).expanduser()
        image_output_name = args.output_name or timestamp_output_stem()
        saved_paths = download_image_candidates(
            image_candidates,
            output_dir,
            output_name=image_output_name,
            overwrite=args.overwrite,
            cookie=cookie,
            timeout=args.timeout,
            referer=platform_referer(platform),
        )
        if args.save_meta:
            meta_name = f"{Path(image_output_name).stem}.json"
            save_metadata(output_dir / meta_name, platform, item_id, [], image_candidates, logs)
        for path in saved_paths:
            if args.show_info:
                print(f"file: {path}")
                print(f"size: {path.stat().st_size / 1024 / 1024:.2f} MiB ({path.stat().st_size} bytes)")
            else:
                print(path)
        ocr_images_if_needed(saved_paths, Path(image_output_name).stem, args)
        return 0

    if platform == "youtube":
        saved_path = download_youtube_video(
            candidates[0],
            Path(args.output_dir).expanduser(),
            output_name=args.output_name,
            yt_dlp_bin=args.yt_dlp_bin,
            format_selector=args.youtube_format,
            cookie=cookie,
            timeout=args.timeout,
            overwrite=args.overwrite,
            verbose=args.verbose,
        )
        if args.save_meta:
            save_metadata(saved_path.with_suffix(".json"), platform, item_id, candidates, image_candidates, logs)
        handle_downloaded_video(saved_path, args)
        return 0

    output_dir = Path(args.output_dir).expanduser()
    output_dir.mkdir(parents=True, exist_ok=True)
    default_name = timestamp_output_name()
    output_name = args.output_name or default_name
    output_path = output_dir / output_name
    if output_path.suffix.lower() != ".mp4":
        output_path = output_path.with_suffix(".mp4")
    if output_path.exists() and not args.overwrite:
        output_path = unique_output_path(output_path)

    last_error: Exception | None = None
    pending_candidates = list(candidates)
    all_video_candidates = list(candidates)
    known_candidate_urls = {candidate.url for candidate in candidates}
    missing_audio_candidate = False
    browser_audio_retry_attempted = False

    while True:
        if not pending_candidates:
            if missing_audio_candidate and args.browser_fallback and not browser_audio_retry_attempted:
                browser_audio_retry_attempted = True
                print("audio_candidate_retry: gathering browser candidates with audio", file=sys.stderr)
                try:
                    browser_item_id, browser_candidates, _, browser_logs = gather_browser_candidates(
                        share_text,
                        platform=platform,
                        cookie=cookie,
                        timeout=args.browser_timeout,
                        chrome_path=args.chrome_path,
                        require_audio=True,
                    )
                except DouyinDownloadError as exc:
                    last_error = exc
                    browser_candidates = []
                    browser_logs = [f"{platform}: audio candidate retry failed: {exc}"]
                    browser_item_id = None

                item_id = item_id or browser_item_id
                logs.extend(browser_logs)
                if args.verbose:
                    for line in browser_logs:
                        print(line, file=sys.stderr)
                new_candidates = [
                    browser_candidate
                    for browser_candidate in browser_candidates
                    if browser_candidate.url not in known_candidate_urls
                ]
                for browser_candidate in new_candidates:
                    known_candidate_urls.add(browser_candidate.url)
                if new_candidates:
                    all_video_candidates.extend(new_candidates)
                    pending_candidates.extend(new_candidates)
                    missing_audio_candidate = False
                    continue
            break

        candidate = pending_candidates.pop(0)
        try:
            saved_path = download_candidate(
                candidate,
                output_path,
                cookie=cookie,
                timeout=args.timeout,
                referer=platform_referer(platform),
            )
            if candidate.live_photo_duration is not None:
                saved_path = compose_live_photo_video(
                    saved_path,
                    candidate,
                    cookie=cookie,
                    timeout=args.timeout,
                    referer=platform_referer(platform),
                    verbose=args.verbose,
                )
        except DouyinDownloadError as exc:
            last_error = exc
            if args.verbose:
                print(f"Rejected candidate: {exc}", file=sys.stderr)
            continue

        if audio_text_processing_requested(args) and video_transcriber.probe_audio_stream(saved_path) is False:
            missing_audio_candidate = True
            last_error = DouyinDownloadError(
                f"Downloaded candidate contains no audio stream ({candidate.source})"
            )
            print(
                f"audio_candidate_rejected: no audio stream ({candidate.source})",
                file=sys.stderr,
            )
            continue

        if args.save_meta:
            save_metadata(
                saved_path.with_suffix(".json"),
                platform,
                item_id,
                all_video_candidates,
                image_candidates,
                logs,
            )
        handle_downloaded_video(saved_path, args)
        return 0

    raise DouyinDownloadError(f"All candidates failed. Last error: {last_error}")


@dataclass
class InteractiveDownloadTask:
    task_id: int
    share_text: str
    args: argparse.Namespace
    cookie: str | None
    platform: str
    label: str
    kind: str = "download"
    status: str = "queued"
    error: str | None = None
    cancel_token: CancellationToken = field(default_factory=CancellationToken, repr=False)


def interactive_task_identity(share_text: str, args: argparse.Namespace) -> tuple[str, str]:
    configured_platform = normalize_platform(str(args.platform))
    platform = detect_platform(share_text) if configured_platform == "auto" else configured_platform
    urls = extract_urls(share_text)
    label = urls[0] if urls else " ".join(share_text.split())
    if len(label) > 96:
        label = label[:93] + "..."
    return platform or "unknown", label


def interactive_download_request(
    args: argparse.Namespace,
    share_text: str,
) -> tuple[argparse.Namespace, str]:
    """Apply one-line interactive modifiers without changing later queued tasks."""
    task_args = copy.deepcopy(args)
    request_text = share_text.strip()
    parts = request_text.rsplit(maxsplit=1)
    if (
        len(parts) == 2
        and parts[1].lower() == "all"
        and (
            extract_douyin_profile_target(parts[0]) is not None
            or extract_xiaohongshu_profile_target(parts[0]) is not None
        )
    ):
        request_text = parts[0].rstrip()
        task_args.profile_limit = "all"
    return task_args, request_text


class InteractivePromptStream:
    def __init__(self, console: InteractivePromptConsole, stream_name: str) -> None:
        self._console = console
        self._stream_name = stream_name

    def write(self, data: str) -> int:
        return self._console.write(self._stream_name, data)

    def flush(self) -> None:
        self._console.flush(self._stream_name)

    def write_progress(self, line: str) -> None:
        self._console.write_progress(self._stream_name, line)

    def finish_progress(self) -> None:
        self._console.finish_progress(self._stream_name)

    def __getattr__(self, name: str) -> Any:
        return getattr(self._console.original_stream(self._stream_name), name)


class InteractivePromptConsole:
    """Keep readline's active input prompt below output from worker threads."""

    def __init__(self) -> None:
        self._stdout = sys.stdout
        self._stderr = sys.stderr
        self._main_thread = threading.current_thread()
        self._lock = threading.RLock()
        self._buffers: dict[tuple[int, str], str] = {}
        self._active_progress: tuple[int, str] | None = None
        self._progress_above_prompt = False
        self._prompt_active = False
        self._installed = False
        self.enabled = bool(
            readline is not None
            and hasattr(readline, "get_line_buffer")
            and sys.stdin.isatty()
            and self._stdout.isatty()
        )
        self.stdout = InteractivePromptStream(self, "stdout")
        self.stderr = InteractivePromptStream(self, "stderr")

    def original_stream(self, stream_name: str) -> Any:
        return self._stderr if stream_name == "stderr" else self._stdout

    def install(self) -> None:
        if not self.enabled or self._installed:
            return
        sys.stdout = self.stdout
        sys.stderr = self.stderr
        self._installed = True

    def restore(self) -> None:
        if not self._installed:
            return
        with self._lock:
            for (thread_id, stream_name), buffered in list(self._buffers.items()):
                if buffered:
                    self._emit_locked(
                        stream_name,
                        [buffered],
                        background=thread_id != self._main_thread.ident,
                    )
            self._buffers.clear()
            self._finish_active_progress_locked()
            self._stdout.flush()
            self._stderr.flush()
            sys.stdout = self._stdout
            sys.stderr = self._stderr
            self._installed = False

    def input_started(self) -> None:
        with self._lock:
            if self._active_progress is not None and not self._progress_above_prompt:
                output = self.original_stream(self._active_progress[1])
                output.write("\n")
                output.flush()
                self._progress_above_prompt = True
            self._prompt_active = True

    def input_finished(self) -> None:
        with self._lock:
            self._prompt_active = False
            if self._progress_above_prompt:
                self._active_progress = None
                self._progress_above_prompt = False

    def write(self, stream_name: str, data: str) -> int:
        if not data:
            return 0
        if not self.enabled:
            return self.original_stream(stream_name).write(data)

        is_background = threading.current_thread() is not self._main_thread
        with self._lock:
            if not is_background and self._prompt_active:
                return self.original_stream(stream_name).write(data)

            key = (threading.get_ident(), stream_name)
            buffered = self._buffers.get(key, "")
            lines: list[str] = []
            for character in data:
                if character == "\r":
                    if buffered:
                        lines.append(buffered)
                        buffered = ""
                    continue
                if character == "\n":
                    lines.append(buffered)
                    buffered = ""
                    continue
                buffered += character
            self._buffers[key] = buffered
            if lines:
                self._emit_locked(stream_name, lines, background=is_background)
        return len(data)

    def flush(self, stream_name: str) -> None:
        if not self.enabled:
            self.original_stream(stream_name).flush()
            return

        is_background = threading.current_thread() is not self._main_thread
        with self._lock:
            key = (threading.get_ident(), stream_name)
            buffered = self._buffers.get(key, "")
            if buffered and not (not is_background and self._prompt_active):
                self._buffers[key] = ""
                self._emit_locked(stream_name, [buffered], background=is_background)
            self.original_stream(stream_name).flush()

    def write_progress(self, stream_name: str, line: str) -> None:
        """Draw a worker progress line in place while keeping the input prompt below it."""
        if not self.enabled:
            output = self.original_stream(stream_name)
            output.write(f"\r{line}")
            output.flush()
            return

        key = (threading.get_ident(), stream_name)
        with self._lock:
            output = self.original_stream(stream_name)
            if self._active_progress is not None and self._active_progress != key:
                self._finish_active_progress_locked()

            if self._prompt_active and output.isatty():
                self._stdout.write("\r\033[2K")
                self._stdout.flush()
                if self._active_progress == key and self._progress_above_prompt:
                    output.write("\033[1A\r\033[2K")
                output.write(line)
                output.write("\n")
                output.flush()
                self._redraw_prompt_locked()
                self._progress_above_prompt = True
            else:
                output.write(f"\r\033[2K{line}")
                output.flush()
                self._progress_above_prompt = False
            self._active_progress = key

    def finish_progress(self, stream_name: str) -> None:
        if not self.enabled:
            output = self.original_stream(stream_name)
            output.write("\n")
            output.flush()
            return

        key = (threading.get_ident(), stream_name)
        with self._lock:
            if self._active_progress == key:
                self._finish_active_progress_locked()

    def _finish_active_progress_locked(self) -> None:
        if self._active_progress is None:
            return
        if not self._progress_above_prompt:
            output = self.original_stream(self._active_progress[1])
            output.write("\n")
            output.flush()
        self._active_progress = None
        self._progress_above_prompt = False

    def _redraw_prompt_locked(self) -> None:
        if readline is None:
            return
        try:
            current_input = readline.get_line_buffer()
        except (OSError, RuntimeError):
            return
        self._stdout.write(f"{INTERACTIVE_PROMPT}{current_input}")
        self._stdout.flush()

    def _emit_locked(self, stream_name: str, lines: list[str], *, background: bool) -> None:
        self._finish_active_progress_locked()
        output = self.original_stream(stream_name)
        redraw_prompt = background and self._prompt_active and output.isatty()
        if redraw_prompt:
            self._stdout.write("\r\033[2K")
            self._stdout.flush()
        for line in lines:
            output.write(line)
            output.write("\n")
        output.flush()
        if redraw_prompt:
            self._redraw_prompt_locked()


class InteractiveTaskQueue:
    """Run interactive downloads sequentially without blocking the input loop."""

    def __init__(
        self,
        handler: Any | None = None,
        x_folder_handler: Any | None = None,
        x_file_handler: Any | None = None,
        ocr_file_handler: Any | None = None,
    ) -> None:
        self._handler = handler or handle_share_text
        self._x_folder_handler = x_folder_handler or process_x_folder
        self._x_file_handler = x_file_handler or process_x_file
        self._ocr_file_handler = ocr_file_handler or process_ocr_file
        self._pending: queue.Queue[InteractiveDownloadTask | None] = queue.Queue()
        self._tasks: list[InteractiveDownloadTask] = []
        self._lock = threading.RLock()
        self._next_task_id = 1
        self._closed = False
        self._current_task: InteractiveDownloadTask | None = None
        self._worker = threading.Thread(
            target=self._worker_loop,
            name="media-downloader-worker",
            daemon=False,
        )
        self._worker.start()

    def enqueue(
        self,
        args: argparse.Namespace,
        share_text: str,
        cookie: str | None,
    ) -> InteractiveDownloadTask:
        task_args, task_share_text = interactive_download_request(args, share_text)
        with self._lock:
            if self._closed:
                raise DouyinDownloadError("Interactive task queue is already closed.")
            platform, label = interactive_task_identity(task_share_text, task_args)
            task = InteractiveDownloadTask(
                task_id=self._next_task_id,
                share_text=task_share_text,
                args=task_args,
                cookie=cookie,
                platform=platform,
                label=label,
            )
            self._next_task_id += 1
            self._tasks.append(task)

        sys.stderr.write(
            f"task_queued: #{task.task_id} platform={task.platform} {task.label}\n"
            f"{self.format_snapshot()}\n"
        )
        sys.stderr.flush()
        self._pending.put(task)
        return task

    def enqueue_x_folder(
        self,
        args: argparse.Namespace,
        folder_text: str,
    ) -> InteractiveDownloadTask:
        folder = Path(folder_text).expanduser()
        if not folder.exists():
            raise DouyinDownloadError(f"X folder does not exist: {folder}")
        if not folder.is_dir():
            raise DouyinDownloadError(f"X folder is not a directory: {folder}")

        task_args = copy.deepcopy(args)
        task_args.x_folder = str(folder)
        label = str(folder)
        if len(label) > 96:
            label = "..." + label[-93:]
        with self._lock:
            if self._closed:
                raise DouyinDownloadError("Interactive task queue is already closed.")
            task = InteractiveDownloadTask(
                task_id=self._next_task_id,
                share_text=str(folder),
                args=task_args,
                cookie=None,
                platform="x",
                label=label,
                kind="x_folder",
            )
            self._next_task_id += 1
            self._tasks.append(task)

        sys.stderr.write(
            f"task_queued: #{task.task_id} platform={task.platform} {task.label}\n"
            f"{self.format_snapshot()}\n"
        )
        sys.stderr.flush()
        self._pending.put(task)
        return task

    def enqueue_x_file(
        self,
        args: argparse.Namespace,
        file_text: str,
    ) -> InteractiveDownloadTask:
        path = resolve_x_file_path(
            file_text,
            getattr(args, "output_dir", None),
        )
        if not path.exists():
            raise DouyinDownloadError(f"X input video does not exist: {path}")
        if not path.is_file():
            raise DouyinDownloadError(f"X input video is not a file: {path}")

        task_args = copy.deepcopy(args)
        task_args.x_file = str(path)
        task_args.x_force = False
        label = str(path)
        if len(label) > 96:
            label = "..." + label[-93:]
        with self._lock:
            if self._closed:
                raise DouyinDownloadError("Interactive task queue is already closed.")
            task = InteractiveDownloadTask(
                task_id=self._next_task_id,
                share_text=str(path),
                args=task_args,
                cookie=None,
                platform="x",
                label=label,
                kind="x_file",
            )
            self._next_task_id += 1
            self._tasks.append(task)

        sys.stderr.write(
            f"task_queued: #{task.task_id} platform={task.platform} {task.label}\n"
            f"{self.format_snapshot()}\n"
        )
        sys.stderr.flush()
        self._pending.put(task)
        return task

    def enqueue_ocr_file(
        self,
        args: argparse.Namespace,
        file_text: str,
    ) -> InteractiveDownloadTask:
        path = resolve_ocr_input_path(
            file_text,
            getattr(args, "output_dir", None),
        )

        task_args = copy.deepcopy(args)
        task_args.ocr_file = str(path)
        label = str(path)
        if len(label) > 96:
            label = "..." + label[-93:]
        with self._lock:
            if self._closed:
                raise DouyinDownloadError("Interactive task queue is already closed.")
            task = InteractiveDownloadTask(
                task_id=self._next_task_id,
                share_text=str(path),
                args=task_args,
                cookie=None,
                platform="ocr",
                label=label,
                kind="ocr_file",
            )
            self._next_task_id += 1
            self._tasks.append(task)

        sys.stderr.write(
            f"task_queued: #{task.task_id} platform={task.platform} {task.label}\n"
            f"{self.format_snapshot()}\n"
        )
        sys.stderr.flush()
        self._pending.put(task)
        return task

    def snapshot(self) -> list[InteractiveDownloadTask]:
        with self._lock:
            return [copy.copy(task) for task in self._tasks]

    def active_count(self) -> int:
        with self._lock:
            return sum(task.status in {"queued", "running", "cancelling"} for task in self._tasks)

    def format_snapshot(self) -> str:
        tasks = self.snapshot()
        if not tasks:
            return "task_queue: empty"
        lines = ["task_queue:"]
        for task in tasks:
            error = " ".join(task.error.split()) if task.error else ""
            if len(error) > 120:
                error = error[:117] + "..."
            detail = f" error={error}" if error else ""
            if (
                extract_douyin_profile_target(task.share_text) is not None
                or extract_xiaohongshu_profile_target(task.share_text) is not None
            ):
                detail += f" limit={task.args.profile_limit}"
            lines.append(
                f"  #{task.task_id} [{task.status}] {task.platform} {task.label}{detail}"
            )
        return "\n".join(lines)

    def print_snapshot(self, *, file: Any | None = None) -> None:
        output = file if file is not None else sys.stdout
        output.write(self.format_snapshot() + "\n")
        output.flush()

    def wait_for_all(self) -> None:
        self._pending.join()

    def cancel_all(self) -> tuple[int | None, list[int]]:
        """Cancel the running task and mark every waiting task as cancelled."""
        with self._lock:
            running_id: int | None = None
            if self._current_task is not None and self._current_task.status in {"running", "cancelling"}:
                running_id = self._current_task.task_id
                self._current_task.status = "cancelling"
                self._current_task.error = "cancellation requested"
                self._current_task.cancel_token.cancel()

            queued_ids: list[int] = []
            for task in self._tasks:
                if task.status == "queued":
                    task.status = "cancelled"
                    task.error = "cancelled by user"
                    task.cancel_token.cancel()
                    queued_ids.append(task.task_id)
        return running_id, queued_ids

    def shutdown(self, *, wait: bool = True) -> None:
        with self._lock:
            if self._closed:
                return
            self._closed = True
        if wait:
            self.wait_for_all()
        self._pending.put(None)
        self._worker.join()

    def _set_task_result(self, task: InteractiveDownloadTask, status: str, error: str | None = None) -> None:
        with self._lock:
            task.status = status
            task.error = error

    def _print_background_state(self, message: str) -> None:
        sys.stderr.write(f"{message}\n{self.format_snapshot()}\n")
        sys.stderr.flush()

    def _worker_loop(self) -> None:
        while True:
            task = self._pending.get()
            try:
                if task is None:
                    return
                with self._lock:
                    if task.status == "cancelled":
                        continue
                    task.status = "running"
                    self._current_task = task
                self._print_background_state(
                    f"task_started: #{task.task_id} platform={task.platform} {task.label}"
                )
                try:
                    _TASK_CONTEXT.cancel_token = task.cancel_token
                    if task.kind == "x_folder":
                        result = self._x_folder_handler(task.args)
                    elif task.kind == "x_file":
                        result = self._x_file_handler(task.args)
                    elif task.kind == "ocr_file":
                        result = self._ocr_file_handler(task.args)
                    else:
                        result = self._handler(task.args, task.share_text, task.cookie)
                except OperationCancelled:
                    self._set_task_result(task, "cancelled", "cancelled by user")
                    self._print_background_state(f"task_cancelled: #{task.task_id}")
                    continue
                except Exception as exc:
                    self._set_task_result(task, "failed", str(exc))
                    self._print_background_state(f"task_failed: #{task.task_id}: {exc}")
                    continue
                if task.cancel_token.is_cancelled():
                    self._set_task_result(task, "cancelled", "cancelled by user")
                    self._print_background_state(f"task_cancelled: #{task.task_id}")
                    continue
                if result == 0:
                    self._set_task_result(task, "completed")
                    self._print_background_state(f"task_completed: #{task.task_id}")
                else:
                    error = f"handler returned exit code {result}"
                    self._set_task_result(task, "failed", error)
                    self._print_background_state(f"task_failed: #{task.task_id}: {error}")
            finally:
                if task is not None:
                    with self._lock:
                        if self._current_task is task:
                            self._current_task = None
                    if getattr(_TASK_CONTEXT, "cancel_token", None) is task.cancel_token:
                        del _TASK_CONTEXT.cancel_token
                self._pending.task_done()


def interactive_loop(args: argparse.Namespace, cookie: str | None) -> int:
    print("Media Downloader interactive mode")
    print("Paste share text or URL to queue it. One background worker processes tasks in order.")
    print("Append 'all' to a Douyin or Xiaohongshu profile URL to download every available post.")
    print("Use :ocr-file <image> to queue OCR for one local image.")
    print("Use :x-file <video> to queue one local video for X-compatible processing.")
    print("Use :x-folder <directory> or :x <directory> to queue X-compatible folder processing.")
    print("Type :queue to show jobs, :cancel to cancel jobs, :help for commands, or exit/quit/q to cancel and stop.")
    active_cookie = cookie
    history_path = setup_interactive_history()
    prompt_console = InteractivePromptConsole()
    prompt_console.install()
    task_queue: InteractiveTaskQueue | None = None
    try:
        task_queue = InteractiveTaskQueue()
        while True:
            prompt_console.input_started()
            try:
                raw_share_text = input(INTERACTIVE_PROMPT)
            except (EOFError, KeyboardInterrupt):
                print()
                return 0
            finally:
                prompt_console.input_finished()
            if history_path is not None:
                add_interactive_history(raw_share_text)
            share_text = raw_share_text.strip()

            if not share_text:
                continue
            if share_text.lower() in {"exit", "quit", "q"}:
                return 0
            if is_interactive_command(share_text):
                keep_running = True
                try:
                    keep_running, active_cookie = handle_interactive_command(
                        args,
                        share_text,
                        active_cookie,
                        task_queue,
                    )
                except (DouyinDownloadError, OSError) as exc:
                    print(f"error: {exc}", file=sys.stderr)
                if not keep_running:
                    return 0
                print()
                continue

            try:
                task_queue.enqueue(args, share_text, active_cookie)
            except (DouyinDownloadError, OSError) as exc:
                print(f"error: {exc}", file=sys.stderr)
            print()
    finally:
        if task_queue is not None:
            active_tasks = task_queue.active_count()
            if active_tasks:
                print(
                    f"queue_shutdown: cancelling {active_tasks} task(s)",
                    file=sys.stderr,
                    flush=True,
                )
            task_queue.cancel_all()
            task_queue.shutdown(wait=True)
        prompt_console.restore()
        save_interactive_history(history_path)


def should_start_interactive(args: argparse.Namespace) -> bool:
    return args.interactive or (
        not args.x_folder
        and not args.ocr_file
        and not args.share
        and not args.input_file
        and sys.stdin.isatty()
    )


def main(argv: list[str] | None = None) -> int:
    args = parse_args(argv or sys.argv[1:])
    if args.x_folder:
        try:
            return process_x_folder(args)
        except (DouyinDownloadError, OSError) as exc:
            print(f"error: {exc}", file=sys.stderr)
            return 1
    if args.ocr_file:
        try:
            return process_ocr_file(args)
        except (DouyinDownloadError, OSError) as exc:
            print(f"error: {exc}", file=sys.stderr)
            return 1
    cookie = normalize_cookie(args.cookie)
    if should_start_interactive(args):
        return interactive_loop(args, cookie)

    try:
        share_text = read_share_text(args)
        return handle_share_text(args, share_text, cookie)
    except (DouyinDownloadError, OSError) as exc:
        print(f"error: {exc}", file=sys.stderr)
        return 1


if __name__ == "__main__":
    raise SystemExit(main())
