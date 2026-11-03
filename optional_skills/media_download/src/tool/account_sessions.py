"""Public account snapshots and explicit logout of this skill's private profiles."""
from contextlib import ExitStack, closing, contextmanager
import fcntl
import hashlib
import json
import os
from pathlib import Path
import shutil
import sqlite3
import stat
import time
import uuid

from browser_profile_lock import profile_lease

DOMAINS = {
    "douyin": ("douyin.com", {"sessionid", "sessionid_ss", "sid_guard"}),
    "xiaohongshu": ("xiaohongshu.com", {"web_session"}),
    "kuaishou": ("kuaishou.com", {"kuaishou.server.web_st", "kuaishou.server.webday7_st"}),
    "toutiao": ("toutiao.com", {"sessionid", "sid_guard"}),
    "weibo": ("weibo.com", {"SUB"}),
    "tiktok": ("tiktok.com", {"sessionid", "sessionid_ss", "sid_tt"}),
    "youtube": ("youtube.com", {"SID", "SAPISID", "__Secure-3PSID"}),
    "bilibili": ("bilibili.com", {"SESSDATA"}),
    "wechat_channels": ("yuanbao.tencent.com", {"hy_token", "access_token"}),
}


class AccountSessionError(RuntimeError):
    def __init__(self, error_code, *, side_effect_applied=False):
        super().__init__(error_code)
        self.error_code = error_code
        self.side_effect_applied = side_effect_applied


@contextmanager
def session_lease(root: Path, *, shared=False):
    """Downloads remain parallel; sign-out must wait for an idle skill session.

    Also covers cookie reuse by a non-browser downloader, before Chrome's own
    profile lock exists. It never serializes downloads against one another.
    """
    directory = root / "accounts"
    if directory.is_symlink():
        raise AccountSessionError("account_storage_invalid")
    directory.mkdir(mode=0o700, parents=True, exist_ok=True)
    descriptor = os.open(directory / ".session-use.lock", os.O_CREAT | os.O_RDWR | os.O_NOFOLLOW, 0o600)
    try:
        try:
            fcntl.flock(descriptor, (fcntl.LOCK_SH if shared else fcntl.LOCK_EX) | fcntl.LOCK_NB)
        except BlockingIOError:
            raise AccountSessionError("account_session_busy") from None
        yield
    finally:
        os.close(descriptor)


def profile_directories(root: Path, platform: str) -> list[Path]:
    if platform not in DOMAINS:
        raise AccountSessionError("platform_unsupported")
    result = []
    for base in (root / "browser-profile", root / "public-browser-profile"):
        if base.is_symlink():
            raise AccountSessionError("account_storage_invalid")
        if not base.is_dir():
            continue
        parents = [base] + [item for item in base.iterdir() if item.name.startswith("macos-") and item.is_dir()]
        for parent in parents:
            profile = parent / platform
            if parent.is_symlink() or profile.is_symlink():
                raise AccountSessionError("account_storage_invalid")
            if profile.is_dir():
                result.append(profile)
    return result


def cookie_evidence(profile: Path, platform: str) -> tuple[str, str | None]:
    """Private credential evidence. Digests never enter a public snapshot."""
    domain, names = DOMAINS[platform]
    expired = False
    credentials = []
    for file in (profile / "Default/Network/Cookies", profile / "Default/Cookies"):
        try:
            for part in (profile / "Default", file.parent, file):
                if stat.S_ISLNK(part.lstat().st_mode):
                    raise AccountSessionError("account_storage_invalid")
            if not file.is_file():
                continue
            with closing(sqlite3.connect(file.as_uri() + "?mode=ro", uri=True, timeout=0.2)) as database:
                database.execute("PRAGMA query_only=ON")
                rows = database.execute(
                    "SELECT host_key,name,expires_utc,"
                    "CASE WHEN length(value)<=16384 THEN value ELSE NULL END,"
                    "CASE WHEN length(encrypted_value)<=16384 THEN encrypted_value ELSE NULL END "
                    f"FROM cookies WHERE name IN ({','.join('?' for _ in names)})", tuple(sorted(names)),
                ).fetchall()
            now = time.time_ns() // 1000 + 11644473600000000
            for host, name, expires, plain, encrypted in rows:
                host = str(host).lstrip(".")
                if not (host == domain or host.endswith("." + domain)) or name not in names or not (plain or encrypted):
                    continue
                if expires and expires <= now:
                    expired = True
                    continue
                evidence = hashlib.sha256()
                evidence.update(json.dumps([str(file.relative_to(profile)), host, name, str(expires), plain],
                                           ensure_ascii=False).encode())
                evidence.update(bytes(encrypted or b""))
                credentials.append(evidence.hexdigest())
        except (sqlite3.Error, OSError, ValueError, TypeError):
            continue
    if credentials:
        binding = hashlib.sha256(json.dumps(sorted(credentials)).encode()).hexdigest()
        return "saved_session", binding
    return ("expired" if expired else "unknown"), None


def cookie_state(profile: Path, platform: str) -> str:
    return cookie_evidence(profile, platform)[0]


def credential_evidence(root: Path, platform: str) -> tuple[list[str], str | None]:
    rows = [(str(profile.relative_to(root)), *cookie_evidence(profile, platform))
            for profile in profile_directories(root, platform)]
    bindings = sorted((profile, binding) for profile, _, binding in rows if binding)
    digest = hashlib.sha256(json.dumps(bindings).encode()).hexdigest() if bindings else None
    return [state for _, state, _ in rows], digest


def write_snapshot(root: Path, snapshot: dict) -> None:
    platform = snapshot["platform"]
    if platform not in DOMAINS:
        raise AccountSessionError("platform_unsupported")
    directory = root / "accounts"
    if directory.is_symlink():
        raise AccountSessionError("account_storage_invalid")
    directory.mkdir(mode=0o700, parents=True, exist_ok=True)
    temporary = directory / (".account-" + uuid.uuid4().hex)
    try:
        stored = public_snapshot(snapshot, platform)
        if stored["status"] == "verified":
            _, binding = credential_evidence(root, platform)
            if binding:
                stored["credential_binding"] = binding
        descriptor = os.open(temporary, os.O_WRONLY | os.O_CREAT | os.O_EXCL | os.O_NOFOLLOW, 0o600)
        with os.fdopen(descriptor, "w", encoding="utf-8") as stream:
            json.dump(stored, stream, ensure_ascii=False)
        temporary.chmod(0o600)
        temporary.replace(directory / (platform + ".json"))
    finally:
        temporary.unlink(missing_ok=True)


def public_snapshot(value: dict, platform: str) -> dict:
    value = value if isinstance(value, dict) else {}
    status = value.get("status")
    status = status if status in {"verified", "saved_session", "expired", "signed_out", "unknown"} else "unknown"
    name = value.get("display_name")
    name = name if status == "verified" and isinstance(name, str) and name.strip() and len(name) <= 256 and not any(ord(c) < 32 or ord(c) == 127 for c in name) else None
    checked = value.get("checked_at")
    return {"schema_version": 1, "platform": platform, "status": status, "display_name": name,
            "checked_at": checked if type(checked) in (int, float) and 0 <= checked < 1e12 else None,
            "session_present": value.get("session_present") if type(value.get("session_present")) is bool else None}


def account_status(root: Path, platforms) -> list[dict]:
    if (root / "accounts").is_symlink():
        raise AccountSessionError("account_storage_invalid")
    result = []
    for platform in platforms:
        file = root / "accounts" / (platform + ".json")
        value = {}
        try:
            if not file.is_symlink() and file.stat().st_size <= 16384:
                value = json.loads(file.read_text())
        except FileNotFoundError:
            pass
        except (OSError, ValueError):
            value = {}
        value = value if isinstance(value, dict) else {}
        snapshot = public_snapshot(value, platform)
        states, binding = credential_evidence(root, platform)
        if "saved_session" in states:
            if not binding or value.get("credential_binding") != binding or snapshot["status"] != "verified":
                snapshot.update(status="saved_session", display_name=None)
            snapshot["session_present"] = True
        elif "expired" in states:
            snapshot.update(status="expired", display_name=None, session_present=False)
        elif snapshot["status"] in {"verified", "saved_session"}:
            snapshot.update(status="unknown", display_name=None, session_present=None)
        result.append(snapshot)
    return result


def logout(root: Path, platform: str, *, authentication=None) -> dict:
    if (root / "accounts").is_symlink():
        raise AccountSessionError("account_storage_invalid")
    with ExitStack() as stack:
        stack.enter_context(session_lease(root))
        profiles = profile_directories(root, platform)
        for profile in profiles:
            if (profile / "SingletonLock").exists() or (profile / "SingletonLock").is_symlink():
                raise AccountSessionError("account_session_busy")
            stack.enter_context(profile_lease(profile, timeout=0))
            if (profile / "SingletonLock").exists() or (profile / "SingletonLock").is_symlink():
                raise AccountSessionError("account_session_busy")
        if authentication is not None:
            stack.enter_context(authentication.lease(wait=False))
        changed = False
        try:
            # Invalidate identity before touching credentials so a partial
            # cleanup cannot leave the old verified name in the current view.
            write_snapshot(root, {"platform": platform, "status": "unknown",
                                  "checked_at": int(time.time()), "session_present": None})
            changed = True
            for profile in profiles:
                for child in profile.iterdir():
                    if child.name in {".agent-profile.lock", "browser-environment.json"}:
                        continue
                    changed = True  # A failed removal can already have cleared some state.
                    if child.is_dir() and not child.is_symlink():
                        shutil.rmtree(child)
                    else:
                        child.unlink()
            if authentication is not None:
                changed = True
                authentication.require_login()
            snapshot = {"schema_version": 1, "platform": platform, "status": "signed_out",
                        "display_name": None, "checked_at": int(time.time()), "session_present": False}
            write_snapshot(root, snapshot)
        except (OSError, RuntimeError) as error:
            raise AccountSessionError("account_operation_failed", side_effect_applied=changed) from error
    return snapshot


def task_use_file(root: Path, request_id: str) -> Path:
    return root / "accounts/tasks" / (hashlib.sha256(request_id.encode()).hexdigest() + ".json")


def read_task_use(root: Path, request_id: str, *, consume=False) -> list[dict]:
    file = task_use_file(root, request_id)
    if file.is_symlink() or (root / "accounts").is_symlink() or file.parent.is_symlink():
        return []
    try:
        if file.stat().st_size > 16384:
            return []
        values = json.loads(file.read_text())
    except (OSError, ValueError):
        return []
    finally:
        if consume:
            try:
                file.unlink(missing_ok=True)
            except OSError:
                pass  # Optional metadata cleanup must not discard media output.
    result = []
    for row in values if isinstance(values, list) else []:
        if isinstance(row, dict) and row.get("platform") in DOMAINS:
            result.append({"schema_version": 1, "platform": row["platform"],
                "mode": "authenticated_browser" if row.get("mode") == "authenticated_browser" else "unknown",
                "account": public_snapshot(row.get("account") or {}, row["platform"])})
    return result
