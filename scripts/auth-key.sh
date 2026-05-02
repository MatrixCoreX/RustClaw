#!/usr/bin/env bash
set -euo pipefail

ROOT_DIR="$(cd "$(dirname "${BASH_SOURCE[0]}")/.." && pwd)"
CONFIG_PATH="${RUSTCLAW_CONFIG_PATH:-$ROOT_DIR/configs/config.toml}"

usage() {
  cat <<'EOF'
Usage:
  scripts/auth-key.sh list
  scripts/auth-key.sh generate [admin|user]
  scripts/auth-key.sh add <user_key> [admin|user]
  scripts/auth-key.sh disable <user_key>
  scripts/auth-key.sh enable <user_key>

  # Web UI（webd）用户名登录：将用户名/密码绑定到已有 user_key
  scripts/auth-key.sh webd-set <user_key> <username>
  scripts/auth-key.sh webd-list
  scripts/auth-key.sh webd-remove <username>

  webd-set 的密码来源（优先级）：环境变量 WEBD_PASSWORD → 非终端 stdin 一行 → 终端下交互输入。

  webd-set 通过 clawd 管理接口完成（由服务端哈希密码）：
    - 使用 RUSTCLAW_ADMIN_KEY；若未设置，则从库里取第一个 enabled 的 admin user_key。
    - BASE_URL 默认 http://127.0.0.1:8787（clawd 需已启动）。

This script is the local-only key management entrypoint.
It reads the SQLite path from configs/config.toml by default.
EOF
}

if [[ $# -lt 1 ]]; then
  usage
  exit 1
fi

python3 - "$ROOT_DIR" "$CONFIG_PATH" "$@" <<'PY'
import os
import json
import sqlite3
import sys
import secrets
from datetime import datetime, timezone
from pathlib import Path

try:
    import tomllib
except ModuleNotFoundError:
    print("Python 3.11+ is required for tomllib.", file=sys.stderr)
    raise SystemExit(1)

root = Path(sys.argv[1])
config_path = Path(sys.argv[2])
cmd = sys.argv[3]
args = sys.argv[4:]

cfg = tomllib.loads(config_path.read_text(encoding="utf-8"))
db_rel = cfg.get("database", {}).get("sqlite_path", "data/rustclaw.db")
db_path = (root / db_rel).resolve()
db_path.parent.mkdir(parents=True, exist_ok=True)
pi_settings_path = root / "pi_app" / ".rustclaw_small_screen_config.json"
pi_key = ""
try:
    pi_settings = json.loads(pi_settings_path.read_text(encoding="utf-8"))
    if isinstance(pi_settings, dict):
        pi_key = str(pi_settings.get("user_key") or "").strip()
except Exception:
    pi_key = ""

conn = sqlite3.connect(db_path)
conn.execute(
    """
    CREATE TABLE IF NOT EXISTS auth_keys (
        user_key     TEXT PRIMARY KEY,
        role         TEXT NOT NULL CHECK (role IN ('admin', 'user')),
        enabled      INTEGER NOT NULL DEFAULT 1,
        created_at   TEXT NOT NULL,
        last_used_at TEXT
    )
    """
)

now = str(int(datetime.now(timezone.utc).timestamp()))

WEBD_DDL = """
CREATE TABLE IF NOT EXISTS webd_login_accounts (
    id             INTEGER PRIMARY KEY AUTOINCREMENT,
    username       TEXT NOT NULL COLLATE NOCASE UNIQUE,
    password_hash  TEXT NOT NULL,
    user_key       TEXT NOT NULL,
    enabled        INTEGER NOT NULL DEFAULT 1,
    created_at     TEXT NOT NULL,
    updated_at     TEXT NOT NULL
);
CREATE INDEX IF NOT EXISTS idx_webd_login_accounts_user_key ON webd_login_accounts(user_key);
"""


def ensure_webd_schema(c):
    c.executescript(WEBD_DDL)


def read_webd_password():
    raw = os.environ.get("WEBD_PASSWORD")
    if raw is not None and str(raw).strip():
        return str(raw).strip()
    # 非 TTY（部分 IDE/自动化）时 readline 会立刻得到空行，不能当作「已输入」
    if not sys.stdin.isatty():
        line = (sys.stdin.readline() or "").strip()
        if line:
            return line
    try:
        import getpass

        return getpass.getpass("Password for webd login: ")
    except Exception as e:
        raise SystemExit(
            "could not read password; set WEBD_PASSWORD or pipe one line on stdin: " + str(e)
        )


def webd_set_via_http(
    base_url: str, admin_key: str, user_key: str, username: str, password: str
) -> None:
    """POST /v1/admin/webd-accounts. Raise SystemExit on any failure."""
    import json
    import urllib.error
    import urllib.request

    url = base_url.rstrip("/") + "/v1/admin/webd-accounts"
    body = json.dumps(
        {"username": username, "password": password, "user_key": user_key},
        ensure_ascii=False,
    ).encode("utf-8")
    req = urllib.request.Request(
        url,
        data=body,
        method="POST",
        headers={
            "Content-Type": "application/json",
            "X-RustClaw-Key": admin_key,
        },
    )
    try:
        with urllib.request.urlopen(req, timeout=60) as resp:
            raw = resp.read().decode("utf-8")
    except urllib.error.HTTPError as e:
        raw = e.read().decode("utf-8", errors="replace")
        raise SystemExit(f"HTTP {e.code}: {raw}") from None
    except OSError as e:
        raise SystemExit(
            f"request failed: {e}. Ensure clawd is running and BASE_URL is reachable."
        ) from None
    try:
        data = json.loads(raw)
    except json.JSONDecodeError:
        raise SystemExit(f"bad JSON: {raw!r}")
    if not data.get("ok"):
        raise SystemExit(data.get("error") or raw)

if cmd == "list":
    webd_usernames_by_key = {}
    try:
        has_webd = conn.execute(
            "SELECT 1 FROM sqlite_master WHERE type='table' AND name='webd_login_accounts' LIMIT 1"
        ).fetchone() is not None
        if has_webd:
            rows_webd = conn.execute(
                """
                SELECT user_key, GROUP_CONCAT(username, ',')
                FROM webd_login_accounts
                WHERE enabled = 1
                GROUP BY user_key
                """
            ).fetchall()
            for uk, names in rows_webd:
                if uk and names:
                    webd_usernames_by_key[str(uk)] = str(names)
    except Exception:
        webd_usernames_by_key = {}

    rows = conn.execute(
        "SELECT user_key, role, enabled, created_at, COALESCE(last_used_at, '') FROM auth_keys ORDER BY created_at ASC"
    ).fetchall()
    for user_key, role, enabled, created_at, last_used_at in rows:
        label = "\tlabel=pi_app" if pi_key and user_key == pi_key else ""
        webd_names = webd_usernames_by_key.get(str(user_key), "")
        webd_col = f"\twebd_usernames={webd_names}" if webd_names else ""
        print(
            f"{user_key}\t{role}\t{'enabled' if enabled else 'disabled'}\tcreated={created_at}\tlast_used={last_used_at}{webd_col}{label}"
        )
elif cmd == "generate":
    role = (args[0].strip().lower() if len(args) > 0 else "user")
    if role not in {"admin", "user"}:
        raise SystemExit("role must be admin or user")
    user_key = "rk-" + secrets.token_urlsafe(18)
    conn.execute(
        """
        INSERT INTO auth_keys (user_key, role, enabled, created_at, last_used_at)
        VALUES (?, ?, 1, ?, NULL)
        """,
        (user_key, role, now),
    )
    conn.commit()
    print(f"{user_key}\t{role}\tenabled\tcreated={now}\tlast_used=")
elif cmd == "add":
    if len(args) < 1:
        raise SystemExit("missing <user_key>")
    user_key = args[0].strip()
    role = (args[1].strip().lower() if len(args) > 1 else "user")
    if role not in {"admin", "user"}:
        raise SystemExit("role must be admin or user")
    conn.execute(
        """
        INSERT INTO auth_keys (user_key, role, enabled, created_at, last_used_at)
        VALUES (?, ?, 1, ?, NULL)
        ON CONFLICT(user_key) DO UPDATE SET role=excluded.role, enabled=1
        """,
        (user_key, role, now),
    )
    conn.commit()
    print(f"added {user_key} ({role})")
elif cmd in {"disable", "enable"}:
    if len(args) < 1:
        raise SystemExit("missing <user_key>")
    user_key = args[0].strip()
    enabled = 1 if cmd == "enable" else 0
    cur = conn.execute("UPDATE auth_keys SET enabled = ? WHERE user_key = ?", (enabled, user_key))
    conn.commit()
    if cur.rowcount == 0:
        raise SystemExit(f"user_key not found: {user_key}")
    print(f"{cmd}d {user_key}")
elif cmd == "webd-set":
    if len(args) < 2:
        raise SystemExit("usage: webd-set <user_key> <username>")
    user_key = args[0].strip()
    username = args[1].strip().lower()
    if not user_key or not username:
        raise SystemExit("user_key and username required")
    pw = read_webd_password()
    if not pw:
        raise SystemExit(
            "empty password — use interactive terminal, or: WEBD_PASSWORD=... ./scripts/auth-key.sh webd-set ..."
        )
    row_target = conn.execute(
        "SELECT 1 FROM auth_keys WHERE user_key = ? AND enabled = 1 LIMIT 1",
        (user_key,),
    ).fetchone()
    if not row_target:
        raise SystemExit(f"user_key not found or disabled in auth_keys: {user_key}")

    admin_key = (os.environ.get("RUSTCLAW_ADMIN_KEY") or "").strip()
    if not admin_key:
        row_adm = conn.execute(
            "SELECT user_key FROM auth_keys WHERE role = 'admin' AND enabled = 1 ORDER BY created_at ASC LIMIT 1"
        ).fetchone()
        if row_adm:
            admin_key = str(row_adm[0]).strip()

    if not admin_key:
        raise SystemExit(
            "no enabled admin key found. Set RUSTCLAW_ADMIN_KEY or create/enable an admin key first."
        )

    base_url = (os.environ.get("BASE_URL") or "http://127.0.0.1:8787").strip()
    if not os.environ.get("RUSTCLAW_ADMIN_KEY"):
        print(
            "note: using first enabled admin user_key from DB for HTTP (set RUSTCLAW_ADMIN_KEY to override)",
            file=sys.stderr,
        )
    webd_set_via_http(base_url, admin_key, user_key, username, pw)
    print(f"webd login set: username={username} -> user_key={user_key}")
elif cmd == "webd-list":
    ensure_webd_schema(conn)
    rows = conn.execute(
        """
        SELECT w.username, w.user_key, w.enabled, w.created_at, w.updated_at
        FROM webd_login_accounts w
        ORDER BY w.username ASC
        """
    ).fetchall()
    for username, uk, enabled, created_at, updated_at in rows:
        st = "enabled" if enabled else "disabled"
        print(f"{username}\t{uk}\t{st}\tcreated={created_at}\tupdated={updated_at}")
elif cmd == "webd-remove":
    if len(args) < 1:
        raise SystemExit("usage: webd-remove <username>")
    username = args[0].strip().lower()
    if not username:
        raise SystemExit("username required")
    ensure_webd_schema(conn)
    cur = conn.execute("DELETE FROM webd_login_accounts WHERE username = ?", (username,))
    conn.commit()
    if cur.rowcount == 0:
        raise SystemExit(f"webd username not found: {username}")
    print(f"removed webd login: {username}")
else:
    raise SystemExit(f"unknown command: {cmd}")
PY
