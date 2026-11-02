"""Public account snapshots and explicit logout of this skill's private profiles."""
from contextlib import ExitStack, contextmanager
import fcntl
import hashlib
import json
import os
from pathlib import Path
import shutil
import sqlite3
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


def cookie_state(profile: Path, platform: str) -> str:
    domain, names = DOMAINS[platform]
    for file in (profile / "Default/Network/Cookies", profile / "Default/Cookies"):
        if not file.is_file() or file.is_symlink():
            continue
        try:
            with sqlite3.connect(file.as_uri() + "?mode=ro", uri=True, timeout=0.2) as database:
                rows = database.execute("SELECT host_key,name,expires_utc,length(value),length(encrypted_value) FROM cookies").fetchall()
            now = int((time.time() + 11644473600) * 1000000)
            found = False
            for host, name, expires, plain, encrypted in rows:
                host = str(host).lstrip(".")
                if not (host == domain or host.endswith("." + domain)) or name not in names or not (plain or encrypted):
                    continue
                found = True
                if not expires or expires > now:
                    return "saved_session"
            return "expired" if found else "unknown"
        except (sqlite3.Error, OSError, ValueError, TypeError):
            return "unknown"
    return "unknown"


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
        with temporary.open("x", encoding="utf-8") as stream:
            json.dump(public_snapshot(snapshot, platform), stream, ensure_ascii=False)
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
        snapshot = public_snapshot(value if isinstance(value, dict) else {}, platform)
        states = [cookie_state(profile, platform) for profile in profile_directories(root, platform)]
        if "saved_session" in states:
            if snapshot["status"] != "verified":
                snapshot["status"] = "saved_session"
            snapshot["session_present"] = True
        elif "expired" in states:
            snapshot.update(status="expired", display_name=None, session_present=False)
        elif snapshot["status"] == "verified":
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
