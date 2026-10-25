"""Read the short-lived skill login challenge shown by the Pi companion UI."""

from __future__ import annotations

import json
import os
import re
import time
import uuid
from pathlib import Path


MAX_STATE_BYTES = 16 * 1024
MAX_IMAGE_BYTES = 5 * 1024 * 1024
COMPANION_REFRESH_SECONDS = 3.0
RASPBERRY_PI_MODEL_PATH = Path("/proc/device-tree/model")
_PLATFORM_RE = re.compile(r"^[a-z][a-z0-9_]{0,47}$")
_SKILL_RE = re.compile(r"^[a-z][a-z0-9_]{0,63}$")
_IMAGE_RE = re.compile(r"^[a-zA-Z0-9_.-]{1,80}\.png$")
_HANDOFF_RE = re.compile(r"^[a-f0-9-]{16,64}$")


def fit_login_challenge_image_size(source_size, box_size, *, allow_upscale=False):
    """Fit a challenge image into its box while preserving the aspect ratio."""
    source_width, source_height = source_size
    box_width, box_height = box_size
    if min(source_width, source_height, box_width, box_height) <= 0:
        raise ValueError("login challenge image dimensions must be positive")
    scale = min(box_width / source_width, box_height / source_height)
    if not allow_upscale:
        scale = min(1.0, scale)
    return (
        max(1, round(source_width * scale)),
        max(1, round(source_height * scale)),
    )


def _runtime_agent_directory(environment=None):
    env = os.environ if environment is None else environment
    runtime_dir = str(env.get("XDG_RUNTIME_DIR") or "").strip()
    if not runtime_dir and os.name == "posix" and hasattr(os, "geteuid"):
        runtime_dir = f"/run/user/{os.geteuid()}"
    return Path(runtime_dir) / "agent-runtime" if runtime_dir else None


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
    directory = _runtime_agent_directory(environment)
    if directory is None:
        return False
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


def _challenge_state_paths(skill_data_root):
    root = Path(skill_data_root)
    try:
        skill_directories = list(root.iterdir())
    except OSError:
        return []
    paths = []
    for skill_directory in skill_directories:
        try:
            if (
                skill_directory.is_symlink()
                or not skill_directory.is_dir()
                or not _SKILL_RE.fullmatch(skill_directory.name)
            ):
                continue
        except OSError:
            continue
        paths.append(skill_directory / "login_challenge" / "current.json")
    return paths


def _task_challenge_state_paths(task_storage_root):
    """Inspect only the host task namespace and short-lived challenge records."""
    if task_storage_root is None:
        return []
    root = Path(task_storage_root)
    try:
        if root.is_symlink():
            return []
        directories = list(root.iterdir())
    except OSError:
        return []
    paths = []
    for directory in directories:
        try:
            if directory.is_symlink() or not directory.is_dir() or str(uuid.UUID(directory.name)) != directory.name:
                continue
        except (OSError, ValueError):
            continue
        paths.extend(_challenge_state_paths(directory))
    return paths


def _load_challenge_state(state_path, current_time, max_age_seconds):
    challenge_dir = state_path.parent
    try:
        if (
            challenge_dir.is_symlink()
            or state_path.is_symlink()
            or state_path.stat().st_size > MAX_STATE_BYTES
        ):
            return None
        state = json.loads(state_path.read_text(encoding="utf-8"))
    except (OSError, ValueError):
        return None
    if not isinstance(state, dict) or state.get("schema_version") != 1:
        return None
    platform = state.get("platform")
    image_file = state.get("image_file")
    updated = state.get("updated_at_epoch")
    handoff_id = state.get("handoff_id")
    interaction = state.get("interaction")
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
    if interaction is not None and interaction != "browser_window":
        return None
    if interaction == "browser_window" and (
        not isinstance(handoff_id, str) or not _HANDOFF_RE.fullmatch(handoff_id)
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
        "source_skill": state_path.parent.parent.name,
        "platform": platform,
        "image_path": os.fspath(image_path),
        "updated_at_epoch": float(updated),
        "cropped": bool(state.get("cropped")),
        "image_mtime_ns": image_stat.st_mtime_ns,
        "image_size_bytes": size,
        "reason_code": str(state.get("reason_code") or ""),
        "handoff_id": handoff_id if interaction == "browser_window" else None,
        "browser_action": interaction == "browser_window",
        "action_path": (
            os.fspath(challenge_dir / f"action-{handoff_id}.json")
            if interaction == "browser_window"
            else None
        ),
    }


def load_login_challenge(skill_data_root, *, task_storage_root=None, now=None, max_age_seconds=15):
    """Return the newest validated challenge from a skill-private state directory."""
    current_time = time.time() if now is None else float(now)
    challenges = [
        challenge
        for state_path in (_challenge_state_paths(skill_data_root) + _task_challenge_state_paths(task_storage_root))
        if (
            challenge := _load_challenge_state(
                state_path, current_time, max_age_seconds
            )
        )
        is not None
    ]
    if not challenges:
        return None
    return max(challenges, key=lambda item: item["updated_at_epoch"])


def request_login_challenge_browser(challenge, *, now=None):
    """Request that the owning skill reveal its existing verification browser."""
    if not isinstance(challenge, dict) or not challenge.get("browser_action"):
        return False
    handoff_id = challenge.get("handoff_id")
    if not isinstance(handoff_id, str) or not _HANDOFF_RE.fullmatch(handoff_id):
        return False
    action_path_value = challenge.get("action_path")
    if not isinstance(action_path_value, str):
        return False
    action_path = Path(action_path_value)
    directory = action_path.parent
    expected_name = f"action-{handoff_id}.json"
    if (
        not action_path.is_absolute()
        or action_path.name != expected_name
        or directory.name != "login_challenge"
        or directory.is_symlink()
    ):
        return False
    temporary = directory / f".{expected_name}.{os.getpid()}.tmp"
    payload = json.dumps(
        {
            "schema_version": 1,
            "action": "show_browser",
            "handoff_id": handoff_id,
            "requested_at_epoch": time.time() if now is None else float(now),
        },
        separators=(",", ":"),
    ).encode("utf-8")
    try:
        if not directory.is_dir():
            return False
        descriptor = os.open(temporary, os.O_CREAT | os.O_TRUNC | os.O_WRONLY, 0o600)
        with os.fdopen(descriptor, "wb") as output:
            output.write(payload)
        os.replace(temporary, action_path)
        return True
    except OSError:
        try:
            temporary.unlink()
        except OSError:
            pass
        return False
