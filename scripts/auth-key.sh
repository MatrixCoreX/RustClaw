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

This script is the local-only key management entrypoint.
It reads the SQLite path from configs/config.toml by default.
EOF
}

if [[ $# -lt 1 ]]; then
  usage
  exit 1
fi

python3 - "$ROOT_DIR" "$CONFIG_PATH" "$@" <<'PY'
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
pi_key_path = root / "pi_app" / ".rustclaw_small_screen_key"
pi_key = ""
try:
    pi_key = pi_key_path.read_text(encoding="utf-8").strip()
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

if cmd == "list":
    rows = conn.execute(
        "SELECT user_key, role, enabled, created_at, COALESCE(last_used_at, '') FROM auth_keys ORDER BY created_at ASC"
    ).fetchall()
    for user_key, role, enabled, created_at, last_used_at in rows:
        label = "\tlabel=pi_app" if pi_key and user_key == pi_key else ""
        print(f"{user_key}\t{role}\t{'enabled' if enabled else 'disabled'}\tcreated={created_at}\tlast_used={last_used_at}{label}")
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
else:
    raise SystemExit(f"unknown command: {cmd}")
PY
