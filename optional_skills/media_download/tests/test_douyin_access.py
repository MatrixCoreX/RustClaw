import importlib.util
import os
from pathlib import Path
import socket
import sqlite3
import sys
import tempfile
import time
import unittest


TOOL_DIR = Path(__file__).parents[1] / "src" / "tool"
ENTRYPOINT = TOOL_DIR / "douyin_access.py"


def load_module():
    sys.path.insert(0, str(TOOL_DIR))
    try:
        spec = importlib.util.spec_from_file_location("media_download_douyin_access", ENTRYPOINT)
        assert spec and spec.loader
        module = importlib.util.module_from_spec(spec)
        sys.modules[spec.name] = module
        spec.loader.exec_module(module)
        return module
    finally:
        sys.path.remove(str(TOOL_DIR))


class DouyinAccessTest(unittest.TestCase):
    @classmethod
    def setUpClass(cls) -> None:
        cls.access = load_module()

    def test_private_profile_is_scoped_to_douyin(self) -> None:
        with tempfile.TemporaryDirectory() as directory:
            profile = self.access.persistent_profile_dir(directory)

            self.assertEqual(profile, Path(directory) / "douyin")
            self.assertTrue(profile.is_dir())

    def test_authenticated_session_requires_a_douyin_login_cookie(self) -> None:
        with tempfile.TemporaryDirectory() as directory:
            profile = Path(directory) / "douyin"
            database = profile / "Default" / "Network" / "Cookies"
            database.parent.mkdir(parents=True)
            connection = sqlite3.connect(database)
            try:
                connection.execute(
                    "create table cookies(name text, host_key text, expires_utc integer, "
                    "value text, encrypted_value blob)"
                )
                connection.execute(
                    "insert into cookies values (?, ?, ?, ?, ?)",
                    ("ttwid", ".douyin.com", 0, "fixture", b""),
                )
                connection.commit()
            finally:
                connection.close()

            self.assertFalse(self.access.profile_has_authenticated_session(profile))

            connection = sqlite3.connect(database)
            try:
                connection.execute(
                    "insert into cookies values (?, ?, ?, ?, ?)",
                    ("sessionid", ".douyin.com", 0, "fixture", b""),
                )
                connection.commit()
            finally:
                connection.close()

            self.assertTrue(self.access.profile_has_authenticated_session(profile))

    def test_expired_or_empty_login_cookie_is_not_authenticated(self) -> None:
        with tempfile.TemporaryDirectory() as directory:
            profile = Path(directory) / "douyin"
            database = profile / "Default" / "Network" / "Cookies"
            database.parent.mkdir(parents=True)
            chrome_epoch = 11_644_473_600
            expired = int((time.time() + chrome_epoch - 60) * 1_000_000)
            future = int((time.time() + chrome_epoch + 3600) * 1_000_000)
            connection = sqlite3.connect(database)
            try:
                connection.execute(
                    "create table cookies(name text, host_key text, expires_utc integer, "
                    "value text, encrypted_value blob)"
                )
                connection.executemany(
                    "insert into cookies values (?, ?, ?, ?, ?)",
                    [
                        ("sessionid", ".douyin.com", expired, "expired", b""),
                        ("sessionid_ss", ".douyin.com", future, "", b""),
                    ],
                )
                connection.commit()
            finally:
                connection.close()

            self.assertFalse(self.access.profile_has_authenticated_session(profile))

            connection = sqlite3.connect(database)
            try:
                connection.execute(
                    "update cookies set encrypted_value = ? where name = ?",
                    (b"encrypted", "sessionid_ss"),
                )
                connection.commit()
            finally:
                connection.close()

            self.assertTrue(self.access.profile_has_authenticated_session(profile))

    def test_stale_chrome_singleton_links_are_removed(self) -> None:
        with tempfile.TemporaryDirectory() as directory:
            profile = Path(directory) / "douyin"
            profile.mkdir()
            (profile / "SingletonCookie").symlink_to("cookie-token")
            (profile / "SingletonLock").symlink_to("host-18")
            (profile / "SingletonSocket").symlink_to(Path(directory) / "missing.sock")

            self.assertTrue(self.access.remove_stale_chrome_singleton_artifacts(profile))
            self.assertFalse((profile / "SingletonCookie").is_symlink())
            self.assertFalse((profile / "SingletonLock").is_symlink())
            self.assertFalse((profile / "SingletonSocket").is_symlink())

    @unittest.skipUnless(hasattr(socket, "AF_UNIX"), "Unix sockets are required")
    def test_live_chrome_singleton_socket_preserves_links(self) -> None:
        with tempfile.TemporaryDirectory() as directory:
            profile = Path(directory) / "douyin"
            profile.mkdir()
            socket_path = Path(directory) / "chrome.sock"
            server = socket.socket(socket.AF_UNIX, socket.SOCK_STREAM)
            try:
                server.bind(str(socket_path))
                server.listen(1)
                (profile / "SingletonCookie").symlink_to("cookie-token")
                (profile / "SingletonLock").symlink_to(f"host-{os.getpid()}")
                (profile / "SingletonSocket").symlink_to(socket_path)

                self.assertFalse(self.access.remove_stale_chrome_singleton_artifacts(profile))
                self.assertTrue((profile / "SingletonCookie").is_symlink())
                self.assertTrue((profile / "SingletonLock").is_symlink())
                self.assertTrue((profile / "SingletonSocket").is_symlink())
            finally:
                server.close()


if __name__ == "__main__":
    unittest.main()
