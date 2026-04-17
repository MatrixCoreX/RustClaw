#!/usr/bin/env bash
set -euo pipefail

ROOT_DIR="$(cd "$(dirname "${BASH_SOURCE[0]}")/.." && pwd)"
CONFIG_PATH="${RUSTCLAW_CONFIG_PATH:-$ROOT_DIR/configs/config.toml}"
CRYPTO_CONFIG_PATH="${RUSTCLAW_CRYPTO_CONFIG_PATH:-$ROOT_DIR/configs/crypto.toml}"

usage() {
  cat <<'EOF'
Usage:
  scripts/import-crypto-credentials.sh [--user-key <rk-...>] [--role admin] [--scrub-config]

Options:
  --user-key <rk-...>   Import credentials into the specified user_key.
  --role <admin|user>   Auto-pick exactly one enabled key by role. Default: admin.
  --scrub-config        After import, remove exchange secret fields from
                        configs/crypto.toml and set enabled=false.

Notes:
  - This script imports Binance / OKX credentials from configs/crypto.toml
    into the SQLite table exchange_api_credentials.
  - It does NOT delete configs/crypto.toml, because that file still contains
    non-secret crypto behavior settings.
  - If multiple enabled keys exist for the selected role, pass --user-key explicitly.
EOF
}

python3 - "$ROOT_DIR" "$CONFIG_PATH" "$CRYPTO_CONFIG_PATH" "$@" <<'PY'
import sqlite3
import sys
from datetime import datetime, timezone
from pathlib import Path

try:
    import tomllib
except ModuleNotFoundError:
    print("Python 3.11+ is required for tomllib.", file=sys.stderr)
    raise SystemExit(1)

root = Path(sys.argv[1])
config_path = Path(sys.argv[2])
crypto_config_path = Path(sys.argv[3])
args = sys.argv[4:]

user_key = ""
role = "admin"
scrub_config = False
i = 0
while i < len(args):
    arg = args[i]
    if arg in {"-h", "--help"}:
        usage = True
        print(
            "Usage: scripts/import-crypto-credentials.sh [--user-key <rk-...>] [--role admin] [--scrub-config]",
            file=sys.stderr,
        )
        raise SystemExit(0)
    if arg == "--user-key":
        i += 1
        if i >= len(args):
            raise SystemExit("--user-key requires a value")
        user_key = args[i].strip()
    elif arg == "--role":
        i += 1
        if i >= len(args):
            raise SystemExit("--role requires a value")
        role = args[i].strip().lower()
    elif arg == "--scrub-config":
        scrub_config = True
    else:
        raise SystemExit(f"unknown argument: {arg}")
    i += 1

if role not in {"admin", "user"}:
    raise SystemExit("--role must be admin or user")

if not config_path.exists():
    raise SystemExit(f"config not found: {config_path}")
if not crypto_config_path.exists():
    raise SystemExit(f"crypto config not found: {crypto_config_path}")

cfg = tomllib.loads(config_path.read_text(encoding="utf-8"))
db_rel = cfg.get("database", {}).get("sqlite_path", "data/rustclaw.db")
db_path = (root / db_rel).resolve()
db_path.parent.mkdir(parents=True, exist_ok=True)

crypto_raw = crypto_config_path.read_text(encoding="utf-8")
crypto_cfg = tomllib.loads(crypto_raw)


def toml_escape(value: str) -> str:
    return value.replace("\\", "\\\\").replace('"', '\\"')


def parse_table_names(raw: str):
    names = set()
    for line in raw.splitlines():
        s = line.strip()
        if s.startswith("[") and s.endswith("]") and not s.startswith("[["):
            names.add(s[1:-1].strip())
    return names


def write_toml_with_scrub(raw: str, imported_exchanges: list[str]) -> str:
    imported = set(imported_exchanges)
    lines = raw.splitlines()
    out = []
    current_table = ""
    seen = set()
    for line in lines:
        stripped = line.strip()
        if stripped.startswith("[") and stripped.endswith("]") and not stripped.startswith("[["):
            current_table = stripped[1:-1].strip()
            out.append(line)
            if current_table in imported and current_table not in seen:
                seen.add(current_table)
            continue
        if current_table in imported:
            if stripped.startswith("api_key"):
                continue
            if stripped.startswith("api_secret"):
                continue
            if stripped.startswith("passphrase"):
                continue
            if stripped.startswith("enabled"):
                out.append("enabled = false")
                continue
        out.append(line)

    existing_tables = parse_table_names(raw)
    for exchange in imported_exchanges:
        if exchange in existing_tables:
            continue
        out.append("")
        out.append(f"[{exchange}]")
        out.append("enabled = false")

    text = "\n".join(out)
    if not text.endswith("\n"):
        text += "\n"
    return text


def is_placeholder(value: str) -> bool:
    v = (value or "").strip()
    if not v:
        return True
    upper = v.upper()
    return (
        upper.startswith("REPLACE_ME")
        or upper in {"CHANGE_ME", "YOUR_API_KEY", "YOUR_API_SECRET", "YOUR_PASSPHRASE"}
    )


def pick_target_user_key(conn: sqlite3.Connection, explicit_key: str, role_name: str) -> str:
    if explicit_key:
        row = conn.execute(
            "SELECT role, enabled FROM auth_keys WHERE user_key = ? LIMIT 1",
            (explicit_key,),
        ).fetchone()
        if not row:
            raise SystemExit(f"user_key not found: {explicit_key}")
        found_role, enabled = row
        if int(enabled) != 1:
            raise SystemExit(f"user_key is disabled: {explicit_key}")
        if role_name and found_role != role_name:
            print(
                f"warning: user_key {explicit_key} role is {found_role}, not requested {role_name}",
                file=sys.stderr,
            )
        return explicit_key

    rows = conn.execute(
        "SELECT user_key FROM auth_keys WHERE role = ? AND enabled = 1 ORDER BY created_at ASC",
        (role_name,),
    ).fetchall()
    if not rows:
        raise SystemExit(f"no enabled {role_name} key found")
    if len(rows) > 1:
        joined = ", ".join(row[0] for row in rows)
        raise SystemExit(
            f"multiple enabled {role_name} keys found, please pass --user-key explicitly: {joined}"
        )
    return rows[0][0]


def collect_credentials(cfg_obj: dict):
    result = []
    for exchange in ("binance", "okx"):
        table = cfg_obj.get(exchange, {})
        if not isinstance(table, dict):
            continue
        api_key = str(table.get("api_key", "") or "").strip()
        api_secret = str(table.get("api_secret", "") or "").strip()
        passphrase = str(table.get("passphrase", "") or "").strip()
        enabled = bool(table.get("enabled", False))
        if is_placeholder(api_key) or is_placeholder(api_secret):
            continue
        if exchange == "okx" and is_placeholder(passphrase):
            passphrase = ""
        result.append(
            {
                "exchange": exchange,
                "enabled": enabled,
                "api_key": api_key,
                "api_secret": api_secret,
                "passphrase": passphrase,
            }
        )
    return result


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
conn.execute(
    """
    CREATE TABLE IF NOT EXISTS exchange_api_credentials (
        id          INTEGER PRIMARY KEY AUTOINCREMENT,
        user_key    TEXT NOT NULL,
        exchange    TEXT NOT NULL,
        api_key     TEXT NOT NULL,
        api_secret  TEXT NOT NULL,
        passphrase  TEXT,
        enabled     INTEGER NOT NULL DEFAULT 1,
        updated_at  TEXT NOT NULL,
        UNIQUE(user_key, exchange)
    )
    """
)

target_user_key = pick_target_user_key(conn, user_key, role)
credentials = collect_credentials(crypto_cfg)
if not credentials:
    raise SystemExit("no importable Binance/OKX credentials found in configs/crypto.toml")

now = str(int(datetime.now(timezone.utc).timestamp()))
imported = []
for item in credentials:
    conn.execute(
        """
        INSERT INTO exchange_api_credentials (
            user_key, exchange, api_key, api_secret, passphrase, enabled, updated_at
        )
        VALUES (?, ?, ?, ?, ?, 1, ?)
        ON CONFLICT(user_key, exchange)
        DO UPDATE SET
            api_key=excluded.api_key,
            api_secret=excluded.api_secret,
            passphrase=excluded.passphrase,
            enabled=1,
            updated_at=excluded.updated_at
        """,
        (
            target_user_key,
            item["exchange"],
            item["api_key"],
            item["api_secret"],
            item["passphrase"] or None,
            now,
        ),
    )
    imported.append(item["exchange"])

conn.commit()

print(f"target_user_key={target_user_key}")
for exchange in imported:
    print(f"imported {exchange} credentials into exchange_api_credentials")

if scrub_config:
    scrubbed = write_toml_with_scrub(crypto_raw, imported)
    crypto_config_path.write_text(scrubbed, encoding="utf-8")
    print(f"scrubbed secrets from {crypto_config_path}")
else:
    print("config file unchanged (use --scrub-config to clear secrets from configs/crypto.toml)")
PY
