"""Read the short-lived skill login challenge shown by the Pi companion UI."""

from __future__ import annotations

import json
import os
import re
import time
from pathlib import Path


MAX_STATE_BYTES = 16 * 1024
MAX_IMAGE_BYTES = 5 * 1024 * 1024
_PLATFORM_RE = re.compile(r"^[a-z][a-z0-9_]{0,47}$")
_IMAGE_RE = re.compile(r"^[a-zA-Z0-9_.-]{1,80}\.png$")


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
