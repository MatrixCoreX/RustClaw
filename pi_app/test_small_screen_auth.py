import json
import os
import sqlite3
import stat
import tempfile
import unittest
from unittest import mock

import small_screen_clawd_client as client
import small_screen_config as config


def _create_auth_db(path, rows=()):
    with sqlite3.connect(path) as conn:
        conn.execute(
            """
            CREATE TABLE auth_keys (
                user_key TEXT PRIMARY KEY,
                role TEXT NOT NULL,
                enabled INTEGER NOT NULL DEFAULT 1,
                created_at TEXT NOT NULL,
                last_used_at TEXT
            )
            """
        )
        conn.executemany(
            "INSERT INTO auth_keys(user_key, role, enabled, created_at) VALUES (?, ?, ?, ?)",
            rows,
        )


class SmallScreenAuthConfigTests(unittest.TestCase):
    def test_enabled_admin_is_preferred_and_never_saved_to_settings(self):
        with tempfile.TemporaryDirectory() as root:
            db_path = os.path.join(root, "runtime.db")
            settings_path = os.path.join(root, "settings.json")
            _create_auth_db(db_path, [("admin-secret", "admin", 1, "1")])
            with mock.patch.object(config, "_load_sqlite_path_from_config", return_value=db_path), mock.patch.object(
                config, "_settings_file", return_value=settings_path
            ):
                self.assertEqual(config.load_preferred_runtime_auth_key(), "admin-secret")
            self.assertFalse(os.path.exists(settings_path))

    def test_missing_admin_registers_a_user_fallback_with_private_settings(self):
        with tempfile.TemporaryDirectory() as root:
            db_path = os.path.join(root, "runtime.db")
            settings_path = os.path.join(root, "settings.json")
            _create_auth_db(db_path)
            with mock.patch.object(config, "_load_sqlite_path_from_config", return_value=db_path), mock.patch.object(
                config, "_settings_file", return_value=settings_path
            ):
                fallback_key = config.load_preferred_runtime_auth_key()

            self.assertTrue(fallback_key.startswith("rk-"))
            with sqlite3.connect(db_path) as conn:
                self.assertEqual(
                    conn.execute(
                        "SELECT role, enabled FROM auth_keys WHERE user_key = ?", (fallback_key,)
                    ).fetchone(),
                    ("user", 1),
                )
            with open(settings_path, "r", encoding="utf-8") as source:
                self.assertEqual(json.load(source)["user_key"], fallback_key)
            self.assertEqual(stat.S_IMODE(os.stat(settings_path).st_mode), 0o600)

    def test_loading_settings_repairs_an_existing_permissive_mode(self):
        with tempfile.TemporaryDirectory() as root:
            settings_path = os.path.join(root, "settings.json")
            with open(settings_path, "w", encoding="utf-8") as target:
                target.write("{}")
            os.chmod(settings_path, 0o664)
            with mock.patch.object(config, "_settings_file", return_value=settings_path):
                config.load_lang()
            self.assertEqual(stat.S_IMODE(os.stat(settings_path).st_mode), 0o600)


class _Response:
    def __init__(self, status, body=b"{}"):
        self.status = status
        self._body = body

    def read(self):
        return self._body


class _Connection:
    def __init__(self, responses):
        self.responses = list(responses)
        self.requests = []

    def request(self, method, path, body=None, headers=None):
        self.requests.append((method, path, body, dict(headers or {})))

    def getresponse(self):
        return self.responses.pop(0)

    def close(self):
        return None


class SmallScreenLocalClientAuthTests(unittest.TestCase):
    def tearDown(self):
        client._api_drop_connection_unlocked()

    def test_local_requests_prefer_the_current_admin_key(self):
        connection = _Connection([_Response(200)])
        with mock.patch.object(client, "load_enabled_admin_user_key", return_value="admin-key"), mock.patch.object(
            client.http.client, "HTTPConnection", return_value=connection
        ):
            client.localhost_api_request("GET", "/v1/health", "stale-user-key")
        self.assertEqual(connection.requests[0][3]["X-Agent-Key"], "admin-key")

    def test_unauthorized_request_reloads_rotated_admin_key_once(self):
        connection = _Connection([_Response(401, b"unauthorized"), _Response(200)])
        with mock.patch.object(
            client, "load_enabled_admin_user_key", side_effect=["old-admin", "new-admin"]
        ), mock.patch.object(client.http.client, "HTTPConnection", return_value=connection):
            client.localhost_api_request("GET", "/v1/health", "fallback-user")
        self.assertEqual(
            [request[3]["X-Agent-Key"] for request in connection.requests],
            ["old-admin", "new-admin"],
        )

    def test_local_requests_fall_back_when_no_admin_exists(self):
        connection = _Connection([_Response(200)])
        with mock.patch.object(client, "load_enabled_admin_user_key", return_value=""), mock.patch.object(
            client.http.client, "HTTPConnection", return_value=connection
        ):
            client.localhost_api_request("GET", "/v1/health", "fallback-user")
        self.assertEqual(connection.requests[0][3]["X-Agent-Key"], "fallback-user")


if __name__ == "__main__":
    unittest.main()
