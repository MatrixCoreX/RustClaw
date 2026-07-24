#!/usr/bin/env python3
"""Exercise the crypto credential import against isolated SQLite files."""
from __future__ import annotations

import os
import sqlite3
import subprocess
import tempfile
from pathlib import Path


ROOT = Path(__file__).resolve().parents[2]
IMPORT_SCRIPT = ROOT / "scripts" / "import-crypto-credentials.sh"


def toml_path(path: Path) -> str:
    return str(path).replace("\\", "\\\\").replace('"', '\\"')


def main() -> int:
    with tempfile.TemporaryDirectory(prefix="crypto-import-contract-") as tmp:
        workspace = Path(tmp)
        main_db = workspace / "runtime.db"
        skill_root = workspace / "skill-data"
        config = workspace / "config.toml"
        crypto = workspace / "crypto.toml"

        db = sqlite3.connect(main_db)
        db.execute(
            """
            CREATE TABLE auth_keys (
                user_key TEXT PRIMARY KEY,
                role TEXT NOT NULL,
                enabled INTEGER NOT NULL,
                created_at TEXT NOT NULL,
                last_used_at TEXT
            )
            """
        )
        db.execute(
            "INSERT INTO auth_keys VALUES ('rk-import-test', 'admin', 1, '1', NULL)"
        )
        db.commit()
        db.close()

        config.write_text(
            "[database]\n"
            f'sqlite_path = "{toml_path(main_db)}"\n'
            f'skill_data_root = "{toml_path(skill_root)}"\n'
            "busy_timeout_ms = 5000\n",
            encoding="utf-8",
        )
        crypto.write_text(
            "[okx]\n"
            "enabled = true\n"
            'api_key = "fixture-key"\n'
            'api_secret = "fixture-secret"\n'
            'passphrase = "fixture-passphrase"\n',
            encoding="utf-8",
        )
        env = os.environ.copy()
        env["RUSTCLAW_CONFIG_PATH"] = str(config)
        env["RUSTCLAW_CRYPTO_CONFIG_PATH"] = str(crypto)
        command = [str(IMPORT_SCRIPT), "--user-key", "rk-import-test"]
        for _ in range(2):
            subprocess.run(
                command,
                cwd=ROOT,
                env=env,
                check=True,
                text=True,
                capture_output=True,
            )

        main = sqlite3.connect(main_db)
        main_table_count = main.execute(
            """
            SELECT COUNT(*) FROM sqlite_master
            WHERE type='table' AND name='exchange_api_credentials'
            """
        ).fetchone()[0]
        main.close()
        assert main_table_count == 0

        crypto_db_path = skill_root / "crypto" / "state.db"
        owned = sqlite3.connect(crypto_db_path)
        row = owned.execute(
            """
            SELECT user_key, exchange, api_key, api_secret, passphrase, enabled
            FROM exchange_api_credentials
            """
        ).fetchone()
        row_count = owned.execute(
            "SELECT COUNT(*) FROM exchange_api_credentials"
        ).fetchone()[0]
        metadata = owned.execute(
            """
            SELECT skill_name, schema_version
            FROM skill_storage_metadata
            """
        ).fetchone()
        owned.close()

        assert row == (
            "rk-import-test",
            "okx",
            "fixture-key",
            "fixture-secret",
            "fixture-passphrase",
            1,
        )
        assert row_count == 1
        assert metadata == ("crypto", 1)
        if os.name == "posix":
            assert skill_root.joinpath("crypto").stat().st_mode & 0o777 == 0o700
            assert crypto_db_path.stat().st_mode & 0o777 == 0o600

    print("CRYPTO_CREDENTIAL_IMPORT_CONTRACT ok")
    return 0


if __name__ == "__main__":
    raise SystemExit(main())
