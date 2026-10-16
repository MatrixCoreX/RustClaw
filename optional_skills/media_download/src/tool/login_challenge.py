"""Publish short-lived interactive-login screenshots for a local companion UI.

Only a rendered screenshot and a small amount of non-secret metadata cross this
boundary. Browser URLs, cookies, tokens, and profile contents stay inside the
skill-owned browser profile.
"""

from __future__ import annotations

import base64
import json
import os
import re
import sys
import time
from pathlib import Path
from typing import Any, Callable


REFRESH_SECONDS = 2.0
MAX_IMAGE_BYTES = 5 * 1024 * 1024
MAX_COMPANION_STATE_BYTES = 4096
COMPANION_MAX_AGE_SECONDS = 10.0
COMPANION_SCAN_TIMEOUT_SECONDS = 180.0
CAPTURE_VIEWPORT_WIDTH = 960
CAPTURE_VIEWPORT_HEIGHT = 720
_PNG_SIGNATURE = b"\x89PNG\r\n\x1a\n"
_PLATFORM_RE = re.compile(r"^[a-z][a-z0-9_]{0,47}$")

_VIEWPORT_SIZE_SCRIPT = "({width: window.innerWidth || 0, height: window.innerHeight || 0})"
_CHALLENGE_EXPIRED_SCRIPT = r"""
(() => {
  const text = (document.body?.innerText || '').replace(/\s+/g, ' ').toLowerCase();
  return /二维码(?:已)?(?:过期|失效)|扫码(?:已)?(?:过期|失效)/.test(text)
    || /(?:qr|scan)\s*(?:code)?\s*(?:has\s*)?expired/.test(text)
    || /refresh\s+(?:the\s+)?qr\s*(?:code)?/.test(text);
})()
"""

_CHALLENGE_RECT_SCRIPT = r"""
(() => {
  const visible = (node, rect) => {
    if (!rect || rect.width < 72 || rect.height < 72) return false;
    const style = getComputedStyle(node);
    return style.display !== 'none' && style.visibility !== 'hidden'
      && Number(style.opacity || 1) > 0;
  };
  const textFor = (node) => {
    const parts = [];
    let current = node;
    for (let depth = 0; current && depth < 4; depth += 1, current = current.parentElement) {
      parts.push(current.id || '', current.className || '', current.getAttribute?.('alt') || '',
        current.getAttribute?.('aria-label') || '', current.getAttribute?.('src') || '',
        current.textContent || '');
    }
    return parts.join(' ').toLowerCase().slice(0, 4000);
  };
  const candidates = Array.from(document.querySelectorAll(
    'canvas, img, svg, [class*="qr" i], [id*="qr" i], [aria-label*="scan" i]'
  ));
  let best = null;
  for (const node of candidates) {
    const rect = node.getBoundingClientRect();
    if (!visible(node, rect)) continue;
    const ratio = rect.width / rect.height;
    if (ratio < 0.58 || ratio > 1.72) continue;
    const tokens = textFor(node);
    const hasQrMarker = /qrcode|qr-code|qr_code|\bqr\b/.test(tokens);
    const hasScanInstruction = /二维码|扫码|scan\s*(the|a)?\s*(code|qr)/.test(tokens);
    if (!hasQrMarker && !hasScanInstruction) continue;
    let score = 0;
    if (hasQrMarker) score += 80;
    if (hasScanInstruction) score += 70;
    if (node.tagName === 'CANVAS' || node.tagName === 'SVG') score += 12;
    score += Math.max(0, 20 - Math.abs(1 - ratio) * 20);
    score += Math.min(30, Math.sqrt(rect.width * rect.height) / 12);
    if (!best || score > best.score) {
      best = {score, x: rect.left + scrollX, y: rect.top + scrollY,
        width: rect.width, height: rect.height};
    }
  }
  if (!best || best.score < 25) return null;
  return best;
})()
"""


def _challenge_dir(profile_dir: Path) -> Path | None:
    profile = Path(profile_dir).expanduser()
    if profile.parent.name != "browser-profile":
        return None
    return profile.parent.parent / "login_challenge"


def _companion_state_path(environment: dict[str, str] | None = None) -> Path | None:
    env = os.environ if environment is None else environment
    runtime_dir = str(env.get("XDG_RUNTIME_DIR") or "").strip()
    if not runtime_dir and sys.platform.startswith("linux"):
        runtime_dir = f"/run/user/{os.geteuid()}"
    if not runtime_dir:
        return None
    return Path(runtime_dir) / "agent-runtime" / "login-challenge-companion.json"


def companion_browser_args(
    environment: dict[str, str] | None = None,
    platform: str | None = None,
    now: float | None = None,
) -> list[str]:
    """Keep login Chromium behind a live same-user companion screen on Linux."""
    host = platform or sys.platform
    if not host.startswith("linux"):
        return []
    state_path = _companion_state_path(environment)
    if state_path is None:
        return []
    try:
        if state_path.is_symlink() or state_path.stat().st_size > MAX_COMPANION_STATE_BYTES:
            return []
        state = json.loads(state_path.read_text(encoding="utf-8"))
    except (OSError, ValueError):
        return []
    updated = state.get("updated_at_epoch") if isinstance(state, dict) else None
    current_time = time.time() if now is None else float(now)
    if (
        not isinstance(state, dict)
        or state.get("schema_version") != 1
        or state.get("status") != "active"
        or state.get("device_class") != "raspberry_pi"
        or isinstance(updated, bool)
        or not isinstance(updated, (int, float))
        or updated > current_time + 5
        or current_time - float(updated) > COMPANION_MAX_AGE_SECONDS
    ):
        return []
    return [
        "--start-minimized",
        "--disable-backgrounding-occluded-windows",
        "--disable-renderer-backgrounding",
    ]


def effective_scan_timeout(
    requested_timeout: float,
    environment: dict[str, str] | None = None,
) -> float:
    timeout = max(5.0, float(requested_timeout))
    if companion_browser_args(environment):
        return min(timeout, COMPANION_SCAN_TIMEOUT_SECONDS)
    return timeout


def challenge_has_expired(
    client: Any,
    wait_result: Callable[[int, float], dict[str, Any]],
    environment: dict[str, str] | None = None,
) -> bool:
    """Detect an expired QR prompt only while the Pi companion flow is active."""
    if not companion_browser_args(environment):
        return False
    try:
        command_id = client.send(
            "Runtime.evaluate",
            {"expression": _CHALLENGE_EXPIRED_SCRIPT, "returnByValue": True},
        )
        return _result_value(wait_result(command_id, 5.0)) is True
    except (OSError, TypeError, ValueError, RuntimeError):
        return False


def _atomic_write(path: Path, content: bytes) -> None:
    temporary = path.with_name(f".{path.name}.{os.getpid()}.tmp")
    descriptor = os.open(temporary, os.O_CREAT | os.O_TRUNC | os.O_WRONLY, 0o600)
    try:
        with os.fdopen(descriptor, "wb") as output:
            output.write(content)
            output.flush()
            os.fsync(output.fileno())
        os.replace(temporary, path)
    finally:
        try:
            temporary.unlink()
        except FileNotFoundError:
            pass


def _result_value(message: dict[str, Any]) -> Any:
    outer = message.get("result")
    inner = outer.get("result") if isinstance(outer, dict) else None
    return inner.get("value") if isinstance(inner, dict) else None


def _screenshot_bytes(message: dict[str, Any]) -> bytes | None:
    result = message.get("result")
    encoded = result.get("data") if isinstance(result, dict) else None
    if not isinstance(encoded, str):
        return None
    try:
        image = base64.b64decode(encoded, validate=True)
    except (ValueError, TypeError):
        return None
    if not image.startswith(_PNG_SIGNATURE) or not (len(_PNG_SIGNATURE) < len(image) <= MAX_IMAGE_BYTES):
        return None
    return image


def publish_login_challenge(
    client: Any,
    wait_result: Callable[[int, float], dict[str, Any]],
    profile_dir: Path,
    platform: str,
) -> bool:
    """Capture the likely QR region (or viewport) and atomically publish it."""
    if not _PLATFORM_RE.fullmatch(platform):
        return False
    destination = _challenge_dir(profile_dir)
    if destination is None:
        return False
    try:
        if companion_browser_args():
            viewport_command_id = client.send(
                "Runtime.evaluate",
                {"expression": _VIEWPORT_SIZE_SCRIPT, "returnByValue": True},
            )
            viewport = _result_value(wait_result(viewport_command_id, 5.0))
            if isinstance(viewport, dict):
                viewport_width = float(viewport.get("width") or 0)
                viewport_height = float(viewport.get("height") or 0)
                if viewport_width < 720 or viewport_height < 600:
                    resize_command_id = client.send(
                        "Emulation.setDeviceMetricsOverride",
                        {
                            "width": CAPTURE_VIEWPORT_WIDTH,
                            "height": CAPTURE_VIEWPORT_HEIGHT,
                            "deviceScaleFactor": 1,
                            "mobile": False,
                            "screenWidth": CAPTURE_VIEWPORT_WIDTH,
                            "screenHeight": CAPTURE_VIEWPORT_HEIGHT,
                        },
                    )
                    wait_result(resize_command_id, 5.0)
        command_id = client.send(
            "Runtime.evaluate",
            {"expression": _CHALLENGE_RECT_SCRIPT, "returnByValue": True},
        )
        rect = _result_value(wait_result(command_id, 5.0))
        if companion_browser_args() and not isinstance(rect, dict):
            return False
        screenshot_params: dict[str, Any] = {
            "format": "png",
            "fromSurface": True,
            "captureBeyondViewport": False,
        }
        cropped = False
        if isinstance(rect, dict):
            width = float(rect.get("width") or 0)
            height = float(rect.get("height") or 0)
            if 72 <= width <= 1400 and 72 <= height <= 1400:
                padding = min(28.0, max(12.0, min(width, height) * 0.08))
                screenshot_params["captureBeyondViewport"] = True
                screenshot_params["clip"] = {
                    "x": max(0.0, float(rect.get("x") or 0) - padding),
                    "y": max(0.0, float(rect.get("y") or 0) - padding),
                    "width": width + padding * 2,
                    "height": height + padding * 2,
                    "scale": 1,
                }
                cropped = True
        command_id = client.send("Page.captureScreenshot", screenshot_params)
        image = _screenshot_bytes(wait_result(command_id, 8.0))
        if image is None:
            return False
        destination.mkdir(parents=True, exist_ok=True, mode=0o700)
        try:
            destination.chmod(0o700)
        except OSError:
            pass
        image_name = "challenge.png"
        _atomic_write(destination / image_name, image)
        metadata = {
            "schema_version": 1,
            "status": "awaiting_user",
            "platform": platform,
            "image_file": image_name,
            "cropped": cropped,
            "updated_at_epoch": time.time(),
        }
        _atomic_write(
            destination / "current.json",
            json.dumps(metadata, ensure_ascii=False, separators=(",", ":")).encode("utf-8"),
        )
        return True
    except (OSError, TypeError, ValueError, RuntimeError):
        return False


def clear_login_challenge(profile_dir: Path, platform: str) -> None:
    destination = _challenge_dir(profile_dir)
    if destination is None:
        return
    state_path = destination / "current.json"
    try:
        state = json.loads(state_path.read_text(encoding="utf-8"))
    except (OSError, ValueError):
        state = None
    if isinstance(state, dict) and state.get("platform") != platform:
        return
    for path in (state_path, destination / "challenge.png"):
        try:
            path.unlink()
        except FileNotFoundError:
            pass
        except OSError:
            continue
