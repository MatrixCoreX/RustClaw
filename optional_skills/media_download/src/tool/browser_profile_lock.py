"""Cross-process leases for skill-owned Chromium profiles (Linux and macOS)."""
from __future__ import annotations

from contextlib import contextmanager
from collections.abc import Callable
from functools import wraps
import inspect
from pathlib import Path
import threading
import time

import fcntl

_mutex = threading.Lock()
_locks: dict[str, threading.RLock] = {}
_held = threading.local()


class BrowserProfileBusy(TimeoutError):
    error_code = "browser_profile_busy"


@contextmanager
def profile_lease(profile: Path, *, timeout: float = 660, on_tick=None):
    key = str(profile.expanduser().resolve())
    with _mutex:
        lock = _locks.setdefault(key, threading.RLock())
    deadline = time.monotonic() + timeout
    while not lock.acquire(timeout=0.1):
        if on_tick:
            on_tick()
        if time.monotonic() >= deadline:
            raise BrowserProfileBusy("browser_profile_busy")
    try:
        held = getattr(_held, "profiles", set())
        if key in held:
            yield
            return
        directory = Path(key)
        directory.mkdir(parents=True, exist_ok=True)
        with (directory / ".agent-profile.lock").open("a+b") as handle:
            while True:
                if on_tick:
                    on_tick()
                try:
                    fcntl.flock(handle, fcntl.LOCK_EX | fcntl.LOCK_NB)
                    break
                except BlockingIOError:
                    if time.monotonic() >= deadline:
                        raise BrowserProfileBusy("browser_profile_busy")
                    time.sleep(0.1)
            _held.profiles = held | {key}
            try:
                yield
            finally:
                _held.profiles = held
                fcntl.flock(handle, fcntl.LOCK_UN)
    finally:
        lock.release()


def serialize_profile(argument: str, *, platform_subdir: Callable[[str], str] | None = None, on_tick=None):
    def decorate(function):
        signature = inspect.signature(function)

        @wraps(function)
        def invoke(*args, **kwargs):
            bound = signature.bind(*args, **kwargs)
            bound.apply_defaults()
            values = bound.arguments
            profile = values.get(argument)
            if not profile:
                return function(*args, **kwargs)
            profile = Path(profile)
            if platform_subdir:
                profile /= platform_subdir(values["platform"])
            with profile_lease(profile, on_tick=values.get("on_tick") or on_tick):
                return function(*args, **kwargs)
        return invoke
    return decorate
