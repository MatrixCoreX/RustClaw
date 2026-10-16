"""Read the short-lived skill login challenge shown by the Pi companion UI."""

from __future__ import annotations

import json
import os
import re
import time
from pathlib import Path


MAX_STATE_BYTES = 16 * 1024
MAX_IMAGE_BYTES = 5 * 1024 * 1024
COMPANION_REFRESH_SECONDS = 3.0
RASPBERRY_PI_MODEL_PATH = Path("/proc/device-tree/model")
_PLATFORM_RE = re.compile(r"^[a-z][a-z0-9_]{0,47}$")
_IMAGE_RE = re.compile(r"^[a-zA-Z0-9_.-]{1,80}\.png$")


def is_raspberry_pi_device(model_path=RASPBERRY_PI_MODEL_PATH):
    try:
        model = Path(model_path).read_bytes().replace(b"\x00", b"").decode(
            "utf-8", errors="replace"
        )
    except OSError:
        return False
    return "raspberry pi" in model.lower()


def publish_login_companion_heartbeat(*, now=None, environment=None, model_path=None):
    """Advertise the same-user companion screen through volatile runtime state."""
    if not is_raspberry_pi_device(
        RASPBERRY_PI_MODEL_PATH if model_path is None else model_path
    ):
        return False
    env = os.environ if environment is None else environment
    runtime_dir = str(env.get("XDG_RUNTIME_DIR") or "").strip()
    if not runtime_dir and os.name == "posix" and hasattr(os, "geteuid"):
        runtime_dir = f"/run/user/{os.geteuid()}"
    if not runtime_dir:
        return False
    directory = Path(runtime_dir) / "agent-runtime"
    state_path = directory / "login-challenge-companion.json"
    temporary = directory / f".login-challenge-companion.{os.getpid()}.tmp"
    payload = json.dumps(
        {
            "schema_version": 1,
            "status": "active",
            "device_class": "raspberry_pi",
            "updated_at_epoch": time.time() if now is None else float(now),
        },
        separators=(",", ":"),
    ).encode("utf-8")
    try:
        directory.mkdir(parents=True, exist_ok=True, mode=0o700)
        descriptor = os.open(temporary, os.O_CREAT | os.O_TRUNC | os.O_WRONLY, 0o600)
        with os.fdopen(descriptor, "wb") as output:
            output.write(payload)
        os.replace(temporary, state_path)
        return True
    except OSError:
        try:
            temporary.unlink()
        except OSError:
            pass
        return False


def load_login_challenge(skill_data_root, *, now=None, max_age_seconds=15):
    """Return validated challenge metadata, or ``None`` when it is stale/unsafe."""
    current_time = time.time() if now is None else float(now)
    challenge_dir = Path(skill_data_root) / "media_download" / "login_challenge"
    state_path = challenge_dir / "current.json"
    try:
        if state_path.is_symlink() or state_path.stat().st_size > MAX_STATE_BYTES:
            return None
        state = json.loads(state_path.read_text(encoding="utf-8"))
    except (OSError, ValueError):
        return None
    if not isinstance(state, dict) or state.get("schema_version") != 1:
        return None
    platform = state.get("platform")
    image_file = state.get("image_file")
    updated = state.get("updated_at_epoch")
    if (
        state.get("status") != "awaiting_user"
        or not isinstance(platform, str)
        or not _PLATFORM_RE.fullmatch(platform)
        or not isinstance(image_file, str)
        or not _IMAGE_RE.fullmatch(image_file)
        or isinstance(updated, bool)
        or not isinstance(updated, (int, float))
        or updated > current_time + 5
        or current_time - updated > max(1, int(max_age_seconds))
    ):
        return None
    image_path = challenge_dir / image_file
    try:
        if image_path.is_symlink():
            return None
        image_stat = image_path.stat()
        size = image_stat.st_size
        if size <= 8 or size > MAX_IMAGE_BYTES:
            return None
        with image_path.open("rb") as source:
            if source.read(8) != b"\x89PNG\r\n\x1a\n":
                return None
    except OSError:
        return None
    return {
        "platform": platform,
        "image_path": os.fspath(image_path),
        "updated_at_epoch": float(updated),
        "cropped": bool(state.get("cropped")),
        "image_mtime_ns": image_stat.st_mtime_ns,
        "image_size_bytes": size,
    }
