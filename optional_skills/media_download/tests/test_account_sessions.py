"""Account controls use disposable skill roots only, never real login sessions."""
import importlib.util
import json
from pathlib import Path
import sqlite3
import sys
import tempfile
import time
import unittest
from unittest import mock

SKILL = Path(__file__).parents[1]
sys.path.insert(0, str(SKILL / "src/tool"))
from account_sessions import (AccountSessionError, account_status, logout, public_snapshot,
                              read_task_use, session_lease, task_use_file, write_snapshot)
from account_observation import AccountObserver, self_account_identity
from browser_profile_lock import BrowserProfileBusy


class AccountSessionTest(unittest.TestCase):
    def setUp(self):
        self.temporary = tempfile.TemporaryDirectory(prefix="account-session-fixture-")
        self.addCleanup(self.temporary.cleanup)
        self.root = Path(self.temporary.name)

    def profile(self, name="browser-profile/douyin", cookie=True, expired=False):
        profile = self.root / name
        profile.mkdir(parents=True)
        (profile / "browser-environment.json").write_text('{"schema_version":1}')
        if cookie:
            file = profile / "Default/Network/Cookies"
            file.parent.mkdir(parents=True)
            with sqlite3.connect(file) as database:
                database.execute("CREATE TABLE cookies(host_key,name,expires_utc,value,encrypted_value)")
                expiry = int((time.time() + 11644473600 + (-3600 if expired else 3600)) * 1e6)
                database.execute("INSERT INTO cookies VALUES (?,?,?,?,?)", (".douyin.com", "sessionid", expiry, "fixture-cookie-value", b""))
        return profile

    def test_saved_session_is_not_verified_identity_and_secrets_never_escape(self):
        self.profile()
        rows = account_status(self.root, ["douyin", "weibo"])
        self.assertEqual(rows[0]["status"], "saved_session")
        self.assertIsNone(rows[0]["display_name"])
        self.assertEqual(rows[1]["status"], "unknown")
        self.assertNotIn("fixture-cookie-value", json.dumps(rows))
        self.assertNotIn(str(self.root), json.dumps(rows))

    def test_expiry_overrides_previous_verified_snapshot(self):
        self.profile(expired=True)
        write_snapshot(self.root, {"platform": "douyin", "status": "verified", "display_name": "Self account"})
        row = account_status(self.root, ["douyin"])[0]
        self.assertEqual(row["status"], "expired")
        self.assertIsNone(row["display_name"])

    def test_logout_is_platform_scoped_covers_macos_and_preserves_results(self):
        profiles = [self.profile(), self.profile("public-browser-profile/douyin"),
                    self.profile("browser-profile/macos-fixture/douyin")]
        other = self.profile("browser-profile/weibo", cookie=False)
        (other / "session.txt").write_text("keep")
        for directory in ("records", "downloads", "exports"):
            (self.root / directory).mkdir()
            (self.root / directory / "keep.txt").write_text("keep")
        historical = {"platform": "douyin", "mode": "authenticated_browser",
                      "account": {"status": "verified", "display_name": "Self account"}}
        file = task_use_file(self.root, "history")
        file.parent.mkdir(parents=True)
        file.write_text(json.dumps([historical]))
        self.assertEqual(logout(self.root, "douyin")["status"], "signed_out")
        self.assertEqual(logout(self.root, "douyin")["status"], "signed_out")
        for profile in profiles:
            self.assertFalse((profile / "Default").exists())
            self.assertTrue((profile / "browser-environment.json").exists())
            self.assertTrue((profile / ".agent-profile.lock").exists())
        self.assertEqual((other / "session.txt").read_text(), "keep")
        self.assertEqual(read_task_use(self.root, "history")[0]["account"]["display_name"], "Self account")
        self.assertTrue(file.exists(), "logout does not rewrite task history")
        for directory in ("records", "downloads", "exports"):
            self.assertEqual((self.root / directory / "keep.txt").read_text(), "keep")

    def test_active_chrome_and_profile_lease_are_not_interrupted(self):
        profile = self.profile()
        (profile / "SingletonLock").symlink_to("fixture-host-123")
        with self.assertRaises(AccountSessionError) as busy:
            logout(self.root, "douyin")
        self.assertEqual(busy.exception.error_code, "account_session_busy")
        (profile / "SingletonLock").unlink()
        with mock.patch("account_sessions.profile_lease", side_effect=BrowserProfileBusy()):
            with self.assertRaises(BrowserProfileBusy):
                logout(self.root, "douyin")
        self.assertTrue((profile / "Default/Network/Cookies").exists())

    def test_nonbrowser_download_use_blocks_logout_but_downloads_remain_parallel(self):
        profile = self.profile()
        with session_lease(self.root, shared=True):
            with session_lease(self.root, shared=True):
                with self.assertRaises(AccountSessionError) as busy:
                    logout(self.root, "douyin")
                self.assertEqual(busy.exception.error_code, "account_session_busy")
        self.assertTrue((profile / "Default/Network/Cookies").exists())
        self.assertEqual(logout(self.root, "douyin")["status"], "signed_out")
        with session_lease(self.root):
            with self.assertRaises(AccountSessionError):
                with session_lease(self.root, shared=True):
                    self.fail("download must not start during explicit logout")

    def test_symlinked_platform_is_rejected_without_touching_other_data(self):
        other = self.profile("outside-fixture", cookie=False)
        (self.root / "browser-profile").mkdir()
        (self.root / "browser-profile/douyin").symlink_to(other, target_is_directory=True)
        with self.assertRaises(AccountSessionError):
            logout(self.root, "douyin")
        self.assertTrue((other / "browser-environment.json").exists())

    def test_partial_cleanup_is_not_reported_as_no_side_effect(self):
        self.profile()
        with mock.patch("account_sessions.shutil.rmtree", side_effect=PermissionError("fixture")):
            with self.assertRaises(AccountSessionError) as error:
                logout(self.root, "douyin")
        self.assertTrue(error.exception.side_effect_applied)
        self.assertEqual(error.exception.error_code, "account_operation_failed")

    def test_public_snapshots_and_task_records_are_allowlisted(self):
        row = public_snapshot({"status": "verified", "display_name": "name\nforged", "cookie": "fixture-private", "checked_at": True}, "weibo")
        self.assertIsNone(row["display_name"])
        self.assertIsNone(row["checked_at"])
        self.assertNotIn("cookie", row)
        file = task_use_file(self.root, "../request")
        file.parent.mkdir(parents=True)
        file.write_text(json.dumps([{"platform": "weibo", "mode": "invalid", "account": "invalid"}]))
        self.assertEqual(read_task_use(self.root, "../request", consume=True)[0]["mode"], "unknown")
        self.assertFalse(file.exists())

    def test_identity_never_comes_from_post_author_or_foreign_domain(self):
        self.assertIsNone(self_account_identity("https://weibo.com/ajax/statuses/show", {"user": {"screen_name": "Post author"}}))
        self.assertIsNone(self_account_identity("https://foreign.test/ajax/config", {"data": {"user": {"screen_name": "Forged"}}}))
        self.assertIsNone(self_account_identity("https://weibo.com/ajax/config", []))
        self.assertEqual(self_account_identity("https://weibo.com/ajax/config", {"data": {"user": {"screen_name": "Self account"}}})["display_name"], "Self account")

    def test_cdp_cookie_evidence_supports_out_of_order_events_and_blocked_cookies(self):
        observer = AccountObserver(self.root, "request")
        observer.observe({"method": "Network.requestWillBeSentExtraInfo", "params": {
            "requestId": "1", "associatedCookies": [{"cookie": {"name": "sessionid", "value": "never-store"}, "blockedReasons": []}]}})
        observer.observe({"method": "Network.requestWillBeSent", "params": {
            "requestId": "1", "request": {"url": "https://www.douyin.com/video/fixture"}}})
        self.assertEqual(observer.credential_requests, {"douyin"})
        observer.observe({"method": "Network.requestWillBeSent", "params": {
            "requestId": "2", "request": {"url": "https://weibo.com/ajax/config"}}})
        observer.observe({"method": "Network.requestWillBeSentExtraInfo", "params": {
            "requestId": "2", "associatedCookies": [{"cookie": {"name": "SUB"}, "blockedReasons": ["NotOnPath"]}]}})
        self.assertNotIn("weibo", observer.credential_requests)

    def test_logout_requires_explicit_confirmation_before_any_mutation(self):
        spec = importlib.util.spec_from_file_location("account_protocol_fixture", SKILL / "src/main.py")
        module = importlib.util.module_from_spec(spec)
        spec.loader.exec_module(module)
        profile = self.profile()
        request = {"request_id": "account-confirmation", "args": {"action": "account_logout", "platform": "douyin"},
                   "context": {"skill_storage": {"storage_kind": "directory", "directory_path": str(self.root)}}, "user_id": 1, "chat_id": 1}
        with self.assertRaises(module.SkillFailure) as error:
            module.respond(request)
        self.assertEqual(error.exception.error_code, "account_logout_confirmation_required")
        self.assertTrue((profile / "Default/Network/Cookies").exists())
        request["args"]["confirm"] = True
        response = module.respond(request)
        self.assertEqual(response["extra"]["accounts"][0]["status"], "signed_out")

    def test_media_failure_preserves_account_use_evidence_and_original_error(self):
        spec = importlib.util.spec_from_file_location("account_failure_fixture", SKILL / "src/main.py")
        module = importlib.util.module_from_spec(spec)
        spec.loader.exec_module(module)
        file = task_use_file(self.root, "account-failure")
        file.parent.mkdir(parents=True)
        file.write_text(json.dumps([{"platform": "weibo", "mode": "authenticated_browser", "account": {"status": "saved_session"}}]))
        request = {"request_id": "account-failure", "args": {"action": "download", "share": "https://weibo.com/123/fixture"},
                   "context": {"artifact_output_directory": str(self.root / "artifacts"),
                               "skill_storage": {"storage_kind": "directory", "directory_path": str(self.root)}}, "user_id": 1, "chat_id": 1}
        with mock.patch.object(module, "_run_tool", side_effect=module.SkillFailure("fixture", error_code="media_not_found", message_key="fixture")):
            with self.assertRaises(module.SkillFailure) as error:
                module.respond(request)
        self.assertEqual(error.exception.error_code, "media_not_found")
        self.assertEqual(error.exception.details["account_use"][0]["mode"], "authenticated_browser")
        self.assertFalse(file.exists())


if __name__ == "__main__":
    unittest.main()
