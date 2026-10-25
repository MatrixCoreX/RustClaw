"""Host-granted authentication only; browser/media caches remain task-owned."""
from __future__ import annotations

import contextlib
import fcntl
import json
import math
import os
from pathlib import Path
import re
import stat
import time
import uuid
from typing import Any, Callable, Iterator
from urllib.parse import urlsplit

DIRECTORY_ENV = "APP_BROWSER_AUTH_STORE_DIRECTORY"
SCOPE_ENV = "APP_BROWSER_AUTH_STORE_SCOPE"
MAX_STATE_BYTES = 256 * 1024


class AuthenticationStoreError(RuntimeError):
    error_code = "browser_authentication_unavailable"


class AuthenticationStore:
    def __init__(self, directory: Path, scope_id: str, origin: str, platform: str):
        if not directory.is_absolute() or not re.fullmatch(r"[a-f0-9]{64}", scope_id):
            raise AuthenticationStoreError("browser_authentication_store_invalid")
        if not re.fullmatch(r"[a-z][a-z0-9_]{0,63}", platform):
            raise AuthenticationStoreError("browser_authentication_store_invalid")
        parsed = urlsplit(origin)
        if parsed.scheme != "https" or parsed.netloc != parsed.hostname or parsed.path not in {"", "/"}:
            raise AuthenticationStoreError("browser_authentication_origin_invalid")
        self.directory, self.scope_id, self.origin = directory, scope_id, origin.rstrip("/")
        self.name = f"{platform}.json"
        self._descriptor: int | None = None

    @classmethod
    def from_environment(cls, origin: str, platform: str) -> AuthenticationStore | None:
        directory, scope = os.environ.get(DIRECTORY_ENV, ""), os.environ.get(SCOPE_ENV, "")
        if not directory and not scope:
            return None
        return cls(Path(directory), scope, origin, platform)

    @contextlib.contextmanager
    def lease(self, cancellation_check: Callable[[], None] | None = None) -> Iterator[AuthenticationStore]:
        descriptor = os.open(self.directory, os.O_RDONLY | os.O_DIRECTORY | os.O_NOFOLLOW)
        lock = None
        try:
            info = os.fstat(descriptor)
            if info.st_uid != os.getuid() or stat.S_IMODE(info.st_mode) & 0o077:
                raise AuthenticationStoreError("browser_authentication_store_permissions_invalid")
            lock = os.open(self.name + ".lock", os.O_CREAT | os.O_RDWR | os.O_NOFOLLOW, 0o600, dir_fd=descriptor)
            lock_info = os.fstat(lock)
            if lock_info.st_nlink != 1 or lock_info.st_uid != os.getuid() or stat.S_IMODE(lock_info.st_mode) & 0o077:
                raise AuthenticationStoreError("browser_authentication_store_invalid")
            while True:
                if cancellation_check:
                    cancellation_check()
                try:
                    fcntl.flock(lock, fcntl.LOCK_EX | fcntl.LOCK_NB)
                    break
                except BlockingIOError:
                    time.sleep(0.1)
            self._descriptor = descriptor
            yield self
        finally:
            self._descriptor = None
            if lock is not None:
                fcntl.flock(lock, fcntl.LOCK_UN)
                os.close(lock)
            os.close(descriptor)

    def _read(self) -> dict[str, Any]:
        if self._descriptor is None:
            raise AuthenticationStoreError("browser_authentication_lease_required")
        try:
            fd = os.open(self.name, os.O_RDONLY | os.O_NOFOLLOW, dir_fd=self._descriptor)
        except FileNotFoundError:
            return {"schema_version": 1, "scope_id": self.scope_id, "origin": self.origin, "cookies": [], "status": "unknown"}
        try:
            info = os.fstat(fd)
            if not stat.S_ISREG(info.st_mode) or info.st_nlink != 1 or info.st_uid != os.getuid() or info.st_size > MAX_STATE_BYTES or stat.S_IMODE(info.st_mode) & 0o077:
                raise AuthenticationStoreError("browser_authentication_state_invalid")
            with os.fdopen(fd, "rb", closefd=False) as stream:
                try:
                    state = json.loads(stream.read(MAX_STATE_BYTES + 1))
                except (ValueError, UnicodeError):
                    raise AuthenticationStoreError("browser_authentication_state_invalid") from None
        finally:
            os.close(fd)
        if (not isinstance(state, dict) or state.get("schema_version") != 1 or state.get("scope_id") != self.scope_id
                or state.get("origin") != self.origin or state.get("status") not in {"unknown", "valid", "expired", "reauthentication_required"}
                or not isinstance(state.get("cookies"), list) or len(state["cookies"]) > 128):
            raise AuthenticationStoreError("browser_authentication_state_invalid")
        return state

    def _cookie(self, cookie: Any, *, keep_expired: bool = False) -> dict[str, Any] | None:
        if not isinstance(cookie, dict):
            return None
        host = urlsplit(self.origin).hostname or ""
        domain = str(cookie.get("domain", "")).lstrip(".").lower()
        if not domain or not (host == domain or host.endswith("." + domain)):
            return None
        name, value = cookie.get("name"), cookie.get("value")
        if not isinstance(name, str) or not name or len(name) > 256 or not isinstance(value, str) or len(value) > 16384:
            return None
        expiry = cookie.get("expires", -1)
        if isinstance(expiry, bool) or not isinstance(expiry, (int, float)) or not math.isfinite(expiry):
            return None
        if not keep_expired and 0 <= expiry <= time.time():
            return None
        path = cookie.get("path", "/")
        if not isinstance(path, str) or not path.startswith("/") or len(path) > 4096:
            return None
        result = {"name": name, "value": value, "domain": cookie["domain"], "path": path,
                  "secure": cookie.get("secure") is True, "httpOnly": cookie.get("httpOnly") is True}
        if expiry >= 0:
            result["expires"] = expiry  # Never extend the platform's expiration.
        if cookie.get("sameSite") in {"Strict", "Lax", "None"}:
            result["sameSite"] = cookie["sameSite"]
        return result

    def restore(self, call: Callable[[str, dict[str, Any]], dict[str, Any]]) -> None:
        state = self._read()
        raw = state.get("cookies", [])
        if not isinstance(raw, list) or len(raw) > 128:
            raise AuthenticationStoreError("browser_authentication_state_invalid")
        cookies = [item for value in raw if (item := self._cookie(value)) is not None]
        if cookies:
            call("Network.setCookies", {"cookies": cookies})

    def _write(self, state: dict[str, Any]) -> None:
        if self._descriptor is None:
            raise AuthenticationStoreError("browser_authentication_lease_required")
        data = json.dumps(state, ensure_ascii=False, separators=(",", ":"), allow_nan=False).encode("utf-8")
        if len(data) > MAX_STATE_BYTES:
            raise AuthenticationStoreError("browser_authentication_state_too_large")
        temporary = ".authentication-" + uuid.uuid4().hex
        fd = os.open(temporary, os.O_CREAT | os.O_EXCL | os.O_WRONLY | os.O_NOFOLLOW, 0o600, dir_fd=self._descriptor)
        try:
            with os.fdopen(fd, "wb", closefd=False) as stream:
                stream.write(data); stream.flush(); os.fsync(fd)
            os.replace(temporary, self.name, src_dir_fd=self._descriptor, dst_dir_fd=self._descriptor)
            os.fsync(self._descriptor)
        finally:
            os.close(fd)
            try:
                os.unlink(temporary, dir_fd=self._descriptor)
            except FileNotFoundError:
                pass

    def capture(self, call: Callable[[str, dict[str, Any]], dict[str, Any]]) -> None:
        response = call("Network.getCookies", {"urls": [self.origin + "/"]})
        raw = response.get("cookies")
        if not isinstance(raw, list) or len(raw) > 128:
            raise AuthenticationStoreError("browser_authentication_response_invalid")
        cookies = [item for value in raw if (item := self._cookie(value)) is not None]
        self._write({"schema_version": 1, "scope_id": self.scope_id, "origin": self.origin, "cookies": cookies,
                     "status": "valid", "last_verified_at": int(time.time()), "last_checked_at": int(time.time())})

    def require_login(self) -> None:
        state = self._read()
        self._write({"schema_version": 1, "scope_id": self.scope_id, "origin": self.origin, "cookies": [],
                     "status": "reauthentication_required", "last_verified_at": state.get("last_verified_at"),
                     "last_checked_at": int(time.time())})

    def metadata(self) -> dict[str, Any]:
        state = self._read()
        cookies = [item for value in state.get("cookies", []) if (item := self._cookie(value, keep_expired=True)) is not None]
        expirations = [cookie["expires"] for cookie in cookies if "expires" in cookie]
        status = state.get("status", "unknown")
        if cookies and not any(self._cookie(cookie) is not None for cookie in cookies):
            status = "expired"
        elif status == "valid" and any(expiry <= time.time() for expiry in expirations):
            status = "unknown"  # A remaining session cookie cannot prove the account login.
        return {"schema_version": 1, "status": status, "last_verified_at": state.get("last_verified_at"),
                "last_checked_at": state.get("last_checked_at"), "earliest_cookie_expiry": min(expirations) if expirations else None,
                "has_session_cookies": any("expires" not in cookie for cookie in cookies)}
